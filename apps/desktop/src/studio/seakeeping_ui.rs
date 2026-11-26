//! Seakeeping in the editor: the selected vehicle's motions in irregular
//! seas, the largest sea it can work in against motion criteria, and a
//! go/no-go outlook from the live forecast for a planned course.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use crate::studio::charts::{Options, SERIES, Series, chart};
use egui::{Color32, RichText, Sense, Ui, Vec2};
use pelagic::seakeeping::{self, Criteria, Report, Setup};
use std::sync::Arc;

pub(super) struct Panel {
    setup: Setup,
    full_grid: bool,
    preset: usize,
    criteria: Criteria,
    result: Option<Arc<Report>>,
    /// Planned course for the outlook, compass degrees.
    course: f64,
    working_point: bool,
    point: [f64; 3],
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            setup: Setup::quick(),
            full_grid: false,
            preset: 0,
            criteria: Criteria::presets().remove(0),
            result: None,
            course: 0.0,
            working_point: false,
            point: [0.0, 0.0, 0.0],
        }
    }
}

/// Darker → lighter blue with the limiting height (higher = more operable).
fn limit_colour(hs: f64) -> Color32 {
    let t = (hs / 4.0).clamp(0.0, 1.0) as f32;
    let a = [16.0, 66.0, 129.0];
    let b = [57.0, 135.0, 229.0];
    let c: Vec<u8> = (0..3).map(|i| (a[i] + (b[i] - a[i]) * t) as u8).collect();
    Color32::from_rgb(c[0], c[1], c[2])
}

impl State {
    /// Sets the current scene's sea, wind, current and sea temperature
    /// from forecast conditions (undoable).
    pub(super) fn apply_forecast_sea(
        &mut self,
        conditions: &pelagic::metocean::Conditions,
        source: &str,
    ) {
        let mut next = self.scene().clone();
        let notes = pelagic::metocean::apply(&mut next, conditions, source);
        let summary = format!(
            "Applied forecast sea ({} UTC): wind sea {:.2} m / {:.1} s, swell {:.2} m / {:.1} s, wind {:.1} m/s.",
            conditions.time.replace('T', " "),
            next.wave_height,
            next.wave_period,
            next.swell_height,
            next.swell_period,
            next.wind_speed
        );
        if self.apply(next, &summary) {
            self.log(summary);
            for n in notes {
                self.log(n);
            }
        }
    }

    pub(super) fn seakeeping_finished(&mut self, report: Report) {
        self.log(format!(
            "Seakeeping table ready for {} ({} sea states).",
            report.vehicle.name,
            report.cells.len()
        ));
        for w in &report.warnings {
            self.log(format!("Seakeeping warning: {w}"));
        }
        self.seakeeping.result = Some(Arc::new(report));
    }

    pub(super) fn start_seakeeping(&mut self) {
        let Some(vehicle) = self.trial_subject() else {
            return self.log("Select a vehicle in the Outliner to compute its seakeeping.");
        };
        let p = &self.seakeeping;
        let mut setup = if p.full_grid {
            Setup {
                speed_fraction: p.setup.speed_fraction,
                reference_height: p.setup.reference_height,
                operating_depth: p.setup.operating_depth,
                ..Setup::default()
            }
        } else {
            p.setup.clone()
        };
        setup.working_point = p
            .working_point
            .then(|| pelagic::math::Vec3::new(p.point[0], p.point[1], p.point[2]));
        if let Err(e) = setup.validate() {
            return self.log(e);
        }
        let environment = pelagic::trials::Environment {
            density: self.scene().density,
            seed: self.scene().seed,
            ..Default::default()
        };
        let threads = pelagic::campaign::default_threads();
        self.seakeeping.result = None;
        self.start_job(
            &format!("Seakeeping for {}…", vehicle.name),
            move |cancel, send| {
                let report = seakeeping::run(
                    &vehicle,
                    &environment,
                    &setup,
                    threads,
                    &cancel,
                    |done, total| {
                        let _ = send.send(Event::Progress(format!("Sea state {done}/{total}")));
                    },
                )?;
                Ok(Output::Seakeeping(Box::new(report)))
            },
        );
    }

