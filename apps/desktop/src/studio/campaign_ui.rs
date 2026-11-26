//! Test-campaign section of the Experiments panel: many seeded runs of the
//! current scene judged against acceptance criteria, with a results table,
//! one-click reproduction of any run and report export.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use crate::project::Scene;
use egui::{Color32, RichText, Ui};
use pelagic::campaign::{self, Criteria, Report, Variation};
use std::sync::Arc;

/// One acceptance check in the editor: label, unit, default limit, drag
/// speed and allowed range.
struct Check {
    label: &'static str,
    unit: &'static str,
    default: f64,
    speed: f64,
    range: (f64, f64),
}
const fn check(
    label: &'static str,
    unit: &'static str,
    default: f64,
    speed: f64,
    range: (f64, f64),
) -> Check {
    Check {
        label,
        unit,
        default,
        speed,
        range,
    }
}
const CHECKS: [Check; 6] = [
    check("Max nav error", " m", 0.5, 0.01, (0.0, 100.0)),
    check("Max depth excursion", " m", 1.0, 0.05, (0.0, 1000.0)),
    check("Min seabed clearance", " m", 1.0, 0.05, (-10.0, 1000.0)),
    check("Min vehicle separation", " m", 1.0, 0.05, (-10.0, 1000.0)),
    check("Max tilt", "°", 15.0, 0.5, (0.0, 180.0)),
    check("Min sonar coverage", "", 0.8, 0.01, (0.0, 1.0)),
];

pub(super) struct Panel {
    variation: Variation,
    seconds: f64,
    enabled: [bool; 6],
    limits: [f64; 6],
    /// The finished report and the scene it was run on (for reproduction).
    result: Option<(Arc<Report>, Arc<Scene>)>,
    failures_only: bool,
    collapse_setup: bool,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            variation: Variation::default(),
            seconds: 30.0,
            enabled: [true, false, true, false, true, false],
            limits: CHECKS.map(|c| c.default),
            result: None,
            failures_only: false,
            collapse_setup: false,
        }
    }
}
impl Panel {
    /// Script hook: a short campaign for captures and automated checks.
    pub(super) fn set_quick(&mut self, runs: usize) {
        self.variation.runs = runs.clamp(1, campaign::MAX_RUNS);
        self.seconds = 5.0;
        self.variation.wave_height_jitter = 0.3;
    }
    fn criteria(&self) -> Criteria {
        let on = |i: usize| self.enabled[i].then_some(self.limits[i]);
        Criteria {
            max_nav_error_m: on(0),
            max_depth_excursion_m: on(1),
            min_seabed_clearance_m: on(2),
            min_vehicle_separation_m: on(3),
            max_tilt_deg: on(4),
            min_sonar_coverage: on(5),
        }
    }
}

impl State {
    pub(super) fn campaign_finished(&mut self, report: Report, scene: Scene) {
        let text = crate::cli::summary(&report);
        let passed = report.passed();
        let total = report.runs.len();
        self.campaign.result = Some((Arc::new(report), Arc::new(scene)));
        self.campaign.collapse_setup = true;
        self.log(format!("Campaign finished: {passed}/{total} runs passed."));
        for line in text.lines().skip(1) {
            self.model.notice = line.to_string();
            self.model.record_notice();
        }
    }

    pub(super) fn start_campaign(&mut self) {
        if let Err(e) = self.campaign.variation.validate() {
            return self.log(e);
        }
        let scene = self.scene().clone();
        let variation = self.campaign.variation.clone();
        let criteria = self.campaign.criteria();
        let seconds = self.campaign.seconds;
        let threads = campaign::default_threads();
        self.campaign.result = None;
        self.start_job(
            &format!("Campaign 0/{} runs", variation.runs),
            move |cancel, send| {
                let report = campaign::run(
                    &scene,
                    seconds,
                    &variation,
                    &criteria,
                    threads,
                    &cancel,
                    |done, total| {
                        let _ = send.send(Event::Progress(format!("Campaign {done}/{total} runs")));
                    },
                )?;
                Ok(Output::Campaign(Box::new((report, scene))))
            },
        );
    }

