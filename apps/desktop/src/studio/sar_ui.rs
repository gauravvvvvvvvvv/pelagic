//! Search and rescue in the editor: the drift of a search object from its
//! last known position under the live wind and current forecast, search
//! plans ranked by probability of success, and a map with a time slider
//! that shows the possible positions clearing as the search sweeps them.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use crate::studio::charts::{Options, SERIES, SURFACE, Series, chart};
use crate::studio::globe::SarOverlay;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2};
use pelagic::sar::{self, DriftSetup, Drivers, OBJECTS, Pattern, Report, STEP_H, Searcher, Spread};
use std::sync::Arc;

const KNOT: f64 = 0.514_444;

pub(super) struct Panel {
    setup: DriftSetup,
    /// The object entered the water this many hours ago; the search unit
    /// arrives this many hours from now.
    ago_h: f64,
    arrive_h: f64,
    searcher: Searcher,
    patterns: [bool; 4],
    /// Steady wind (m/s, from °) and current (m/s, towards °) instead of the
    /// live forecast.
    steady: bool,
    wind: [f64; 2],
    current: [f64; 2],
    /// Camera estimate: height (m), field of view (°), image width (px),
    /// pixels needed across the object.
    camera: [f64; 4],
    result: Option<Arc<Report>>,
    spreads: Vec<Spread>,
    chosen: usize,
    /// Map time (hours after the data's hour 0) and playback.
    hour: f64,
    playing: bool,
    show_all: bool,
    /// Map scale (metres per point) and centre; None fits the drift.
    view: Option<(f64, [f64; 2])>,
    overlay: Option<((usize, u64), Arc<SarOverlay>)>,
    /// Visible height of the results column (the map leaves room for the chart).
    room: f32,
    /// Fold the setup once after a result arrives, so the plans show.
    fold_setup: bool,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            setup: DriftSetup {
                latitude: 15.2,
                longitude: 73.2,
                ..Default::default()
            },
            ago_h: 2.0,
            arrive_h: 1.0,
            searcher: Searcher::default(),
            patterns: [true; 4],
            steady: false,
            wind: [8.0, 270.0],
            current: [0.3, 180.0],
            camera: [8.0, 60.0, 1920.0, 10.0],
            result: None,
            spreads: vec![],
            chosen: 0,
            hour: 0.0,
            playing: false,
            show_all: false,
            view: None,
            overlay: None,
            room: 600.0,
            fold_setup: false,
        }
    }
}

impl Panel {
    /// Sets the last known position (Earth view, scripts).
    pub(super) fn set_position(&mut self, latitude: f64, longitude: f64) {
        self.setup.latitude = latitude.clamp(-80.0, 80.0);
        self.setup.longitude = (longitude + 540.0).rem_euclid(360.0) - 180.0;
    }
    /// Script hook: steady wind and current (offline captures and checks).
    pub(super) fn set_steady(&mut self, wind: [f64; 2], current: [f64; 2]) {
        self.steady = true;
        self.wind = wind;
        self.current = current;
    }
    /// Script hook: move the map to `minutes` after the search starts.
    pub(super) fn show_search_minute(&mut self, minutes: f64) {
        if let Some(r) = &self.result {
            self.hour = (r.searcher.start_hour + minutes / 60.0)
                .clamp(r.drift.first_hour, r.drift.last_hour());
        }
    }

    /// Detection probability of particle `i` at `hour` for the chosen plan.
    fn detected(report: &Report, chosen: usize, hour: f64) -> Option<&Vec<f32>> {
        let o = report.options.get(chosen)?;
        if hour < o.plan.start_hour {
            return None;
        }
        let k = ((hour - o.plan.start_hour) / STEP_H).floor() as usize;
        o.detection.get(k.min(o.detection.len().saturating_sub(1)))
    }

    /// Particles, track, area and last known position for the Earth view.
    pub(super) fn overlay(&mut self) -> Option<Arc<SarOverlay>> {
        let report = self.result.clone()?;
        let key = (self.chosen, (self.hour * 60.0).round() as u64);
        if let Some((k, o)) = &self.overlay
            && *k == key
        {
            return Some(o.clone());
        }
        let d = &report.drift;
        let detected = Self::detected(&report, self.chosen, self.hour);
        let particles = d
            .positions(self.hour)
            .iter()
            .enumerate()
            .filter(|(_, (_, wet))| *wet)
            .map(|(i, (p, _))| {
                let (lat, lon) = d.plane.geo(*p);
                let left = 1.0 - detected.and_then(|v| v.get(i)).copied().unwrap_or(0.0);
                (lat, lon, left)
            })
            .collect();
        let mut track = vec![];
        let mut area = vec![];
        if let Some(o) = report.options.get(self.chosen) {
            for (_, p) in o.plan.schedule(report.searcher.speed_mps) {
                track.push(d.plane.geo(p));
            }
            let a = moved_area(o, self.hour);
            if a.round {
                for k in 0..=48 {
                    let t = k as f64 / 48.0 * 360.0;
                    let r = sar::towards(a.length_m * 0.5, t);
                    area.push(d.plane.geo([a.centre[0] + r[0], a.centre[1] + r[1]]));
                }
            } else {
                for c in a.corners().iter().chain(std::iter::once(&a.corners()[0])) {
                    area.push(d.plane.geo(*c));
                }
            }
        }
        let overlay = Arc::new(SarOverlay {
            lkp: (report.setup.latitude, report.setup.longitude),
            particles,
            track,
            area,
            colour: (0xd9, 0x59, 0x26),
        });
        self.overlay = Some((key, overlay.clone()));
        Some(overlay)
    }
}