    fn export_seakeeping(&mut self) {
        let Some(report) = self.seakeeping.result.clone() else {
            return;
        };
        let Some(parent) =
            native::pick_folder(self.hwnd, "Choose a folder for the seakeeping report")
        else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-seakeeping-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        let criteria = self.seakeeping.criteria.clone();
        let mut result = report.write(&folder, &criteria);
        if result.is_ok()
            && let Some(f) = self.globe.forecast()
        {
            let outlook = report.outlook(&f, self.seakeeping.course, &criteria);
            let mut csv = String::from(
                "time_utc,wave_height_m,operable,limited_by,roll_deg,pitch_deg,point_vertical_g,point_lateral_g,slam_probability,wetness_probability\n",
            );
            for h in &outlook {
                let m = h.motions.unwrap_or_default();
                csv += &format!(
                    "{},{},{},\"{}\",{:.3},{:.3},{:.4},{:.4},{:.4},{:.4}\n",
                    h.time,
                    h.wave_height.map_or(String::new(), |v| format!("{v:.2}")),
                    h.operable.map_or(String::new(), |v| v.to_string()),
                    h.limited_by.join("; "),
                    m.roll_deg,
                    m.pitch_deg,
                    m.point_vertical_g,
                    m.point_lateral_g,
                    m.slam_probability,
                    m.wetness_probability
                );
            }
            result = std::fs::write(folder.join("outlook.csv"), csv).map_err(|e| e.to_string());
        }
        match result {
            Ok(()) => self.log(format!("Seakeeping report saved: {}", folder.display())),
            Err(e) => self.log(format!("Could not save the seakeeping report: {e}")),
        }
    }