    /// Re-runs one campaign member as a recording, ready for Replay.
    fn reproduce(&mut self, index: usize) {
        let Some((report, scene)) = self.campaign.result.clone() else {
            return;
        };
        let scene = report.variation.scene(&scene, index);
        let seconds = report.seconds;
        self.start_job(
            &format!("Reproducing run {index}…"),
            move |cancel, send| {
                let run = pelagic::experiment::batch(&scene, seconds, &cancel, |tick, total| {
                    let _ = send.send(Event::Progress(format!(
                        "Run {index}: {tick}/{total} ticks"
                    )));
                })?;
                Ok(Output::Run(Box::new(run)))
            },
        );
    }

    fn export_campaign(&mut self) {
        let Some((report, _)) = self.campaign.result.clone() else {
            return;
        };
        let Some(parent) =
            native::pick_folder(self.hwnd, "Choose a folder for the campaign report")
        else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-campaign-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        let result = report.write(&folder).and_then(|()| {
            std::fs::write(folder.join("summary.txt"), crate::cli::summary(&report))
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(()) => self.log(format!("Campaign report saved: {}", folder.display())),
            Err(e) => self.log(format!("Could not save the campaign report: {e}")),
        }
    }

    pub(super) fn campaign_panel(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(8.0);
        theme::heading(ui, "Test campaign");
        ui.label(
            RichText::new(
                "Runs the scene many times with different seeds (waves, marine life, sensor \
                 noise) and judges every run against the checks below.",
            )
            .small()
            .color(theme::MUTED),
        );
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, theme::primary("▶ Run campaign"))
                .on_hover_text("Uses all CPU cores but one; the editor stays responsive")
                .clicked()
            {
                self.start_campaign();
            }
            if self.campaign.result.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new("💾 Export report…"))
                    .on_hover_text("campaign.json, campaign.csv and summary.txt")
                    .clicked()
            {
                self.export_campaign();
            }
        });
        if let Some((report, _)) = self.campaign.result.clone() {
            self.campaign_results(ui, &report, busy);
        }
        self.campaign_setup(ui);
    }

    fn campaign_results(&mut self, ui: &mut Ui, report: &Report, busy: bool) {
        let total = report.runs.len();
        let passed = report.passed();
        let colour = if passed == total {
            Color32::from_rgb(120, 210, 110)
        } else {
            Color32::from_rgb(235, 110, 90)
        };
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "{passed}/{total} passed · {} failed · {} errors",
                total - passed - report.errors(),
                report.errors()
            ))
            .strong()
            .color(colour),
        );
        let w = report.worst();
        ui.label(
            RichText::new(format!(
                "Worst: nav {:.3} m · depth {:.3} m · clearance {:.2} m · separation {:.2} m · tilt {:.1}° · sonar {:.2}",
                w.nav_error_max_m,
                w.depth_excursion_max_m,
                w.seabed_clearance_min_m,
                w.vehicle_separation_min_m,
                w.tilt_max_deg,
                w.sonar_coverage_mean
            ))
            .small(),
        );
        ui.checkbox(&mut self.campaign.failures_only, "Show failed runs only");
        let mut reproduce = None;
        egui::ScrollArea::vertical()
            .id_salt("campaign-results")
            .max_height(260.0)
            .show(ui, |ui| {
                egui::Grid::new("campaign-results-grid")
                    .striped(true)
                    .num_columns(9)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for h in [
                            "Run",
                            "Seed",
                            "Result",
                            "Nav err",
                            "Depth",
                            "Clearance",
                            "Tilt",
                            "Sonar",
                            "",
                        ] {
                            ui.label(RichText::new(h).small().color(theme::MUTED));
                        }
                        ui.end_row();
                        for r in report
                            .runs
                            .iter()
                            .filter(|r| !self.campaign.failures_only || !r.passed)
                        {
                            let m = &r.metrics;
                            ui.label(r.index.to_string());
                            ui.label(r.seed.to_string());
                            let (text, colour) = match (&r.error, r.passed) {
                                (Some(_), _) => ("error", Color32::from_rgb(235, 170, 70)),
                                (None, true) => ("pass", Color32::from_rgb(120, 210, 110)),
                                (None, false) => ("fail", Color32::from_rgb(235, 110, 90)),
                            };
                            let why = r.error.clone().unwrap_or_else(|| r.failures.join("; "));
                            let result = ui.label(RichText::new(text).color(colour));
                            if !why.is_empty() {
                                result.on_hover_text(why);
                            }
                            ui.label(format!("{:.3}", m.nav_error_max_m));
                            ui.label(format!("{:.3}", m.depth_excursion_max_m));
                            ui.label(format!("{:.2}", m.seabed_clearance_min_m));
                            ui.label(format!("{:.1}°", m.tilt_max_deg));
                            ui.label(format!("{:.2}", m.sonar_coverage_mean));
                            let button = ui
                                .add_enabled(!busy, egui::Button::new("Reproduce").small())
                                .on_hover_text("Re-run exactly this run as a recording for Replay");
                            crate::studio::a11y::name(
                                &button,
                                &format!("Reproduce run {}", r.index),
                            );
                            if button.clicked() {
                                reproduce = Some(r.index);
                            }
                            ui.end_row();
                        }
                    });
            });
        if let Some(index) = reproduce {
            self.reproduce(index);
        }
    }

    fn campaign_setup(&mut self, ui: &mut Ui) {
        // Folded away once results arrive; reopened on demand.
        let close = std::mem::take(&mut self.campaign.collapse_setup);
        egui::CollapsingHeader::new(RichText::new("Setup and acceptance criteria").strong())
            .id_salt("campaign-setup-header")
            .default_open(true)
            .open(close.then_some(false))
            .show(ui, |ui| {
                let p = &mut self.campaign;
                egui::Grid::new("campaign-setup")
                    .num_columns(4)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        let mut runs = p.variation.runs as f64;
                        ui.label("Runs");
                        let r = ui.add(
                            egui::DragValue::new(&mut runs)
                                .range(1.0..=500.0)
                                .speed(1.0),
                        );
                        crate::studio::a11y::name(&r, "Campaign runs");
                        p.variation.runs = runs as usize;
                        ui.label("Seconds per run");
                        let r = ui.add(
                            egui::DragValue::new(&mut p.seconds)
                                .range(0.1..=120.0)
                                .speed(0.5)
                                .suffix(" s"),
                        );
                        crate::studio::a11y::name(&r, "Seconds per run");
                        ui.end_row();
                        ui.label("First seed");
                        let r = ui.add(egui::DragValue::new(&mut p.variation.seed_base).speed(1.0));
                        crate::studio::a11y::name(&r, "First seed");
                        ui.label("");
                        ui.label("");
                        ui.end_row();
                        let mut wave = p.variation.wave_height_jitter * 100.0;
                        ui.label("Wave height ±");
                        let r = ui.add(
                            egui::DragValue::new(&mut wave)
                                .range(0.0..=90.0)
                                .speed(0.5)
                                .suffix(" %"),
                        );
                        crate::studio::a11y::name(&r, "Wave height variation");
                        p.variation.wave_height_jitter = wave / 100.0;
                        let mut current = p.variation.current_jitter * 100.0;
                        ui.label("Current ±");
                        let r = ui.add(
                            egui::DragValue::new(&mut current)
                                .range(0.0..=90.0)
                                .speed(0.5)
                                .suffix(" %"),
                        );
                        crate::studio::a11y::name(&r, "Current variation");
                        p.variation.current_jitter = current / 100.0;
                        ui.end_row();
                    });
                ui.add_space(2.0);
                egui::Grid::new("campaign-criteria")
                    .num_columns(2)
                    .spacing([10.0, 2.0])
                    .show(ui, |ui| {
                        for (i, c) in CHECKS.iter().enumerate() {
                            ui.checkbox(&mut p.enabled[i], c.label);
                            let r = ui.add_enabled(
                                p.enabled[i],
                                egui::DragValue::new(&mut p.limits[i])
                                    .range(c.range.0..=c.range.1)
                                    .speed(c.speed)
                                    .suffix(c.unit),
                            );
                            crate::studio::a11y::name(&r, &format!("{} limit", c.label));
                            ui.end_row();
                        }
                    });
            });
    }
}
