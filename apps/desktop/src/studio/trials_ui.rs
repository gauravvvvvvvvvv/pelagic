//! Standard trials in the editor: run free decay, speed, turning-circle,
//! zig-zag, stopping and wave-response trials on the selected vehicle, see
//! the results judged against acceptance limits, inspect them as charts and
//! export the report.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use crate::studio::charts::{Options, SERIES, Series, chart};
use crate::viewport_edit::Actor;
use egui::{RichText, Ui};
use pelagic::entities::Entity;
use pelagic::trials::{self, Criteria, Environment, Report, Setup};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Chart {
    Decay,
    Speed,
    Turning,
    Zigzag,
    Waves,
}

pub(super) struct Panel {
    setup: Setup,
    /// Use the scene's density, depth, current and sea instead of the basin.
    scene_conditions: bool,
    water_depth: f64,
    imo: bool,
    min_speed: Option<f64>,
    max_roll_rao: Option<f64>,
    result: Option<Arc<Report>>,
    chart: Chart,
    decay_axis: usize,
    wave_heading: usize,
    collapse_setup: bool,
    /// Visible height of the chart column, so a chart fits without scrolling.
    chart_room: f32,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            setup: Setup::default(),
            scene_conditions: false,
            water_depth: 200.0,
            imo: true,
            min_speed: None,
            max_roll_rao: None,
            result: None,
            chart: Chart::Turning,
            decay_axis: 0,
            wave_heading: 0,
            collapse_setup: false,
            chart_room: 400.0,
        }
    }
}
impl Panel {
    /// Script hook: the short set, for captures and automated checks.
    pub(super) fn set_quick(&mut self) {
        self.setup = Setup::quick();
    }
    /// Script hook: show one chart (decay, speed, turning, zigzag, waves).
    pub(super) fn show_chart(&mut self, name: &str) {
        self.chart = match name {
            "decay" => Chart::Decay,
            "speed" => Chart::Speed,
            "zigzag" => Chart::Zigzag,
            "waves" => Chart::Waves,
            _ => Chart::Turning,
        };
    }
}

impl State {
    /// The vehicle the trials will test: the selected body, else nothing.
    pub(super) fn trial_subject(&self) -> Option<Entity> {
        let scene = self.scene();
        match self.selected_actor()? {
            Actor::Reference => scene.vehicle.then(|| scene.reference_entity()),
            Actor::Entity(id) => scene.entities.iter().find(|e| e.id == id).cloned(),
            Actor::Structure => None,
        }
    }

    pub(super) fn trials_finished(&mut self, report: Report) {
        let failed = report.failed();
        let total = report.checks.len();
        self.log(if total == 0 {
            format!("Trials finished for {}.", report.vehicle.name)
        } else if failed == 0 {
            format!(
                "Trials finished for {}: {}.",
                report.vehicle.name,
                if total == 1 {
                    "the check passed".to_string()
                } else {
                    format!("all {total} checks passed")
                }
            )
        } else {
            format!(
                "Trials finished for {}: {failed} of {total} checks failed.",
                report.vehicle.name
            )
        });
        for w in &report.warnings {
            self.log(format!("Trials warning: {w}"));
        }
        self.trials.chart = if !report.turning.is_empty() {
            Chart::Turning
        } else if !report.decay.is_empty() {
            Chart::Decay
        } else {
            Chart::Waves
        };
        self.trials.result = Some(Arc::new(report));
        self.trials.collapse_setup = true;
    }

