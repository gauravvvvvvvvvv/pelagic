//! Seabed survey in the editor: plan multibeam or side-scan lines over the
//! scene's seabed for an IHO S-44 order, see the coverage, gaps and depth
//! uncertainty, and run the lines with the selected vehicle's physics.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use crate::studio::charts::{Options, SERIES, SURFACE, Series, chart};
use egui::{Align2, Color32, FontId, Pos2, RichText, Sense, Shape, Stroke, Ui, Vec2};
use pelagic::survey::{self, ORDERS, Report, Setup, Sonar, SonarKind};
use std::sync::Arc;

const KNOT: f64 = 0.514_444;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layer {
    Coverage,
    Depth,
    Tvu,
}

pub(super) struct Panel {
    setup: Setup,
    result: Option<Arc<Report>>,
    layer: Layer,
    /// Show the simulated run (else the plan).
    simulated: bool,
    texture: Option<((usize, bool, u8), egui::TextureHandle)>,
    /// Map scale (m per point) and centre; None fits the area.
    view: Option<(f64, [f64; 2])>,
    /// The area was fitted to this scene's seabed (by scene name).
    fitted: Option<String>,
    fold_setup: bool,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            setup: Setup::default(),
            result: None,
            layer: Layer::Coverage,
            simulated: true,
            texture: None,
            view: None,
            fitted: None,
            fold_setup: false,
        }
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a[0] + (b[0] - a[0]) * t) as u8,
        (a[1] + (b[1] - a[1]) * t) as u8,
        (a[2] + (b[2] - a[2]) * t) as u8,
    )
}
const GAP: Color32 = Color32::from_rgb(0xd9, 0x59, 0x26);
const MET: Color32 = Color32::from_rgb(0x39, 0x87, 0xe5);
const EXTRA: Color32 = Color32::from_rgb(0x19, 0x9e, 0x70);
/// Land or water too shallow for the platform.
const DRY: Color32 = Color32::from_rgb(78, 72, 60);

/// Fits the survey area to the seabed, 10 % inside its edges.
fn fit_area(setup: &mut Setup, scene: &pelagic::scene::Scene) {
    let b = survey::seabed(scene).bounds();
    setup.area.centre = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let (w, h) = ((b[2] - b[0]) * 0.8, (b[3] - b[1]) * 0.8);
    let a = setup.area.axis_deg.to_radians();
    // Lines along the axis: length along it.
    let (s, c) = (a.sin().abs(), a.cos().abs());
    let (along, across) = if s > c { (w, h) } else { (h, w) };
    setup.area.length_m = along.clamp(5.0, 50_000.0);
    setup.area.width_m = across.clamp(5.0, 50_000.0);
}

impl State {
    pub(super) fn survey_finished(&mut self, report: Report) {
        let st = &report.ideal.stats;
        self.log(format!(
            "Survey plan: {} lines, {:.2} line km, planned coverage {:.1} % within {} TVU.",
            report.lines.len(),
            report.ideal.line_km,
            st.covered_percent,
            report.setup.order().name
        ));
        if let Some(r) = &report.simulated {
            self.log(format!(
                "Survey with {}: coverage {:.1} %, {} gaps, cross-track {:.2} m RMS, {:.2} h.",
                report.vehicle.clone().unwrap_or_default(),
                r.stats.covered_percent,
                r.stats.gaps,
                r.cross_track_rms_m,
                r.hours
            ));
        }
        for w in report
            .warnings
            .iter()
            .chain(report.simulated.iter().flat_map(|r| &r.notes))
        {
            self.log(format!("Survey: {w}"));
        }
        let p = &mut self.survey;
        p.simulated = report.simulated.is_some();
        p.texture = None;
        p.view = None;
        p.fold_setup = true;
        p.result = Some(Arc::new(report));
    }