/// The plan's area where it is at `hour` (it moves with the drift).
fn moved_area(o: &sar::Outcome, hour: f64) -> sar::Area {
    let mut a = o.plan.area;
    let t = (hour - o.plan.start_hour).clamp(0.0, o.duration_h) * 3600.0;
    a.centre[0] += o.plan.drift_velocity[0] * t;
    a.centre[1] += o.plan.drift_velocity[1] * t;
    a
}

/// "1:20" for hours.
fn clock(hours: f64) -> String {
    let minutes = (hours.abs() * 60.0).round() as i64;
    format!(
        "{}{}:{:02}",
        if hours < -1e-9 { "−" } else { "" },
        minutes / 60,
        minutes % 60
    )
}

fn nice(value: f64) -> f64 {
    let base = 10f64.powf(value.max(1e-6).log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * base)
        .find(|s| *s >= value)
        .unwrap_or(10.0 * base)
}

fn distance_text(m: f64) -> String {
    if m >= 1000.0 {
        format!("{:.1} km ({:.1} NM)", m / 1000.0, m / 1852.0)
    } else {
        format!("{m:.0} m")
    }
}

impl State {
    pub(super) fn sar_finished(&mut self, report: Report) {
        let best = report.options.first();
        self.log(format!(
            "Search plan ready: {} options for {}; best {} at {:.0} % probability of success.",
            report.options.len(),
            report.drift.object.name,
            best.map_or("—", |o| o.plan.pattern.name()),
            best.map_or(0.0, |o| o.pos * 100.0)
        ));
        for w in report.warnings.iter().chain(&report.drift.notes) {
            self.log(format!("Search and rescue: {w}"));
        }
        let p = &mut self.sar;
        p.spreads = report.spreads();
        p.hour = report.searcher.start_hour;
        p.chosen = 0;
        p.view = None;
        p.playing = false;
        p.overlay = None;
        p.fold_setup = true;
        p.result = Some(Arc::new(report));
    }

    pub(super) fn start_sar(&mut self) {
        let p = &self.sar;
        let mut setup = p.setup.clone();
        let mut searcher = p.searcher.clone();
        if let Some(v) = self.trial_subject() {
            searcher.name = v.name.clone();
        }
        let patterns: Vec<Pattern> = Pattern::ALL
            .iter()
            .zip(p.patterns)
            .filter(|(_, on)| *on)
            .map(|(p, _)| *p)
            .collect();
        if patterns.is_empty() {
            return self.log("Choose at least one search pattern.");
        }
        if let Err(e) = setup.validate().and_then(|()| searcher.validate()) {
            return self.log(e);
        }
        if p.arrive_h < -p.ago_h {
            return self.log("The search unit must arrive after the object entered the water.");
        }
        let (ago, arrive) = (p.ago_h, p.arrive_h);
        let steady = p
            .steady
            .then_some([p.wind[0], p.wind[1], p.current[0], p.current[1]]);
        let threads = pelagic::campaign::default_threads();
        self.sar.result = None;
        self.sar.overlay = None;
        self.start_job("Planning the search…", move |cancel, send| {
            let now = crate::forecast::now_minute();
            let drivers = match steady {
                Some([ws, wf, cs, ct]) => {
                    let start = pelagic::metocean::parse_utc(&now)
                        .map(|t| pelagic::metocean::utc_hour(t - 48 * 3600))
                        .unwrap_or_default();
                    Drivers::steady(&start, 120, setup.latitude, setup.longitude, ws, wf, cs, ct)
                }
                None => {
                    let _ = send.send(Event::Progress("Fetching wind and current…".into()));
                    Drivers::from_forecasts(&crate::forecast::drift_area(
                        setup.latitude,
                        setup.longitude,
                    )?)?
                }
            };
            let now_h = drivers
                .hour_of(&now)
                .ok_or("Could not place the current time in the forecast.")?;
            setup.incident_hour = now_h - ago;
            searcher.start_hour = now_h + arrive;
            let _ = send.send(Event::Progress("Drifting…".into()));
            let report = sar::run(
                &drivers,
                &setup,
                &searcher,
                &patterns,
                threads,
                &cancel,
                |done, total| {
                    let _ = send.send(Event::Progress(format!("Search plan {done}/{total}")));
                },
            )?;
            Ok(Output::Sar(Box::new(report)))
        });
    }