    pub(super) fn start_trials(&mut self) {
        let Some(vehicle) = self.trial_subject() else {
            return self.log("Select a vehicle in the Outliner to run trials on it.");
        };
        let p = &self.trials;
        let mut environment = if p.scene_conditions {
            Environment::from_scene(self.scene())
        } else {
            Environment {
                density: self.scene().density,
                water_depth: p.water_depth,
                ..Environment::default()
            }
        };
        if !p.scene_conditions {
            environment.seed = self.scene().seed;
        }
        let setup = p.setup.clone();
        if let Err(e) = setup.validate().and_then(|()| environment.validate()) {
            return self.log(e);
        }
        let criteria = Criteria {
            min_speed_mps: p.min_speed,
            max_roll_rao: p.max_roll_rao,
            ..Criteria::default()
        };
        let imo = p.imo;
        let threads = pelagic::campaign::default_threads();
        self.trials.result = None;
        self.start_job(
            &format!("Trials for {}…", vehicle.name),
            move |cancel, send| {
                let mut report = trials::run(
                    &vehicle,
                    &environment,
                    &setup,
                    &criteria,
                    threads,
                    &cancel,
                    |done, total| {
                        let _ = send.send(Event::Progress(format!("Trial {done}/{total}")));
                    },
                )?;
                if imo {
                    let mut limits = report.imo_criteria();
                    limits.min_speed_mps = criteria.min_speed_mps;
                    limits.max_roll_rao = criteria.max_roll_rao;
                    report.judge_with(limits);
                }
                Ok(Output::Trials(Box::new(report)))
            },
        );
    }

    fn export_trials(&mut self) {
        let Some(report) = self.trials.result.clone() else {
            return;
        };
        let Some(parent) = native::pick_folder(self.hwnd, "Choose a folder for the trial report")
        else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-trials-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        match report.write(&folder) {
            Ok(()) => self.log(format!("Trial report saved: {}", folder.display())),
            Err(e) => self.log(format!("Could not save the trial report: {e}")),
        }
    }