    pub(super) fn seakeeping_panel(&mut self, ui: &mut Ui) {
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("seakeeping-left")
                .show(left, |ui| self.seakeeping_controls(ui));
            egui::ScrollArea::vertical()
                .id_salt("seakeeping-right")
                .show(right, |ui| self.seakeeping_outlook(ui));
        });
    }

    fn seakeeping_controls(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(4.0);
        theme::heading(ui, "Seakeeping and operability");
        ui.label(
            RichText::new(
                "Motions in irregular seas for each wave period and heading, the largest sea \
                 the vehicle can work in, and a go/no-go outlook from the live forecast.",
            )
            .small()
            .color(theme::MUTED),
        );
        let subject = self.trial_subject();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Vehicle").color(theme::MUTED));
            match &subject {
                Some(e) => ui.label(RichText::new(&e.name).strong().color(theme::STRONG)),
                None => ui.label(RichText::new("select one in the Outliner").color(theme::WARN)),
            };
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
                .add_enabled(
                    !busy && subject.is_some(),
                    theme::primary("▶ Compute seakeeping"),
                )
                .on_hover_text("Runs every sea state of the grid on all CPU cores but one");
            crate::studio::a11y::name(&run, "Compute seakeeping");
            if run.clicked() {
                self.start_seakeeping();
            }
            if self.seakeeping.result.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new("💾 Export report…"))
                    .on_hover_text("seakeeping.json, responses.csv, summary.txt and the outlook")
                    .clicked()
            {
                self.export_seakeeping();
            }
        });
        egui::CollapsingHeader::new(RichText::new("Setup and criteria").strong())
            .id_salt("seakeeping-setup")
            .default_open(true)
            .show(ui, |ui| {
                let p = &mut self.seakeeping;
                egui::Grid::new("seakeeping-setup-grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Transit thrust");
                        let mut pct = p.setup.speed_fraction * 100.0;
                        let r = ui.add(
                            egui::DragValue::new(&mut pct)
                                .range(0.0..=100.0)
                                .speed(1.0)
                                .suffix(" %"),
                        );
                        crate::studio::a11y::name(&r, "Transit thrust");
                        p.setup.speed_fraction = pct / 100.0;
                        ui.end_row();
                        ui.label("Sea states");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut p.full_grid, false, "Quick (3×3)");
                            ui.radio_value(&mut p.full_grid, true, "Full (9 periods × 7 headings)");
                        });
                        ui.end_row();
                        ui.label("Working point");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut p.working_point, "")
                                .on_hover_text("Bridge, crane tip or sensor, body frame (m). Off: on deck 20 % forward of amidships.");
                            if p.working_point {
                                for (i, axis) in ["x", "y", "z"].iter().enumerate() {
                                    let r = ui.add(
                                        egui::DragValue::new(&mut p.point[i])
                                            .speed(0.05)
                                            .range(-50.0..=50.0)
                                            .prefix(format!("{axis} "))
                                            .suffix(" m"),
                                    );
                                    crate::studio::a11y::name(&r, &format!("Working point {axis}"));
                                }
                            }
                        });
                        ui.end_row();
                        ui.label("Criteria");
                        let presets = Criteria::presets();
                        let combo = egui::ComboBox::from_id_salt("seakeeping-criteria")
                            .selected_text(&presets[p.preset.min(presets.len() - 1)].name)
                            .width(250.0)
                            .show_ui(ui, |ui| {
                                for (i, c) in presets.iter().enumerate() {
                                    if ui.selectable_label(p.preset == i, &c.name).clicked() {
                                        p.preset = i;
                                        p.criteria = c.clone();
                                    }
                                }
                            });
                        crate::studio::a11y::name(&combo.response, "Motion criteria");
                        ui.end_row();
                        let c = &mut p.criteria;
                        let limit = |ui: &mut Ui, label: &str, v: &mut Option<f64>, unit: &str, speed: f64| {
                            let mut on = v.is_some();
                            ui.checkbox(&mut on, label);
                            let mut x = v.unwrap_or(0.1);
                            let r = ui.add_enabled(
                                on,
                                egui::DragValue::new(&mut x)
                                    .range(0.0..=90.0)
                                    .speed(speed)
                                    .suffix(unit),
                            );
                            crate::studio::a11y::name(&r, &format!("{label} limit"));
                            *v = on.then_some(x);
                            ui.end_row();
                        };
                        limit(ui, "Roll RMS", &mut c.max_roll_deg, "°", 0.1);
                        limit(ui, "Pitch RMS", &mut c.max_pitch_deg, "°", 0.1);
                        limit(ui, "Bow vertical acc.", &mut c.max_bow_vertical_g, " g", 0.005);
                        limit(ui, "Point vertical acc.", &mut c.max_point_vertical_g, " g", 0.005);
                        limit(ui, "Point lateral acc.", &mut c.max_point_lateral_g, " g", 0.005);
                        limit(ui, "Slamming probability", &mut c.max_slam_probability, "", 0.001);
                        limit(ui, "Deck wetness probability", &mut c.max_wetness_probability, "", 0.001);
                    });
            });
        let Some(report) = self.seakeeping.result.clone() else {
            return;
        };
        ui.add_space(6.0);
        theme::heading(ui, "Limiting significant wave height (m)");
        ui.label(
            RichText::new(format!(
                "{} · {} at {:.1} m/s · rows: peak period, columns: relative heading (180° head seas)",
                report.vehicle.name,
                report.mode,
                report.cells.first().map_or(0.0, |c| c.speed_mps)
            ))
            .small()
            .color(theme::MUTED),
        );
        let criteria = self.seakeeping.criteria.clone();
        let mut headings: Vec<f64> = report.cells.iter().map(|c| c.heading_deg).collect();
        headings.sort_by(f64::total_cmp);
        headings.dedup();
        let mut periods: Vec<f64> = report.cells.iter().map(|c| c.period_s).collect();
        periods.sort_by(f64::total_cmp);
        periods.dedup();
        egui::Grid::new("seakeeping-limits")
            .num_columns(headings.len() + 1)
            .spacing([3.0, 3.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Tp \\ μ").small().color(theme::MUTED));
                for h in &headings {
                    ui.label(
                        RichText::new(format!("{h:.0}°"))
                            .small()
                            .color(theme::MUTED),
                    );
                }
                ui.end_row();
                for t in &periods {
                    ui.label(
                        RichText::new(format!("{t:.0} s"))
                            .small()
                            .color(theme::MUTED),
                    );
                    for h in &headings {
                        let Some(cell) = report
                            .cells
                            .iter()
                            .find(|c| c.period_s == *t && c.heading_deg == *h)
                        else {
                            ui.label("—");
                            continue;
                        };
                        let (hs, by) = report.limiting_height(cell, &criteria);
                        let (rect, response) =
                            ui.allocate_exact_size(Vec2::new(46.0, 20.0), Sense::hover());
                        // Beyond 20 m no real sea limits the vehicle.
                        let text = if hs <= 20.0 {
                            format!("{hs:.1}")
                        } else {
                            "> 20".into()
                        };
                        ui.painter().rect_filled(rect, 3.0, limit_colour(hs));
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &text,
                            egui::FontId::proportional(11.5),
                            theme::STRONG,
                        );
                        let tip = format!(
                            "Tp {t:.0} s, heading {h:.0}°: limiting Hs {text} m ({by})\n\
                             per metre Hs: roll {:.2}°, pitch {:.2}°, heave {:.2} m, \
                             point vertical {:.3} g, lateral {:.3} g",
                            cell.roll_deg,
                            cell.pitch_deg,
                            cell.heave_m,
                            cell.point_vertical_g,
                            cell.point_lateral_g
                        );
                        crate::studio::a11y::name(&response, &tip);
                        response.on_hover_text(tip);
                    }
                    ui.end_row();
                }
            });
        for w in &report.warnings {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("⚠").color(theme::WARN));
                ui.label(RichText::new(w).small());
            });
        }
    }

    fn seakeeping_outlook(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        theme::heading(ui, "Operability outlook");
        let forecast = self.globe.forecast();
        let origin = {
            let p = &self.scene().provenance;
            (p.origin_latitude != 0.0 || p.origin_longitude != 0.0)
                .then_some((p.origin_latitude, p.origin_longitude))
        };
        ui.horizontal_wrapped(|ui| {
            if let Some((lat, lon)) = origin
                && ui
                    .button("Forecast at this scene's location")
                    .on_hover_text("Loads the 7-day sea forecast for the scene's real-world origin")
                    .clicked()
            {
                self.globe.request_forecast(lat, lon);
            }
            if ui.button("Pick a place on Earth").clicked() {
                self.globe.focus = true;
            }
        });
        let Some(f) = forecast else {
            ui.label(
                RichText::new(
                    "Load a forecast: on the Earth tab right-click the sea and choose \
                     \"Sea forecast here\", or use the button above for a real-world scene.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        ui.label(
            RichText::new(format!(
                "Forecast at {:.3}°, {:.3}° · {} hours",
                f.latitude,
                f.longitude,
                f.hours.len()
            ))
            .small()
            .color(theme::MUTED),
        );
        ui.horizontal(|ui| {
            ui.label("Planned course");
            let r = ui.add(
                egui::DragValue::new(&mut self.seakeeping.course)
                    .range(0.0..=359.0)
                    .speed(1.0)
                    .suffix("° true"),
            );
            crate::studio::a11y::name(&r, "Planned course");
        });
        let Some(report) = self.seakeeping.result.clone() else {
            ui.label(
                RichText::new("Compute seakeeping for the vehicle to judge the forecast.")
                    .small()
                    .color(theme::MUTED),
            );
            return;
        };
        let outlook = report.outlook(&f, self.seakeeping.course, &self.seakeeping.criteria);
        let judged: Vec<_> = outlook.iter().filter(|h| h.operable.is_some()).collect();
        let good = judged.iter().filter(|h| h.operable == Some(true)).count();
        if judged.is_empty() {
            ui.label(RichText::new("The forecast has no usable sea state.").color(theme::WARN));
            return;
        }
        let share = good as f64 / judged.len() as f64 * 100.0;
        ui.horizontal_wrapped(|ui| {
            theme::chip(
                ui,
                if share >= 50.0 {
                    theme::GOOD
                } else {
                    theme::BAD
                },
                &format!("Operable {good} of {} hours ({share:.0} %)", judged.len()),
            );
            let now = crate::forecast::now_hour();
            if let Some(h) = outlook.iter().find(|h| h.time >= now) {
                let (c, text) = match h.operable {
                    Some(true) => (theme::GOOD, "✔ Go now".to_string()),
                    Some(false) => (
                        theme::BAD,
                        format!("✖ No-go now: {}", h.limited_by.join(", ")),
                    ),
                    None => (theme::MUTED, "No data now".to_string()),
                };
                theme::chip(ui, c, &text);
            }
        });
        // Hour strip: one segment per forecast hour.
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
        let n = outlook.len().max(1) as f32;
        for (i, h) in outlook.iter().enumerate() {
            let x0 = rect.left() + rect.width() * i as f32 / n;
            let x1 = rect.left() + rect.width() * (i + 1) as f32 / n;
            let colour = match h.operable {
                Some(true) => theme::GOOD,
                Some(false) => theme::BAD,
                None => theme::BORDER,
            };
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x0, rect.top()),
                    egui::pos2(x1 - 0.5, rect.bottom()),
                ),
                0.0,
                colour,
            );
        }
        if let Some(p) = response.hover_pos() {
            let i = (((p.x - rect.left()) / rect.width()) * n) as usize;
            if let Some(h) = outlook.get(i.min(outlook.len() - 1)) {
                let m = h.motions.unwrap_or_default();
                response.on_hover_text(format!(
                    "{} UTC · Hs {} m · {}\nroll {:.1}° · pitch {:.1}° · vertical {:.3} g · lateral {:.3} g · slam {:.3} · wetness {:.3}",
                    h.time.replace('T', " "),
                    h.wave_height.map_or("—".into(), |v| format!("{v:.2}")),
                    match h.operable {
                        Some(true) => "operable".to_string(),
                        Some(false) => format!("limited by {}", h.limited_by.join(", ")),
                        None => "no data".to_string(),
                    },
                    m.roll_deg,
                    m.pitch_deg,
                    m.point_vertical_g,
                    m.point_lateral_g,
                    m.slam_probability,
                    m.wetness_probability
                ));
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("■").color(theme::GOOD));
            ui.label(RichText::new("operable").small());
            ui.label(RichText::new("■").color(theme::BAD));
            ui.label(RichText::new("limited").small());
            ui.label(
                RichText::new("(hover an hour for its motions)")
                    .small()
                    .color(theme::MUTED),
            );
        });
        // Motions against their limits.
        let start = f.hours.first().map(|h| h.time.clone()).unwrap_or_default();
        let x = |t: &str| crate::studio::globe::hours_since(&start, t) as f64;
        let line = |g: fn(&pelagic::seakeeping::Motions) -> f64| {
            outlook
                .iter()
                .filter_map(|h| h.motions.as_ref().map(|m| [x(&h.time), g(m)]))
                .collect::<Vec<_>>()
        };
        let c = &self.seakeeping.criteria;
        let mut guides = vec![];
        if let Some(l) = c.max_roll_deg {
            guides.push(l);
        }
        chart(
            ui,
            "Roll and pitch RMS (°) on the planned course",
            &[
                Series {
                    name: "Roll",
                    colour: SERIES[0],
                    points: line(|m| m.roll_deg),
                },
                Series {
                    name: "Pitch",
                    colour: SERIES[1],
                    points: line(|m| m.pitch_deg),
                },
            ],
            "hours from the first forecast hour",
            "degrees RMS",
            Options {
                origin: true,
                guides,
                room: 300.0,
                ..Default::default()
            },
        );
        let mut guides = vec![];
        if let Some(l) = c.max_point_vertical_g {
            guides.push(l);
        }
        chart(
            ui,
            "Vertical acceleration at the working point (g RMS)",
            &[Series {
                name: "Vertical",
                colour: SERIES[0],
                points: line(|m| m.point_vertical_g),
            }],
            "hours from the first forecast hour",
            "g RMS",
            Options {
                origin: true,
                guides,
                room: 300.0,
                ..Default::default()
            },
        );
        ui.label(
            RichText::new(crate::forecast::ATTRIBUTION)
                .small()
                .color(theme::MUTED),
        );
    }
}