    fn export_sar(&mut self) {
        let Some(report) = self.sar.result.clone() else {
            return;
        };
        let Some(parent) = native::pick_folder(self.hwnd, "Choose a folder for the search plan")
        else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-search-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        match report.write(&folder, self.sar.chosen) {
            Ok(()) => self.log(format!(
                "Search plan saved (summary, drift, particles, GPX route, QGC mission): {}",
                folder.display()
            )),
            Err(e) => self.log(format!("Could not save the search plan: {e}")),
        }
    }

    pub(super) fn sar_panel(&mut self, ui: &mut Ui) {
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("sar-left")
                .show(left, |ui| self.sar_controls(ui));
            self.sar.room = right.available_height();
            egui::ScrollArea::vertical()
                .id_salt("sar-right")
                .show(right, |ui| self.sar_results(ui));
        });
    }

    fn sar_controls(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(4.0);
        theme::heading(ui, "Search and rescue");
        ui.label(
            RichText::new(
                "Where a person, raft or boat drifts from its last known position under the \
                 live wind and current, and which search plan gives the best chance of \
                 finding it.",
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
        ui.horizontal(|ui| {
            let run = ui
                .add_enabled(!busy, theme::primary("▶ Plan search"))
                .on_hover_text("Drifts thousands of possible objects and scores every search plan");
            crate::studio::a11y::name(&run, "Plan search");
            if run.clicked() {
                self.start_sar();
            }
            if self.sar.result.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new("💾 Export plan…"))
                    .on_hover_text(
                        "summary.txt, drift.csv, particles.csv, plans.json, plan.gpx and a QGC mission (plan.waypoints)",
                    )
                    .clicked()
            {
                self.export_sar();
            }
        });
        let earth = (self.globe.center(), self.scene().provenance.clone());
        let fold = std::mem::take(&mut self.sar.fold_setup).then_some(false);
        egui::CollapsingHeader::new(RichText::new("Incident").strong())
            .id_salt("sar-incident")
            .default_open(true)
            .open(fold)
            .show(ui, |ui| {
                let p = &mut self.sar;
                egui::Grid::new("sar-incident-grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Search object");
                        let current = sar::find_object(&p.setup.object).unwrap_or(&OBJECTS[0]);
                        let combo = egui::ComboBox::from_id_salt("sar-object")
                            .selected_text(current.name)
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for o in &OBJECTS {
                                    if ui
                                        .selectable_label(p.setup.object == o.id, o.name)
                                        .on_hover_text(format!(
                                            "{}: downwind {:.2} % of the wind {:+.1} cm/s, crosswind ±{:.2} %",
                                            o.id, o.downwind[0], o.downwind[1], o.right[0].abs().max(o.left[0].abs())
                                        ))
                                        .clicked()
                                    {
                                        p.setup.object = o.id.into();
                                    }
                                }
                            });
                        crate::studio::a11y::name(&combo.response, "Search object");
                        ui.end_row();
                        ui.label("Last known position");
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::DragValue::new(&mut p.setup.latitude)
                                    .range(-80.0..=80.0)
                                    .speed(0.001)
                                    .max_decimals(4)
                                    .suffix("° N"),
                            );
                            crate::studio::a11y::name(&r, "Last known latitude");
                            let r = ui.add(
                                egui::DragValue::new(&mut p.setup.longitude)
                                    .range(-180.0..=180.0)
                                    .speed(0.001)
                                    .max_decimals(4)
                                    .suffix("° E"),
                            );
                            crate::studio::a11y::name(&r, "Last known longitude");
                        });
                        ui.end_row();
                        ui.label("");
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .small_button("Earth view centre")
                                .on_hover_text("Or right-click the sea on the Earth tab → Search and rescue here")
                                .clicked()
                            {
                                p.setup.latitude = earth.0.0;
                                p.setup.longitude = earth.0.1;
                            }
                            let o = &earth.1;
                            if (o.origin_latitude != 0.0 || o.origin_longitude != 0.0)
                                && ui.small_button("Scene location").clicked()
                            {
                                p.setup.latitude = o.origin_latitude;
                                p.setup.longitude = o.origin_longitude;
                            }
                        });
                        ui.end_row();
                        ui.label("Position uncertainty");
                        let r = ui.add(
                            egui::DragValue::new(&mut p.setup.radius_m)
                                .range(0.0..=50_000.0)
                                .speed(10.0)
                                .suffix(" m (90 %)"),
                        );
                        crate::studio::a11y::name(&r, "Position uncertainty");
                        ui.end_row();
                        ui.label("In the water");
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::DragValue::new(&mut p.ago_h)
                                    .range(0.0..=46.0)
                                    .speed(0.05)
                                    .suffix(" h ago"),
                            );
                            crate::studio::a11y::name(&r, "Hours in the water");
                            let r = ui
                                .add(
                                    egui::DragValue::new(&mut p.setup.window_h)
                                        .range(0.0..=24.0)
                                        .speed(0.05)
                                        .prefix("within ")
                                        .suffix(" h after"),
                                )
                                .on_hover_text(
                                    "Uncertainty in the time: the object entered the water between then and this many hours later",
                                );
                            crate::studio::a11y::name(&r, "Incident window");
                        });
                        ui.end_row();
                        ui.label("Wind and current");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut p.steady, false, "Live forecast");
                            ui.radio_value(&mut p.steady, true, "Steady");
                        });
                        ui.end_row();
                        if p.steady {
                            ui.label("");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut p.wind[0])
                                        .range(0.0..=40.0)
                                        .speed(0.1)
                                        .prefix("wind ")
                                        .suffix(" m/s"),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut p.wind[1])
                                        .range(0.0..=359.0)
                                        .speed(1.0)
                                        .prefix("from ")
                                        .suffix("°"),
                                );
                            });
                            ui.end_row();
                            ui.label("");
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut p.current[0])
                                        .range(0.0..=4.0)
                                        .speed(0.01)
                                        .prefix("current ")
                                        .suffix(" m/s"),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut p.current[1])
                                        .range(0.0..=359.0)
                                        .speed(1.0)
                                        .prefix("towards ")
                                        .suffix("°"),
                                );
                            });
                            ui.end_row();
                        }
                    });
            });
        egui::CollapsingHeader::new(RichText::new("Search unit").strong())
            .id_salt("sar-unit")
            .default_open(true)
            .open(fold)
            .show(ui, |ui| {
                let p = &mut self.sar;
                egui::Grid::new("sar-unit-grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("On scene");
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::DragValue::new(&mut p.arrive_h)
                                    .range(-46.0..=48.0)
                                    .speed(0.05)
                                    .prefix("in ")
                                    .suffix(" h"),
                            );
                            crate::studio::a11y::name(&r, "Arrival on scene");
                            let r = ui.add(
                                egui::DragValue::new(&mut p.searcher.endurance_h)
                                    .range(0.25..=48.0)
                                    .speed(0.05)
                                    .prefix("for ")
                                    .suffix(" h"),
                            );
                            crate::studio::a11y::name(&r, "Time on scene");
                        });
                        ui.end_row();
                        ui.label("Search speed");
                        let mut kn = p.searcher.speed_mps / KNOT;
                        let r = ui.add(
                            egui::DragValue::new(&mut kn)
                                .range(0.5..=150.0)
                                .speed(0.1)
                                .suffix(" kn"),
                        );
                        crate::studio::a11y::name(&r, "Search speed");
                        p.searcher.speed_mps = kn * KNOT;
                        ui.end_row();
                        ui.label("Sweep width");
                        let r = ui
                            .add(
                                egui::DragValue::new(&mut p.searcher.sweep_width_m)
                                    .range(5.0..=20_000.0)
                                    .speed(5.0)
                                    .suffix(" m"),
                            )
                            .on_hover_text(
                                "Effective swept width for this object, sensor and weather. Take it from your SAR manual's tables or estimate it from a camera below.",
                            );
                        crate::studio::a11y::name(&r, "Sweep width");
                        ui.end_row();
                        ui.label("Detection");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut p.searcher.definite_range, false, "Inverse cube law")
                                .on_hover_text("Detection grows smoothly as the unit passes closer (Koopman); the usual planning model");
                            ui.radio_value(&mut p.searcher.definite_range, true, "Definite range")
                                .on_hover_text("Certain within W/2, nothing beyond: a sensor with a hard swath");
                        });
                        ui.end_row();
                        ui.label("Pattern");
                        ui.checkbox(&mut p.searcher.follow_drift, "moves with the drift")
                            .on_hover_text("Lay the pattern relative to the drifting datum (as when navigating relative to the water)");
                        ui.end_row();
                        ui.label("Try");
                        ui.horizontal_wrapped(|ui| {
                            for (i, pattern) in Pattern::ALL.iter().enumerate() {
                                ui.checkbox(&mut p.patterns[i], pattern.name());
                            }
                        });
                        ui.end_row();
                    });
                ui.label(
                    RichText::new(
                        "Expanding square and sector suit a small area around a precise datum; \
                         parallel track a large area; creeping line a long, narrow one.",
                    )
                    .small()
                    .color(theme::MUTED),
                );
                egui::CollapsingHeader::new("Estimate sweep width from a camera")
                    .id_salt("sar-camera")
                    .show(ui, |ui| {
                        let object = sar::find_object(&p.setup.object).unwrap_or(&OBJECTS[0]);
                        ui.horizontal_wrapped(|ui| {
                            for (i, (prefix, suffix, lo, hi, speed)) in [
                                ("height ", " m", 0.5, 3000.0, 0.5),
                                ("view ", "°", 1.0, 180.0, 0.5),
                                ("", " px wide", 64.0, 16_000.0, 10.0),
                                ("needs ", " px", 1.0, 100.0, 0.5),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                ui.add(
                                    egui::DragValue::new(&mut p.camera[i])
                                        .range(lo..=hi)
                                        .speed(speed)
                                        .prefix(prefix)
                                        .suffix(suffix),
                                );
                            }
                        });
                        let w = sar::camera_sweep_width(
                            object.size_m,
                            p.camera[0],
                            p.camera[1],
                            p.camera[2],
                            p.camera[3],
                        );
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{:.1} m object → about {w:.0} m (definite range, calm seas)",
                                    object.size_m
                                ))
                                .small(),
                            );
                            if ui.small_button("Use").clicked() {
                                p.searcher.sweep_width_m = w.max(5.0);
                                p.searcher.definite_range = true;
                            }
                        });
                        ui.label(
                            RichText::new(
                                "Pixels on target and the horizon only: whitecaps, glare, haze and \
                                 waves hiding small objects reduce it further.",
                            )
                            .small()
                            .color(theme::MUTED),
                        );
                    });
            });
        let Some(report) = self.sar.result.clone() else {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Set the last known position and press Plan search. The live forecast \
                     covers the past two days and the next three.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        ui.add_space(6.0);
        theme::heading(ui, "Search plans");
        ui.label(
            RichText::new("Best first. Click a plan to show it on the map.")
                .small()
                .color(theme::MUTED),
        );
        let shown = if self.sar.show_all {
            report.options.len()
        } else {
            report.options.len().min(8)
        };
        egui::Grid::new("sar-options")
            .num_columns(6)
            .striped(true)
            .spacing([8.0, 3.0])
            .show(ui, |ui| {
                for h in ["Plan", "Spacing", "Area", "C", "POC", "POS"] {
                    ui.label(RichText::new(h).small().color(theme::MUTED));
                }
                ui.end_row();
                for (i, o) in report.options.iter().enumerate().take(shown) {
                    let r = ui.selectable_label(self.sar.chosen == i, o.plan.pattern.name());
                    let tip = format!(
                        "{}: spacing {:.0} m, {:.1} km², coverage {:.2}, POC {:.0} %, POD {:.0} %, POS {:.0} %",
                        o.plan.pattern.name(),
                        o.plan.spacing_m,
                        o.plan.area.size_m2() / 1e6,
                        o.coverage,
                        o.poc * 100.0,
                        o.pod * 100.0,
                        o.pos * 100.0
                    );
                    crate::studio::a11y::name(&r, &tip);
                    if r.on_hover_text(tip).clicked() {
                        self.sar.chosen = i;
                    }
                    let small = |ui: &mut Ui, t: String| ui.label(RichText::new(t).small());
                    small(
                        ui,
                        if o.plan.pattern == Pattern::Sector {
                            format!("R {:.0} m", o.plan.spacing_m)
                        } else {
                            format!("{:.0} m", o.plan.spacing_m)
                        },
                    );
                    small(ui, format!("{:.1} km²", o.plan.area.size_m2() / 1e6));
                    small(ui, format!("{:.2}", o.coverage));
                    small(ui, format!("{:.0} %", o.poc * 100.0));
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(54.0, 10.0), Sense::hover());
                        ui.painter().rect_filled(rect, 3.0, theme::BORDER);
                        let mut bar = rect;
                        bar.set_width(rect.width() * o.pos as f32);
                        ui.painter().rect_filled(bar, 3.0, SERIES[0]);
                        ui.label(RichText::new(format!("{:.0} %", o.pos * 100.0)).small().strong());
                    });
                    ui.end_row();
                }
            });
        if report.options.len() > 8 {
            let label = if self.sar.show_all {
                "Show the best 8"
            } else {
                "Show all plans"
            };
            if ui.small_button(label).clicked() {
                self.sar.show_all = !self.sar.show_all;
            }
        }
        ui.label(
            RichText::new(
                "C coverage (sweep width × track ÷ area) · POC chance the object is in the area · \
                 POS chance this search finds it (includes detections outside the area).",
            )
            .small()
            .color(theme::MUTED),
        );
    }

    fn sar_results(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let Some(report) = self.sar.result.clone() else {
            theme::heading(ui, "Drift and search map");
            ui.label(
                RichText::new(
                    "The map shows the possible positions of the object (each dot one \
                     hypothesis), the datum track, the search area and the planned track.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        let d = &report.drift;
        let chosen = self.sar.chosen.min(report.options.len().saturating_sub(1));
        let o = report.options.get(chosen);
        ui.horizontal_wrapped(|ui| {
            if let Some(o) = o {
                theme::chip(
                    ui,
                    if o.pos >= 0.5 {
                        theme::GOOD
                    } else {
                        theme::WARN
                    },
                    &format!("{} · POS {:.0} %", o.plan.pattern.name(), o.pos * 100.0),
                );
            }
            let s = d.spread(report.searcher.start_hour);
            let (lat, lon) = d.plane.geo(s.datum);
            theme::chip(
                ui,
                theme::MUTED,
                &format!(
                    "Datum at search start {lat:.4}°, {lon:.4}° · {} towards {:.0}°",
                    distance_text(s.datum[0].hypot(s.datum[1])),
                    sar::bearing(s.datum)
                ),
            );
        });
        self.sar_map(ui, &report);
        // Time slider and playback.
        let (first, last) = (d.first_hour, d.last_hour());
        let start = report.searcher.start_hour;
        ui.horizontal(|ui| {
            let play = if self.sar.playing { "⏸" } else { "▶" };
            let b = ui
                .button(play)
                .on_hover_text("Play the drift and the search");
            crate::studio::a11y::name(&b, "Play drift");
            if b.clicked() {
                if !self.sar.playing && self.sar.hour >= last - 1e-6 {
                    self.sar.hour = first;
                }
                self.sar.playing = !self.sar.playing;
            }
            ui.spacing_mut().slider_width = (ui.available_width() - 210.0).max(80.0);
            let r = ui.add(egui::Slider::new(&mut self.sar.hour, first..=last).show_value(false));
            crate::studio::a11y::name(&r, "Drift time");
            if r.dragged() {
                self.sar.playing = false;
            }
            let rel = self.sar.hour - start;
            ui.label(
                RichText::new(format!(
                    "{} UTC · {}",
                    d.time_of(self.sar.hour).replace('T', " "),
                    if rel >= 0.0 {
                        format!("search +{}", clock(rel))
                    } else {
                        format!("{} before search", clock(-rel))
                    }
                ))
                .small(),
            );
        });
        if self.sar.playing {
            let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
            self.sar.hour += dt * (last - first) / 20.0;
            if self.sar.hour >= last {
                self.sar.hour = last;
                self.sar.playing = false;
            }
            ui.ctx().request_repaint();
        }
        if let Some(o) = o {
            chart(
                ui,
                "Probability of success during the search (%)",
                &[Series {
                    name: o.plan.pattern.name(),
                    colour: SERIES[0],
                    points: o.pos_curve.iter().map(|p| [p[0], p[1] * 100.0]).collect(),
                }],
                "hours on scene",
                "POS %",
                Options {
                    origin: true,
                    room: 230.0,
                    marker_x: Some(self.sar.hour - start),
                    ..Default::default()
                },
            );
            ui.label(
                RichText::new(format!(
                    "{} · track {} in {:.1} h · POC {:.0} % · POD {:.0} % · {} hypotheses",
                    report.searcher.name,
                    distance_text(o.track_m),
                    o.duration_h,
                    o.poc * 100.0,
                    o.pod * 100.0,
                    d.particles()
                ))
                .small()
                .color(theme::MUTED),
            );
        }
        for w in report.warnings.iter().chain(&d.notes) {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("⚠").color(theme::WARN));
                ui.label(RichText::new(w).small());
            });
        }
        ui.label(
            RichText::new(format!(
                "Leeway: {} ({}), Allen & Plourde 1999 / Allen 2005. Wind and current: {}.",
                d.object.name, d.object.id, d.source
            ))
            .small()
            .color(theme::MUTED),
        );
        if d.source.contains("Open-Meteo") {
            ui.label(
                RichText::new(crate::forecast::ATTRIBUTION)
                    .small()
                    .color(theme::MUTED),
            );
        }
        ui.label(
            RichText::new(
                "Planning aid: confirm plans with your rescue coordination centre's tools.",
            )
            .small()
            .color(theme::MUTED),
        );
    }

    fn sar_map(&mut self, ui: &mut Ui, report: &Report) {
        let width = ui.available_width().max(240.0);
        // Leave room below for the time slider and the POS chart.
        let height = (width * 0.72)
            .clamp(260.0, 560.0)
            .min((self.sar.room - 340.0).max(260.0));
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        crate::studio::a11y::name(&response, "Drift and search map");
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, SURFACE);
        let d = &report.drift;
        let chosen = self.sar.chosen.min(report.options.len().saturating_sub(1));
        let outcome = report.options.get(chosen);
        let speed = report.searcher.speed_mps;
        let hour = self.sar.hour;
        let margin = 24.0;
        let fit = || {
            // 1st–99th percentile of the final cloud, the LKP and the track.
            let mut xs = vec![0.0];
            let mut ys = vec![0.0];
            let mut cloud: Vec<[f64; 2]> = d
                .positions(d.last_hour())
                .into_iter()
                .filter(|(_, w)| *w)
                .map(|(p, _)| p)
                .collect();
            for p in &self.sar.spreads {
                cloud.push(p.datum);
            }
            let mut cx: Vec<f64> = cloud.iter().map(|p| p[0]).collect();
            let mut cy: Vec<f64> = cloud.iter().map(|p| p[1]).collect();
            cx.sort_by(f64::total_cmp);
            cy.sort_by(f64::total_cmp);
            let q = |v: &[f64], f: f64| v[((v.len() - 1) as f64 * f) as usize];
            if !cx.is_empty() {
                xs.extend([q(&cx, 0.02), q(&cx, 0.98)]);
                ys.extend([q(&cy, 0.02), q(&cy, 0.98)]);
            }
            if let Some(o) = outcome {
                for (_, p) in o.plan.schedule(speed) {
                    xs.push(p[0]);
                    ys.push(p[1]);
                }
            }
            let (x0, x1) = xs
                .iter()
                .fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
            let (y0, y1) = ys
                .iter()
                .fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
            let scale = ((x1 - x0) / (rect.width() as f64 - 2.0 * margin))
                .max((y1 - y0) / (rect.height() as f64 - 2.0 * margin))
                .max(0.5);
            (scale * 1.05, [(x0 + x1) * 0.5, (y0 + y1) * 0.5])
        };
        if self.sar.view.is_none() || response.double_clicked() {
            self.sar.view = Some(fit());
        }
        let (mut scale, mut centre) = self.sar.view.unwrap_or((1.0, [0.0, 0.0]));
        if response.dragged() {
            let delta = response.drag_delta();
            centre[0] -= delta.x as f64 * scale;
            centre[1] += delta.y as f64 * scale;
        }
        if let Some(pointer) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let before = [
                    centre[0] + (pointer.x - rect.center().x) as f64 * scale,
                    centre[1] - (pointer.y - rect.center().y) as f64 * scale,
                ];
                scale = (scale * (-scroll as f64 * 0.002).exp()).clamp(0.05, 50_000.0);
                centre = [
                    before[0] - (pointer.x - rect.center().x) as f64 * scale,
                    before[1] + (pointer.y - rect.center().y) as f64 * scale,
                ];
            }
        }
        self.sar.view = Some((scale, centre));
        let c = rect.center();
        let screen = |p: [f64; 2]| {
            Pos2::new(
                c.x + ((p[0] - centre[0]) / scale) as f32,
                c.y - ((p[1] - centre[1]) / scale) as f32,
            )
        };
        // Recessive grid.
        let step = nice(scale * 90.0);
        let grid = Color32::from_rgba_unmultiplied(124, 140, 153, 28);
        let (wx0, wx1) = (
            centre[0] - rect.width() as f64 * 0.5 * scale,
            centre[0] + rect.width() as f64 * 0.5 * scale,
        );
        let (wy0, wy1) = (
            centre[1] - rect.height() as f64 * 0.5 * scale,
            centre[1] + rect.height() as f64 * 0.5 * scale,
        );
        let mut x = (wx0 / step).ceil() * step;
        while x <= wx1 {
            let sx = screen([x, 0.0]).x;
            painter.line_segment(
                [Pos2::new(sx, rect.top()), Pos2::new(sx, rect.bottom())],
                Stroke::new(1.0, grid),
            );
            x += step;
        }
        let mut y = (wy0 / step).ceil() * step;
        while y <= wy1 {
            let sy = screen([0.0, y]).y;
            painter.line_segment(
                [Pos2::new(rect.left(), sy), Pos2::new(rect.right(), sy)],
                Stroke::new(1.0, grid),
            );
            y += step;
        }
        // Possible positions, fading as the search clears them.
        let detected = Panel::detected(report, chosen, hour);
        for (i, (p, wet)) in d.positions(hour).iter().enumerate() {
            if !wet {
                continue;
            }
            let q = screen(*p);
            if !rect.contains(q) {
                continue;
            }
            let left = 1.0 - detected.and_then(|v| v.get(i)).copied().unwrap_or(0.0);
            let a = (25.0 + 175.0 * left) as u8;
            painter.rect_filled(
                Rect::from_center_size(q, Vec2::splat(2.2)),
                0.0,
                Color32::from_rgba_unmultiplied(0xd9, 0x59, 0x26, a),
            );
        }
        // Datum track (hourly) from the last known position.
        let datum: Vec<Pos2> = std::iter::once(screen([0.0, 0.0]))
            .chain(self.sar.spreads.iter().map(|s| screen(s.datum)))
            .collect();
        painter.add(Shape::line(
            datum.clone(),
            Stroke::new(1.5, Color32::from_rgba_unmultiplied(230, 236, 242, 150)),
        ));
        for p in &datum[1..] {
            painter.circle_filled(*p, 2.0, Color32::from_rgba_unmultiplied(230, 236, 242, 170));
        }
        let now = d.spread(hour);
        if now.released > 0 {
            painter.circle_stroke(screen(now.datum), 5.0, Stroke::new(1.5, theme::STRONG));
        }
        // Last known position.
        let l = screen([0.0, 0.0]);
        painter.line_segment(
            [l - Vec2::new(6.0, 6.0), l + Vec2::new(6.0, 6.0)],
            Stroke::new(2.0, theme::STRONG),
        );
        painter.line_segment(
            [l - Vec2::new(6.0, -6.0), l + Vec2::new(6.0, -6.0)],
            Stroke::new(2.0, theme::STRONG),
        );
        painter.text(
            l + Vec2::new(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            "LKP",
            FontId::proportional(11.0),
            theme::STRONG,
        );
        if let Some(o) = outcome {
            // Search area where it is now.
            let a = moved_area(o, hour);
            let area_stroke = Stroke::new(1.5, SERIES[2]);
            if a.round {
                painter.circle_stroke(
                    screen(a.centre),
                    (a.length_m * 0.5 / scale) as f32,
                    area_stroke,
                );
            } else {
                let mut corners: Vec<Pos2> = a.corners().iter().map(|p| screen(*p)).collect();
                corners.push(corners[0]);
                painter.extend(Shape::dashed_line(&corners, area_stroke, 6.0, 4.0));
            }
            // Ground track: flown so far (solid), still to fly (faint).
            let seconds = o.duration_h * 3600.0;
            let elapsed = ((hour - o.plan.start_hour) * 3600.0).clamp(0.0, seconds);
            let samples = 400usize;
            let at = |t: f64| screen(o.plan.at(t * speed, t));
            let done: Vec<Pos2> = (0..=samples)
                .map(|k| at(elapsed * k as f64 / samples as f64))
                .collect();
            let todo: Vec<Pos2> = (0..=samples)
                .map(|k| at(elapsed + (seconds - elapsed) * k as f64 / samples as f64))
                .collect();
            painter.add(Shape::line(
                todo,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x39, 0x87, 0xe5, 110)),
            ));
            if elapsed > 0.0 {
                painter.add(Shape::line(done, Stroke::new(2.0, SERIES[0])));
                let me = at(elapsed);
                painter.circle_filled(me, 5.0, SERIES[0]);
                painter.circle_stroke(me, 5.0, Stroke::new(1.5, theme::STRONG));
            }
        }
        // Legend, north arrow, scale bar.
        let legend = [
            (
                Color32::from_rgb(0xd9, 0x59, 0x26),
                "possible positions (fade when searched)",
            ),
            (SERIES[0], "search track"),
            (SERIES[2], "search area"),
            (
                Color32::from_rgb(230, 236, 242),
                "datum track (hourly), LKP ✕",
            ),
        ];
        painter.rect_filled(
            Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::new(250.0, 66.0)),
            4.0,
            Color32::from_rgba_unmultiplied(12, 18, 24, 215),
        );
        let mut y = rect.top() + 10.0;
        for (colour, text) in legend {
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(rect.left() + 10.0, y + 3.0), Vec2::new(10.0, 4.0)),
                1.0,
                colour,
            );
            painter.text(
                Pos2::new(rect.left() + 26.0, y),
                Align2::LEFT_TOP,
                text,
                FontId::proportional(10.5),
                theme::TEXT,
            );
            y += 15.0;
        }
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
        let bar = nice(scale * 110.0);
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
        if let Some(pointer) = response.hover_pos() {
            let w = [
                centre[0] + (pointer.x - c.x) as f64 * scale,
                centre[1] - (pointer.y - c.y) as f64 * scale,
            ];
            let (lat, lon) = d.plane.geo(w);
            painter.text(
                Pos2::new(rect.left() + 10.0, rect.bottom() - 10.0),
                Align2::LEFT_BOTTOM,
                format!(
                    "{lat:.4}°, {lon:.4}° · {} at {:.0}° from the LKP",
                    distance_text(w[0].hypot(w[1])),
                    sar::bearing(w)
                ),
                FontId::proportional(10.5),
                theme::TEXT,
            );
        } else {
            painter.text(
                Pos2::new(rect.left() + 10.0, rect.bottom() - 10.0),
                Align2::LEFT_BOTTOM,
                "drag to pan · scroll to zoom · double-click to fit",
                FontId::proportional(10.5),
                theme::MUTED,
            );
        }
    }
}