    /// Plans the lines (and, with `physics`, runs them with the selected
    /// vehicle) in the background.
    pub(super) fn start_survey(&mut self, physics: bool) {
        let setup = self.survey.setup.clone();
        if let Err(e) = setup.validate() {
            return self.log(e);
        }
        let vehicle = if physics {
            match self.trial_subject() {
                Some(v) => Some(v),
                None => {
                    return self.log(
                        "Select a vehicle in the Outliner to run the survey with its physics.",
                    );
                }
            }
        } else {
            None
        };
        let scene = self.scene().clone();
        self.survey.result = None;
        self.survey.texture = None;
        let label = match &vehicle {
            Some(v) => format!("Surveying with {}…", v.name),
            None => "Planning survey lines…".into(),
        };
        self.start_job(&label, move |cancel, send| {
            let report = survey::run(&scene, &setup, vehicle.as_ref(), &cancel, |f| {
                let _ = send.send(Event::Progress(format!(
                    "Running the lines: {:.0} %",
                    f * 100.0
                )));
            })?;
            Ok(Output::Survey(Box::new(report)))
        });
    }

    fn export_survey(&mut self) {
        let Some(report) = self.survey.result.clone() else {
            return;
        };
        let Some(parent) = native::pick_folder(self.hwnd, "Choose a folder for the survey") else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-survey-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        match report.write(&folder) {
            Ok(()) => self.log(format!(
                "Survey saved (summary, lines, coverage/TVU/depth grids, GPX, QGC mission): {}",
                folder.display()
            )),
            Err(e) => self.log(format!("Could not save the survey: {e}")),
        }
    }