    pub(super) fn trials_panel(&mut self, ui: &mut Ui) {
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("trials-left")
                .show(left, |ui| self.trials_controls(ui));
            self.trials.chart_room = right.available_height();
            egui::ScrollArea::vertical()
                .id_salt("trials-right")
                .show(right, |ui| self.trials_charts(ui));
        });
    }

    fn trials_controls(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(4.0);
        theme::heading(ui, "Standard trials");
        ui.label(
            RichText::new(
                "Free decay, speed, turning circle, zig-zag, crash stop and wave response \
                 for one vehicle in a test basin. Results are deterministic.",
            )
            .small()
            .color(theme::MUTED),
        );
        ui.add_space(4.0);
        let subject = self.trial_subject();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Vehicle").color(theme::MUTED));
            match &subject {
                Some(e) => {
                    ui.label(RichText::new(&e.name).strong().color(theme::STRONG));
                }
                None => {
                    ui.label(RichText::new("select one in the Outliner").color(theme::WARN));
                }
            }
        });
        if let Some(job) = &self.job {
            let cancel = job.cancel.clone();
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&self.job_status);
                if ui.button("Cancel").clicked() {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
        }
        ui.horizontal(|ui| {
            let run = ui
                .add_enabled(!busy && subject.is_some(), theme::primary("▶ Run trials"))
                .on_hover_text("Runs every selected trial on all CPU cores but one");
            crate::studio::a11y::name(&run, "Run trials");
            if run.clicked() {
                self.start_trials();
            }
            if self.trials.result.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new("💾 Export report…"))
                    .on_hover_text("trials.json, summary.txt and CSV tables")
                    .clicked()
            {
                self.export_trials();
            }
        });
        if let Some(report) = self.trials.result.clone() {
            trial_results(ui, &report);
        }
        self.trials_setup(ui);
    }

    fn trials_setup(&mut self, ui: &mut Ui) {
        let close = std::mem::take(&mut self.trials.collapse_setup);
        egui::CollapsingHeader::new(RichText::new("Setup and acceptance limits").strong())
            .id_salt("trials-setup-header")
            .default_open(true)
            .open(close.then_some(false))
            .show(ui, |ui| {
                let p = &mut self.trials;
                let s = &mut p.setup;
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut s.free_decay, "Free decay");
                    ui.checkbox(&mut s.speed, "Speed");
                    ui.checkbox(&mut s.turning, "Turning circle");
                    ui.checkbox(&mut s.zigzag, "Zig-zag");
                    ui.checkbox(&mut s.stopping, "Stopping");
                    ui.checkbox(&mut s.waves, "Waves");
                });
                egui::Grid::new("trials-setup")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        let fraction = |ui: &mut Ui, label: &str, v: &mut f64, hint: &str| {
                            ui.label(label).on_hover_text(hint);
                            let mut pct = *v * 100.0;
                            let r = ui.add(
                                egui::DragValue::new(&mut pct)
                                    .range(5.0..=100.0)
                                    .speed(1.0)
                                    .suffix(" %"),
                            );
                            crate::studio::a11y::name(&r, label);
                            *v = pct / 100.0;
                            ui.end_row();
                        };
                        fraction(
                            ui,
                            "Approach thrust",
                            &mut s.approach_thrust,
                            "Forward thrust for turning, zig-zag and stopping",
                        );
                        fraction(
                            ui,
                            "Thruster yaw",
                            &mut s.yaw_command,
                            "Yaw command for vehicles without rudders, share of yaw authority",
                        );
                        let value = |ui: &mut Ui,
                                         label: &str,
                                         v: &mut f64,
                                         range: std::ops::RangeInclusive<f64>,
                                         speed: f64,
                                         unit: &str| {
                            ui.label(label);
                            let r = ui.add(
                                egui::DragValue::new(v)
                                    .range(range)
                                    .speed(speed)
                                    .suffix(unit),
                            );
                            crate::studio::a11y::name(&r, label);
                            ui.end_row();
                        };
                        value(ui, "Rudder angle", &mut s.rudder_deg, 1.0..=60.0, 0.5, "°");
                        value(
                            ui,
                            "Wave height",
                            &mut s.wave_height,
                            0.02..=5.0,
                            0.01,
                            " m",
                        );
                        value(
                            ui,
                            "Operating depth",
                            &mut s.operating_depth,
                            0.5..=11000.0,
                            0.5,
                            " m",
                        );
                        ui.label("Zig-zags");
                        ui.horizontal(|ui| {
                            for angle in [10.0, 20.0] {
                                let mut on = s.zigzag_angles.contains(&angle);
                                if ui.checkbox(&mut on, format!("{angle:.0}°/{angle:.0}°")).changed() {
                                    s.zigzag_angles.retain(|a| *a != angle);
                                    if on {
                                        s.zigzag_angles.push(angle);
                                        s.zigzag_angles.sort_by(f64::total_cmp);
                                    }
                                }
                            }
                        });
                        ui.end_row();
                        ui.label("Conditions");
                        ui.checkbox(&mut p.scene_conditions, "Scene's current, sea and depth")
                            .on_hover_text(
                                "Off: still, deep water. On: manoeuvres in the scene's current and \
                                 irregular sea (free decay and wave response stay in still water).",
                            );
                        ui.end_row();
                        if !p.scene_conditions {
                            value(
                                ui,
                                "Basin depth",
                                &mut p.water_depth,
                                5.0..=11000.0,
                                1.0,
                                " m",
                            );
                        }
                        ui.label("Limits");
                        ui.checkbox(&mut p.imo, "IMO reference (MSC.137(76))")
                            .on_hover_text(
                                "Advance ≤ 4.5 L, tactical diameter ≤ 5 L, initial turning ≤ 2.5 L, \
                                 zig-zag overshoots by L/V, stopping ≤ 15 L. Defined for ships \
                                 with rudders; a guide for other craft.",
                            );
                        ui.end_row();
                        let optional = |ui: &mut Ui,
                                            label: &str,
                                            v: &mut Option<f64>,
                                            default: f64,
                                            unit: &str| {
                            let mut on = v.is_some();
                            ui.checkbox(&mut on, label);
                            let mut x = v.unwrap_or(default);
                            let r = ui.add_enabled(
                                on,
                                egui::DragValue::new(&mut x)
                                    .range(0.0..=100.0)
                                    .speed(0.05)
                                    .suffix(unit),
                            );
                            crate::studio::a11y::name(&r, &format!("{label} limit"));
                            *v = on.then_some(x);
                            ui.end_row();
                        };
                        optional(ui, "Min top speed", &mut p.min_speed, 2.0, " m/s");
                        optional(ui, "Max roll response", &mut p.max_roll_rao, 2.0, " °/°");
                    });
            });
    }

    fn trials_charts(&mut self, ui: &mut Ui) {
        let Some(report) = self.trials.result.clone() else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No trial results yet").color(theme::MUTED));
                ui.label(
                    RichText::new(
                        "Select a vehicle (or import a definition), then Run trials.\n\
                         Charts of each trial appear here.",
                    )
                    .small()
                    .color(theme::MUTED),
                );
            });
            return;
        };
        ui.add_space(4.0);
        let available = [
            (Chart::Decay, "Free decay", !report.decay.is_empty()),
            (Chart::Speed, "Speed", !report.speed.is_empty()),
            (
                Chart::Turning,
                "Turning",
                !report.turning.is_empty() || report.stopping.is_some(),
            ),
            (Chart::Zigzag, "Zig-zag", !report.zigzag.is_empty()),
            (Chart::Waves, "Waves", !report.waves.is_empty()),
        ];
        ui.horizontal_wrapped(|ui| {
            for (chart, label, present) in available {
                if present {
                    ui.selectable_value(&mut self.trials.chart, chart, label);
                }
            }
        });
        ui.add_space(4.0);
        let p = &mut self.trials;
        match p.chart {
            Chart::Decay => {
                ui.horizontal(|ui| {
                    for (i, d) in report.decay.iter().enumerate() {
                        ui.selectable_value(&mut p.decay_axis, i, capitalise(&d.axis));
                    }
                });
                if let Some(d) = report.decay.get(p.decay_axis.min(report.decay.len() - 1)) {
                    let unit = if d.unit == "m" { "m" } else { "°" };
                    chart(
                        ui,
                        &format!("{} free decay", capitalise(&d.axis)),
                        &[Series {
                            name: &d.axis,
                            colour: SERIES[0],
                            points: d.series.clone(),
                        }],
                        "time (s)",
                        &format!("{} ({unit})", d.axis),
                        Options {
                            guides: vec![d.equilibrium],
                            room: p.chart_room,
                            ..Options::default()
                        },
                    );
                    ui.label(
                        RichText::new(format!(
                            "Period {} s · damping ratio {} · linear estimate {} s{}",
                            fmt(d.period_s, 2),
                            fmt(d.damping_ratio, 3),
                            fmt(d.predicted_period_s, 2),
                            d.metacentric_height_m
                                .map(|g| format!(" · GM {g:.3} m"))
                                .unwrap_or_default()
                        ))
                        .small(),
                    );
                }
            }
            Chart::Speed => {
                let points: Vec<[f64; 2]> = report
                    .speed
                    .iter()
                    .map(|s| [s.thrust_n, s.speed_mps])
                    .collect();
                chart(
                    ui,
                    "Speed through the water against thrust",
                    &[Series {
                        name: "speed",
                        colour: SERIES[0],
                        points,
                    }],
                    "thrust (N)",
                    "speed (m/s)",
                    Options {
                        origin: true,
                        markers: true,
                        room: p.chart_room,
                        ..Options::default()
                    },
                );
            }
            Chart::Turning => {
                let mut series = vec![];
                for (i, t) in report.turning.iter().enumerate() {
                    series.push(Series {
                        name: if t.side == "port" {
                            "Port turn"
                        } else {
                            "Starboard turn"
                        },
                        colour: SERIES[i.min(1)],
                        points: t.track.iter().map(|p| [p[1], p[2]]).collect(),
                    });
                }
                if let Some(s) = &report.stopping {
                    series.push(Series {
                        name: "Crash stop",
                        colour: SERIES[2],
                        points: s.track.iter().map(|p| [p[1], p[2]]).collect(),
                    });
                }
                chart(
                    ui,
                    "Tracks from the execute point",
                    &series,
                    "along original course (m)",
                    "to port (m)",
                    Options {
                        equal: true,
                        origin: true,
                        room: p.chart_room,
                        ..Options::default()
                    },
                );
                if let Some(t) = report.turning.first() {
                    let l = report.length_m;
                    ui.label(
                        RichText::new(format!(
                            "Advance {} m ({}) · tactical diameter {} m ({}) · heel up to {:.0}°",
                            fmt(t.advance_m, 1),
                            lengths(t.advance_m, l),
                            fmt(t.tactical_diameter_m, 1),
                            lengths(t.tactical_diameter_m, l),
                            t.max_tilt_deg
                        ))
                        .small(),
                    );
                }
            }
            Chart::Zigzag => {
                let series: Vec<Series> = report
                    .zigzag
                    .iter()
                    .enumerate()
                    .map(|(i, z)| Series {
                        name: if z.angle_deg == 10.0 {
                            "10°/10°"
                        } else if z.angle_deg == 20.0 {
                            "20°/20°"
                        } else {
                            "zig-zag"
                        },
                        colour: SERIES[i.min(2)],
                        points: z.series.iter().map(|p| [p[0], p[1]]).collect(),
                    })
                    .collect();
                let mut guides = vec![0.0];
                for z in &report.zigzag {
                    guides.push(z.angle_deg);
                    guides.push(-z.angle_deg);
                }
                chart(
                    ui,
                    "Heading during the zig-zag",
                    &series,
                    "time (s)",
                    "heading change (°, + to port)",
                    Options {
                        guides,
                        room: p.chart_room,
                        ..Options::default()
                    },
                );
            }
            Chart::Waves => {
                let headings: Vec<f64> = {
                    let mut h: Vec<f64> = report.waves.iter().map(|w| w.heading_deg).collect();
                    h.dedup();
                    h
                };
                ui.horizontal(|ui| {
                    for (i, h) in headings.iter().enumerate() {
                        let name = match *h as i64 {
                            180 => "Head seas".to_string(),
                            90 => "Beam seas".to_string(),
                            0 => "Following seas".to_string(),
                            other => format!("{other}°"),
                        };
                        ui.selectable_value(&mut p.wave_heading, i, name);
                    }
                });
                let heading = headings[p.wave_heading.min(headings.len() - 1)];
                let cases: Vec<_> = report
                    .waves
                    .iter()
                    .filter(|w| w.heading_deg == heading)
                    .collect();
                let line = |f: fn(&pelagic::trials::WaveResponse) -> f64| {
                    cases.iter().map(|w| [w.period_s, f(w)]).collect::<Vec<_>>()
                };
                chart(
                    ui,
                    "Response in regular waves",
                    &[
                        Series {
                            name: "Heave (m/m)",
                            colour: SERIES[0],
                            points: line(|w| w.heave_rao),
                        },
                        Series {
                            name: "Roll (°/° slope)",
                            colour: SERIES[1],
                            points: line(|w| w.roll_rao),
                        },
                        Series {
                            name: "Pitch (°/° slope)",
                            colour: SERIES[2],
                            points: line(|w| w.pitch_rao),
                        },
                    ],
                    "wave period (s)",
                    "response ratio",
                    Options {
                        origin: true,
                        markers: true,
                        room: p.chart_room,
                        ..Options::default()
                    },
                );
            }
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn fmt(v: Option<f64>, digits: usize) -> String {
    v.map_or_else(|| "—".into(), |v| format!("{v:.digits$}"))
}

fn lengths(v: Option<f64>, l: f64) -> String {
    v.map_or_else(|| "—".into(), |v| format!("{:.2} L", v / l))
}

/// Summary, checks, warnings and skipped trials.
fn trial_results(ui: &mut Ui, report: &Report) {
    ui.add_space(6.0);
    let failed = report.failed();
    let total = report.checks.len();
    ui.horizontal_wrapped(|ui| {
        if total > 0 {
            let (colour, text) = if failed == 0 {
                (
                    theme::GOOD,
                    if total == 1 {
                        "✔ The check passed".to_string()
                    } else {
                        format!("✔ All {total} checks passed")
                    },
                )
            } else {
                (theme::BAD, format!("✖ {failed} of {total} checks failed"))
            };
            theme::chip(ui, colour, &text);
        }
        theme::chip(ui, theme::ACCENT, &report.mode);
        theme::chip(
            ui,
            theme::ACCENT,
            &format!("steered by {}", report.steering),
        );
        theme::chip(ui, theme::MUTED, &format!("L {:.2} m", report.length_m));
    });
    ui.add_space(2.0);
    egui::Grid::new("trial-figures")
        .num_columns(2)
        .spacing([12.0, 3.0])
        .show(ui, |ui| {
            let mut figure = |label: &str, value: String| {
                ui.label(RichText::new(label).color(theme::MUTED));
                ui.label(RichText::new(value).color(theme::STRONG));
                ui.end_row();
            };
            if let Some(top) = report.speed.last() {
                figure(
                    "Top speed",
                    format!(
                        "{:.2} m/s ({:.1} kn) at {:.0} N",
                        top.speed_mps,
                        top.speed_mps * 1.943_844,
                        top.thrust_n
                    ),
                );
            }
            if let Some(t) = report.turning.first() {
                figure(
                    "Tactical diameter",
                    format!(
                        "{} m ({}){}",
                        fmt(t.tactical_diameter_m, 1),
                        lengths(t.tactical_diameter_m, report.length_m),
                        t.rudder_deg
                            .map(|r| format!(" at {r:.0}° rudder"))
                            .unwrap_or_default()
                    ),
                );
            }
            for z in &report.zigzag {
                figure(
                    &format!("{0:.0}°/{0:.0}° overshoots", z.angle_deg),
                    format!(
                        "{}° / {}°",
                        fmt(z.first_overshoot_deg, 1),
                        fmt(z.second_overshoot_deg, 1)
                    ),
                );
            }
            if let Some(s) = &report.stopping {
                figure(
                    "Stopping",
                    format!(
                        "{} m in {} s ({})",
                        fmt(s.track_reach_m, 1),
                        fmt(s.time_to_stop_s, 1),
                        lengths(s.track_reach_m, report.length_m)
                    ),
                );
            }
            for d in &report.decay {
                figure(
                    &format!("{} period", capitalise(&d.axis)),
                    format!("{} s · ζ {}", fmt(d.period_s, 2), fmt(d.damping_ratio, 3)),
                );
            }
        });
    if !report.checks.is_empty() {
        ui.add_space(4.0);
        egui::Grid::new("trial-checks")
            .num_columns(3)
            .spacing([10.0, 2.0])
            .show(ui, |ui| {
                for h in ["Check", "Value", "Limit"] {
                    ui.label(RichText::new(h).small().color(theme::MUTED));
                }
                ui.end_row();
                for c in &report.checks {
                    let (mark, colour) = if c.passed {
                        ("✔", theme::GOOD)
                    } else {
                        ("✖", theme::BAD)
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(mark).color(colour));
                        ui.label(&c.name);
                    });
                    ui.label(format!("{} {}", fmt(c.value, 2), c.unit));
                    ui.label(format!(
                        "{} {} {}",
                        if c.bound == "max" { "≤" } else { "≥" },
                        c.limit,
                        c.unit
                    ));
                    ui.end_row();
                }
            });
    }
    for w in &report.warnings {
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("⚠").color(theme::WARN));
            ui.label(RichText::new(w).small().color(theme::TEXT));
        });
    }
    for s in &report.skipped {
        ui.label(
            RichText::new(format!("Skipped: {s}"))
                .small()
                .color(theme::MUTED),
        );
    }
    ui.add_space(6.0);
}