    /// Script hooks: fit the area, choose the sensor and order.
    pub(super) fn survey_script(&mut self, command: &str) {
        match command {
            "fit" => {
                let scene = self.scene().clone();
                fit_area(&mut self.survey.setup, &scene);
            }
            "sss" => {
                self.survey.setup.sonar = Sonar::side_scan();
                self.survey.setup.platform.underwater = true;
            }
            "mbes" => self.survey.setup.sonar = Sonar::multibeam(),
            other if other.starts_with("area=") => {
                // area=CX,CY,LENGTH,WIDTH,HEADING (scene metres, degrees).
                let v: Vec<f64> = other[5..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if let [x, y, l, w, h] = v[..] {
                    let a = &mut self.survey.setup.area;
                    a.centre = [x, y];
                    a.length_m = l;
                    a.width_m = w;
                    a.axis_deg = h;
                    // Keep this area when the tab first opens.
                    self.survey.fitted = Some(self.scene().name.clone());
                }
            }
            other => {
                if let Some(o) = ORDERS.iter().position(|o| {
                    let name = o.name.to_ascii_lowercase();
                    name.replace(' ', "-") == other || name.split(' ').any(|word| word == other)
                }) {
                    self.survey.setup.order = o;
                }
            }
        }
    }

    pub(super) fn survey_panel(&mut self, ui: &mut Ui) {
        let name = self.scene().name.clone();
        if self.survey.fitted.as_deref() != Some(name.as_str()) {
            let scene = self.scene().clone();
            fit_area(&mut self.survey.setup, &scene);
            self.survey.fitted = Some(name);
        }
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("survey-left")
                .show(left, |ui| self.survey_controls(ui));
            let room = right.available_height();
            egui::ScrollArea::vertical()
                .id_salt("survey-right")
                .show(right, |ui| self.survey_results(ui, room));
        });
    }

    fn survey_controls(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(4.0);
        theme::heading(ui, "Seabed survey");
        ui.label(
            RichText::new(
                "Survey lines for a multibeam or side-scan sonar over this scene's seabed, \
                 judged against IHO S-44, and run with a vehicle's own physics.",
            )
            .small()
            .color(theme::MUTED),
        );
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
        let subject = self.trial_subject();
        ui.horizontal_wrapped(|ui| {
            let plan = ui
                .add_enabled(!busy, theme::primary("▶ Plan survey"))
                .on_hover_text("Lines, planned coverage, gaps and uncertainty");
            crate::studio::a11y::name(&plan, "Plan survey");
            if plan.clicked() {
                self.start_survey(false);
            }
            let run = ui
                .add_enabled(
                    !busy && subject.is_some(),
                    egui::Button::new("▶ Run with vehicle physics"),
                )
                .on_hover_text(
                    "The selected vehicle runs the lines in the scene's sea and current; the swath follows its real pose",
                );
            crate::studio::a11y::name(&run, "Run survey with vehicle physics");
            if run.clicked() {
                self.start_survey(true);
            }
            if self.survey.result.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new("💾 Export…"))
                    .clicked()
            {
                self.export_survey();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Vehicle").color(theme::MUTED));
            match &subject {
                Some(e) => ui.label(RichText::new(&e.name).strong().color(theme::STRONG)),
                None => ui.label(
                    RichText::new("select one in the Outliner for a physics run")
                        .color(theme::MUTED),
                ),
            };
        });
        let fold = std::mem::take(&mut self.survey.fold_setup).then_some(false);
        let scene = self.scene().clone();
        egui::CollapsingHeader::new(RichText::new("Sensor and standard").strong())
            .id_salt("survey-sensor")
            .default_open(true)
            .open(fold)
            .show(ui, |ui| {
                let s = &mut self.survey.setup;
                egui::Grid::new("survey-sensor-grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Sonar");
                        ui.horizontal(|ui| {
                            if ui
                                .radio(s.sonar.kind == SonarKind::Multibeam, "Multibeam")
                                .clicked()
                            {
                                s.sonar = Sonar::multibeam();
                            }
                            if ui
                                .radio(s.sonar.kind == SonarKind::SideScan, "Side-scan")
                                .clicked()
                            {
                                s.sonar = Sonar::side_scan();
                                s.platform.underwater = true;
                            }
                        });
                        ui.end_row();
                        let field = |ui: &mut Ui, label: &str, v: &mut f64, lo: f64, hi: f64, speed: f64, suffix: &str| {
                            ui.label(label);
                            let r = ui.add(
                                egui::DragValue::new(v)
                                    .range(lo..=hi)
                                    .speed(speed)
                                    .suffix(suffix),
                            );
                            crate::studio::a11y::name(&r, label);
                            ui.end_row();
                        };
                        if s.sonar.kind == SonarKind::Multibeam {
                            field(ui, "Swath angle", &mut s.sonar.swath_deg, 10.0, 170.0, 1.0, "°");
                            field(ui, "Maximum range", &mut s.sonar.range_m, 5.0, 12_000.0, 5.0, " m");
                            ui.label("Beams");
                            ui.add(egui::DragValue::new(&mut s.sonar.beams).range(2..=4096));
                            ui.end_row();
                        } else {
                            field(ui, "Range per side", &mut s.sonar.range_m, 5.0, 2000.0, 1.0, " m");
                            field(ui, "Blind zone", &mut s.sonar.blind_zone, 0.0, 5.0, 0.05, " × altitude");
                        }
                        field(ui, "Ping rate", &mut s.sonar.ping_hz, 0.1, 100.0, 0.1, " Hz");
                        ui.label("Platform");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut s.platform.underwater, false, "Surface");
                            ui.radio_value(&mut s.platform.underwater, true, "Underwater");
                        });
                        ui.end_row();
                        if s.platform.underwater {
                            field(ui, "Altitude", &mut s.platform.altitude_m, 0.5, 500.0, 0.1, " m");
                        } else {
                            field(ui, "Transducer depth", &mut s.platform.draft_m, 0.0, 30.0, 0.05, " m");
                        }
                        ui.label("IHO S-44 order");
                        let combo = egui::ComboBox::from_id_salt("survey-order")
                            .selected_text(s.order().name)
                            .show_ui(ui, |ui| {
                                for (i, o) in ORDERS.iter().enumerate() {
                                    ui.selectable_value(&mut s.order, i, o.name).on_hover_text(
                                        format!(
                                            "TVU a {:.2} m, b {:.4} · coverage {:.0} %{}",
                                            o.tvu_a,
                                            o.tvu_b,
                                            o.coverage_percent,
                                            o.feature_m.map_or(String::new(), |f| format!(
                                                " · features > {f} m"
                                            ))
                                        ),
                                    );
                                }
                            });
                        crate::studio::a11y::name(&combo.response, "IHO S-44 order");
                        ui.end_row();
                    });
                let depth = -pelagic::ocean::Bathymetry::elevation(
                    &survey::seabed(&scene),
                    s.area.centre[0],
                    s.area.centre[1],
                );
                let o = s.order();
                ui.label(
                    RichText::new(format!(
                        "At {depth:.0} m: TVU within {:.2} m, THU within {:.1} m, {:.0} % coverage{}.",
                        o.tvu(depth),
                        o.thu(depth),
                        o.coverage_percent,
                        o.feature(depth)
                            .map_or(String::new(), |f| format!(", features over {f:.1} m"))
                    ))
                    .small()
                    .color(theme::MUTED),
                );
                egui::CollapsingHeader::new("Sounding uncertainty model")
                    .id_salt("survey-uncertainty")
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut s.sonar.sigma_fixed_m)
                                    .range(0.0..=2.0)
                                    .speed(0.005)
                                    .prefix("fixed ")
                                    .suffix(" m"),
                            )
                            .on_hover_text("Heave, draft and tide (1σ)");
                            let mut pct = s.sonar.sigma_range * 100.0;
                            ui.add(
                                egui::DragValue::new(&mut pct)
                                    .range(0.0..=5.0)
                                    .speed(0.01)
                                    .prefix("range ")
                                    .suffix(" %"),
                            )
                            .on_hover_text("Sound speed and range (1σ, share of slant range)");
                            s.sonar.sigma_range = pct / 100.0;
                            ui.add(
                                egui::DragValue::new(&mut s.sonar.sigma_angle_deg)
                                    .range(0.0..=5.0)
                                    .speed(0.01)
                                    .prefix("pointing ")
                                    .suffix("°"),
                            )
                            .on_hover_text("Attitude, refraction and beam pointing (1σ)");
                        });
                    });
            });
        egui::CollapsingHeader::new(RichText::new("Area and lines").strong())
            .id_salt("survey-area")
            .default_open(true)
            .open(fold)
            .show(ui, |ui| {
                let s = &mut self.survey.setup;
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .small_button("Fit to seabed")
                        .on_hover_text("The scene's seabed, 10 % inside its edges")
                        .clicked()
                    {
                        fit_area(s, &scene);
                    }
                    if ui
                        .small_button("Lines along the contours")
                        .on_hover_text("Lines across the slope keep the swath width steady")
                        .clicked()
                    {
                        s.area.axis_deg = survey::contour_heading(&survey::seabed(&scene), &s.area);
                    }
                });
                egui::Grid::new("survey-area-grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Centre");
                        ui.horizontal(|ui| {
                            for (i, axis) in ["x ", "y "].iter().enumerate() {
                                ui.add(
                                    egui::DragValue::new(&mut s.area.centre[i])
                                        .speed(1.0)
                                        .prefix(*axis)
                                        .suffix(" m"),
                                );
                            }
                        });
                        ui.end_row();
                        ui.label("Size");
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::DragValue::new(&mut s.area.length_m)
                                    .range(5.0..=50_000.0)
                                    .speed(2.0)
                                    .suffix(" m along"),
                            );
                            crate::studio::a11y::name(&r, "Area length");
                            let r = ui.add(
                                egui::DragValue::new(&mut s.area.width_m)
                                    .range(5.0..=50_000.0)
                                    .speed(2.0)
                                    .suffix(" m across"),
                            );
                            crate::studio::a11y::name(&r, "Area width");
                        });
                        ui.end_row();
                        ui.label("Line heading");
                        let r = ui.add(
                            egui::DragValue::new(&mut s.area.axis_deg)
                                .range(0.0..=359.9)
                                .speed(1.0)
                                .suffix("°"),
                        );
                        crate::studio::a11y::name(&r, "Line heading");
                        ui.end_row();
                        ui.label("Overlap");
                        let mut pct = s.overlap * 100.0;
                        let r = ui.add(
                            egui::DragValue::new(&mut pct)
                                .range(0.0..=90.0)
                                .speed(1.0)
                                .suffix(" %"),
                        );
                        crate::studio::a11y::name(&r, "Overlap");
                        s.overlap = pct / 100.0;
                        ui.end_row();
                        ui.label("Survey speed");
                        let mut kn = s.speed_mps / KNOT;
                        let r = ui.add(
                            egui::DragValue::new(&mut kn)
                                .range(0.2..=30.0)
                                .speed(0.1)
                                .suffix(" kn"),
                        );
                        crate::studio::a11y::name(&r, "Survey speed");
                        s.speed_mps = kn * KNOT;
                        ui.end_row();
                        ui.label("Run-in");
                        ui.add(
                            egui::DragValue::new(&mut s.run_in_m)
                                .range(0.0..=2000.0)
                                .speed(1.0)
                                .suffix(" m"),
                        )
                        .on_hover_text("Straight approach before each line so the vehicle settles");
                        ui.end_row();
                    });
            });
        let Some(report) = self.survey.result.clone() else {
            return;
        };
        ui.add_space(6.0);
        theme::heading(ui, "Results");
        let row = |ui: &mut Ui, name: &str, r: &survey::Run, lines: usize| {
            let st = &r.stats;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(name).strong());
                theme::chip(
                    ui,
                    if st.meets_order {
                        theme::GOOD
                    } else {
                        theme::WARN
                    },
                    &format!(
                        "{} {:.1} %",
                        if st.required_passes > 1 {
                            "200 %:"
                        } else {
                            "coverage"
                        },
                        if st.required_passes > 1 {
                            st.multiple_percent
                        } else {
                            st.covered_percent
                        }
                    ),
                );
                theme::chip(
                    ui,
                    if st.gaps == 0 {
                        theme::GOOD
                    } else {
                        theme::WARN
                    },
                    &format!("{} gaps", st.gaps),
                );
            });
            ui.label(
                RichText::new(format!(
                    "{lines} lines · {:.2} km on line · {:.2} h · any data {:.1} %{}{}",
                    r.line_km,
                    r.hours,
                    st.raw_percent,
                    st.tvu_mean_m
                        .map_or(String::new(), |t| format!(" · mean TVU {t:.2} m")),
                    if st.gaps > 0 {
                        format!(" · largest gap {:.0} m²", st.largest_gap_m2)
                    } else {
                        String::new()
                    }
                ))
                .small()
                .color(theme::MUTED),
            );
        };
        row(ui, "Planned", &report.ideal, report.lines.len());
        if let Some(r) = &report.simulated {
            row(
                ui,
                &format!("With {}", report.vehicle.clone().unwrap_or_default()),
                r,
                r.lines_done,
            );
            ui.label(
                RichText::new(format!(
                    "Cross-track {:.2} m RMS (max {:.1} m) · roll {:.2}° · pitch {:.2}° RMS on line",
                    r.cross_track_rms_m, r.cross_track_max_m, r.roll_rms_deg, r.pitch_rms_deg
                ))
                .small()
                .color(theme::MUTED),
            );
        }
        let d = &report.density;
        ui.label(
            RichText::new(format!(
                "Soundings every {:.2} m along track, {:.2} m across at the swath edge{}",
                d.along_m,
                d.across_edge_m,
                match (d.feature_m, d.resolves_feature) {
                    (Some(f), Some(true)) => format!(" — fine enough for {f:.1} m features"),
                    (Some(f), Some(false)) => format!(" — too coarse for {f:.1} m features"),
                    _ => String::new(),
                }
            ))
            .small()
            .color(theme::MUTED),
        );
        for w in report
            .warnings
            .iter()
            .chain(report.simulated.iter().flat_map(|r| &r.notes))
        {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("⚠").color(theme::WARN));
                ui.label(RichText::new(w).small());
            });
        }
    }

    fn survey_results(&mut self, ui: &mut Ui, room: f32) {
        ui.add_space(4.0);
        let Some(report) = self.survey.result.clone() else {
            theme::heading(ui, "Coverage map");
            ui.label(
                RichText::new(
                    "Plan the survey to see the lines and the coverage they give: \
                     blue meets the order, green has extra passes, orange is a gap.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        ui.horizontal_wrapped(|ui| {
            let p = &mut self.survey;
            for (layer, name) in [
                (Layer::Coverage, "Coverage"),
                (Layer::Depth, "Depth"),
                (Layer::Tvu, "Uncertainty"),
            ] {
                if layer == Layer::Tvu && report.setup.sonar.kind != SonarKind::Multibeam {
                    continue;
                }
                if ui.selectable_label(p.layer == layer, name).clicked() {
                    p.layer = layer;
                }
            }
            if report.simulated.is_some() {
                ui.separator();
                ui.selectable_value(&mut p.simulated, false, "Planned");
                ui.selectable_value(
                    &mut p.simulated,
                    true,
                    format!("With {}", report.vehicle.clone().unwrap_or_default()),
                );
            }
        });
        let show_sim = self.survey.simulated && report.simulated.is_some();
        let run = if show_sim {
            report.simulated.as_ref().expect("simulated")
        } else {
            &report.ideal
        };
        let chart_room = if show_sim { 250.0 } else { 40.0 };
        self.survey_map(ui, &report, run, (room - chart_room - 70.0).max(260.0));
        if show_sim && !run.cross_track.is_empty() {
            let t0 = run.cross_track[0][0];
            chart(
                ui,
                "Cross-track error on the survey lines (m, + to the left)",
                &[Series {
                    name: "Cross-track",
                    colour: SERIES[0],
                    points: run
                        .cross_track
                        .iter()
                        .map(|p| [(p[0] - t0) / 60.0, p[1]])
                        .collect(),
                }],
                "minutes",
                "metres",
                Options {
                    origin: true,
                    room: 220.0,
                    ..Default::default()
                },
            );
        }
        ui.label(
            RichText::new(
                "IHO S-44 Ed. 6.2.0 Table 1 limits. Sounding uncertainty from the sensor model \
                 above; positioning (THU) and refraction are not simulated.",
            )
            .small()
            .color(theme::MUTED),
        );
    }

    fn survey_map(&mut self, ui: &mut Ui, report: &Report, run: &survey::Run, max_height: f32) {
        let width = ui.available_width().max(240.0);
        let height = (width * 0.75).clamp(240.0, 620.0).min(max_height);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        crate::studio::a11y::name(&response, "Survey coverage map");
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, SURFACE);
        let c = &run.coverage;
        let area = c.area;
        // Texture of the chosen layer.
        let key = (
            Arc::as_ptr(self.survey.result.as_ref().expect("result")) as usize,
            self.survey.simulated,
            self.survey.layer as u8,
        );
        if self.survey.texture.as_ref().is_none_or(|(k, _)| *k != key) {
            let stride = (c.cols.max(c.rows) / 2048 + 1).max(1);
            let (w, h) = (c.cols.div_ceil(stride), c.rows.div_ceil(stride));
            let need = report.setup.order().passes();
            let (dmin, dmax) = (
                run.stats.min_depth_m,
                run.stats.max_depth_m.max(run.stats.min_depth_m + 0.1),
            );
            let order = report.setup.order();
            let mut pixels = Vec::with_capacity(w * h);
            // Rows top to bottom = +across (right of the axis) first.
            for y in 0..h {
                for x in 0..w {
                    let (i, j) = (x * stride, (h - 1 - y) * stride);
                    let k = j.min(c.rows - 1) * c.cols + i.min(c.cols - 1);
                    let colour = match self.survey.layer {
                        _ if c.dry[k] => DRY,
                        Layer::Coverage => match c.passes[k] {
                            p if p < need => {
                                if c.raw[k] > 0 {
                                    // Data, but outside the order's TVU.
                                    Color32::from_rgb(0x8a, 0x3f, 0x22)
                                } else {
                                    GAP
                                }
                            }
                            p if p == need => MET,
                            _ => EXTRA,
                        },
                        Layer::Depth => {
                            let t = ((f64::from(c.depth[k]) - dmin) / (dmax - dmin)) as f32;
                            mix([196.0, 222.0, 247.0], [16.0, 52.0, 110.0], t)
                        }
                        Layer::Tvu => {
                            let tvu = c.tvu[k];
                            if tvu.is_nan() {
                                Color32::from_rgb(40, 48, 56)
                            } else {
                                let allowed = order.tvu(f64::from(c.depth[k])) as f32;
                                if tvu > allowed {
                                    GAP
                                } else {
                                    mix([196.0, 222.0, 247.0], [16.0, 52.0, 110.0], tvu / allowed)
                                }
                            }
                        }
                    };
                    pixels.push(colour);
                }
            }
            let image = egui::ColorImage::new([w, h], pixels);
            let texture =
                ui.ctx()
                    .load_texture("survey-layer", image, egui::TextureOptions::NEAREST);
            self.survey.texture = Some((key, texture));
        }
        // View: fit the area (and the track) with a margin.
        let fit = || {
            let mut xs = vec![];
            let mut ys = vec![];
            for p in area.corners() {
                xs.push(p[0]);
                ys.push(p[1]);
            }
            for p in &run.track {
                xs.push(p[1]);
                ys.push(p[2]);
            }
            let (x0, x1) = xs
                .iter()
                .fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
            let (y0, y1) = ys
                .iter()
                .fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
            let scale = ((x1 - x0) / (rect.width() as f64 - 40.0))
                .max((y1 - y0) / (rect.height() as f64 - 40.0))
                .max(0.01);
            (scale * 1.05, [(x0 + x1) * 0.5, (y0 + y1) * 0.5])
        };
        if self.survey.view.is_none() || response.double_clicked() {
            self.survey.view = Some(fit());
        }
        let (mut scale, mut centre) = self.survey.view.unwrap_or((1.0, [0.0, 0.0]));
        if response.dragged() {
            let d = response.drag_delta();
            centre[0] -= d.x as f64 * scale;
            centre[1] += d.y as f64 * scale;
        }
        if let Some(pointer) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let before = [
                    centre[0] + (pointer.x - rect.center().x) as f64 * scale,
                    centre[1] - (pointer.y - rect.center().y) as f64 * scale,
                ];
                scale = (scale * (-scroll as f64 * 0.002).exp()).clamp(0.005, 1000.0);
                centre = [
                    before[0] - (pointer.x - rect.center().x) as f64 * scale,
                    before[1] + (pointer.y - rect.center().y) as f64 * scale,
                ];
            }
        }
        self.survey.view = Some((scale, centre));
        let mid = rect.center();
        let screen = |p: [f64; 2]| {
            Pos2::new(
                mid.x + ((p[0] - centre[0]) / scale) as f32,
                mid.y - ((p[1] - centre[1]) / scale) as f32,
            )
        };
        // Textured area: corners in (−along,−across), (+,−), (+,+), (−,+)
        // order; texture rows run from +across (top) to −across.
        if let Some((_, texture)) = &self.survey.texture {
            let k = area.corners();
            let mut mesh = egui::Mesh::with_texture(texture.id());
            let uv = [
                Pos2::new(0.0, 1.0),
                Pos2::new(1.0, 1.0),
                Pos2::new(1.0, 0.0),
                Pos2::new(0.0, 0.0),
            ];
            for (p, t) in k.iter().zip(uv) {
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: screen(*p),
                    uv: t,
                    color: Color32::WHITE,
                });
            }
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        }
        let mut outline: Vec<Pos2> = area.corners().iter().map(|p| screen(*p)).collect();
        outline.push(outline[0]);
        painter.add(Shape::line(outline, Stroke::new(1.0, theme::TEXT)));
        // Planned lines and, for a physics run, the track.
        for l in &report.lines {
            painter.line_segment(
                [screen(l.start), screen(l.end)],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(242, 247, 250, 150)),
            );
        }
        if self.survey.simulated && !run.track.is_empty() {
            let path: Vec<Pos2> = run.track.iter().map(|p| screen([p[1], p[2]])).collect();
            painter.add(Shape::line(
                path,
                Stroke::new(1.2, Color32::from_rgb(242, 182, 92)),
            ));
        }
        // Legend.
        let legend: Vec<(Color32, String)> = match self.survey.layer {
            Layer::Coverage => vec![
                (
                    MET,
                    format!(
                        "{} pass{} (meets the order)",
                        report.setup.order().passes(),
                        if report.setup.order().passes() == 1 {
                            ""
                        } else {
                            "es"
                        }
                    ),
                ),
                (EXTRA, "extra passes".into()),
                (GAP, "gap".into()),
                (
                    Color32::from_rgb(0x8a, 0x3f, 0x22),
                    "data outside the TVU limit".into(),
                ),
                (DRY, "land or too shallow".into()),
            ],
            Layer::Depth => vec![
                (
                    Color32::from_rgb(196, 222, 247),
                    format!("{:.1} m", run.stats.min_depth_m),
                ),
                (
                    Color32::from_rgb(16, 52, 110),
                    format!("{:.1} m", run.stats.max_depth_m),
                ),
            ],
            Layer::Tvu => vec![
                (
                    Color32::from_rgb(196, 222, 247),
                    "well within the TVU limit".into(),
                ),
                (Color32::from_rgb(16, 52, 110), "at the limit".into()),
                (GAP, "over the limit".into()),
            ],
        };
        // Legend below the map, so it never hides coverage.
        let track = self.survey.simulated && !run.track.is_empty();
        let legend_ui = |ui: &mut Ui| {
            ui.horizontal_wrapped(|ui| {
                for (colour, text) in &legend {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
                    ui.painter().rect_filled(r, 2.0, *colour);
                    ui.label(RichText::new(text).small().color(theme::TEXT));
                    ui.add_space(6.0);
                }
                let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 10.0), Sense::hover());
                ui.painter().line_segment(
                    [r.left_center(), r.right_center()],
                    Stroke::new(1.0, Color32::from_rgb(242, 247, 250)),
                );
                ui.label(RichText::new("planned lines").small().color(theme::TEXT));
                if track {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 10.0), Sense::hover());
                    ui.painter().line_segment(
                        [r.left_center(), r.right_center()],
                        Stroke::new(1.5, Color32::from_rgb(242, 182, 92)),
                    );
                    ui.label(RichText::new("vehicle track").small().color(theme::TEXT));
                }
            });
        };
        // North arrow and scale bar.
        let n = Pos2::new(rect.right() - 18.0, rect.top() + 28.0);
        painter.arrow(
            n + Vec2::new(0.0, 10.0),
            Vec2::new(0.0, -20.0),
            Stroke::new(1.5, theme::TEXT),
        );
        painter.text(
            n + Vec2::new(0.0, -14.0),
            Align2::CENTER_BOTTOM,
            "N",
            FontId::proportional(11.0),
            theme::TEXT,
        );
        let raw = scale * 110.0;
        let base = 10f64.powf(raw.log10().floor());
        let bar = [1.0, 2.0, 5.0, 10.0]
            .into_iter()
            .map(|m| m * base)
            .find(|s| *s >= raw)
            .unwrap_or(10.0 * base);
        let px = (bar / scale) as f32;
        let b0 = Pos2::new(rect.right() - 16.0 - px, rect.bottom() - 16.0);
        painter.line_segment([b0, b0 + Vec2::new(px, 0.0)], Stroke::new(2.0, theme::TEXT));
        painter.text(
            b0 + Vec2::new(px * 0.5, -4.0),
            Align2::CENTER_BOTTOM,
            if bar >= 1000.0 {
                format!("{} km", bar / 1000.0)
            } else {
                format!("{bar} m")
            },
            FontId::proportional(10.5),
            theme::TEXT,
        );
        let hint = if let Some(pointer) = response.hover_pos() {
            let w = [
                centre[0] + (pointer.x - mid.x) as f64 * scale,
                centre[1] - (pointer.y - mid.y) as f64 * scale,
            ];
            let (a, r) = area.axes();
            let d = [w[0] - area.centre[0], w[1] - area.centre[1]];
            let u = (d[0] * a[0] + d[1] * a[1] + area.length_m * 0.5) / c.cell_u;
            let v = (d[0] * r[0] + d[1] * r[1] + area.width_m * 0.5) / c.cell_v;
            if u >= 0.0 && v >= 0.0 && (u as usize) < c.cols && (v as usize) < c.rows {
                let k = v as usize * c.cols + u as usize;
                format!(
                    "x {:.0} y {:.0} m · depth {:.1} m · {} pass{} · TVU {}",
                    w[0],
                    w[1],
                    c.depth[k],
                    c.passes[k],
                    if c.passes[k] == 1 { "" } else { "es" },
                    if c.tvu[k].is_nan() {
                        "—".into()
                    } else {
                        format!(
                            "{:.2} m (limit {:.2})",
                            c.tvu[k],
                            report.setup.order().tvu(f64::from(c.depth[k]))
                        )
                    }
                )
            } else {
                format!("x {:.0} y {:.0} m", w[0], w[1])
            }
        } else {
            "drag to pan · scroll to zoom · double-click to fit".into()
        };
        painter.text(
            Pos2::new(rect.left() + 10.0, rect.bottom() - 10.0),
            Align2::LEFT_BOTTOM,
            hint,
            FontId::proportional(10.5),
            theme::TEXT,
        );
        legend_ui(ui);
    }
}
