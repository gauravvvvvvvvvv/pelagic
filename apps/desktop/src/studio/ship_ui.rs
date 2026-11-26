//! Ship handling and COLREGs training in the editor: own ship (the selected
//! surface vessel, with its physics) meets other vessels in standard or
//! random encounters. Steer with helm and engine orders (or hand over to
//! the reference autopilot), watch the radar plot and the CPA/TCPA table,
//! and get a rule-by-rule debrief.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use egui::{Align2, Color32, FontId, Key, Pos2, RichText, Sense, Shape, Stroke, Ui, Vec2};
use pelagic::colregs::{self, Assessment, Outcome, Rules, Scenario, Session};
use std::sync::Arc;

const KNOT: f64 = 0.514_444;

pub(super) struct Panel {
    /// Scenarios for the prepared vehicle: (vehicle id and name, service
    /// speed, length, list).
    prepared: Option<(String, f64, f64, Vec<Scenario>)>,
    choice: usize,
    seed: u64,
    safe_m: f64,
    session: Option<Session>,
    running: bool,
    time_scale: f64,
    debrief: Option<Assessment>,
    suite: Option<Arc<Vec<Outcome>>>,
    /// Radar range (m); 0 chooses one from the scenario.
    range_m: f64,
    /// A vehicle that could not be own ship, and why (not retried).
    failed: Option<(String, String)>,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            prepared: None,
            choice: 0,
            seed: 1,
            safe_m: 0.0,
            session: None,
            running: false,
            time_scale: 4.0,
            debrief: None,
            suite: None,
            range_m: 0.0,
            failed: None,
        }
    }
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn nice(v: f64) -> f64 {
    let base = 10f64.powf(v.max(1e-6).log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|m| m * base)
        .find(|s| *s >= v)
        .unwrap_or(10.0 * base)
}

impl State {
    /// Scenarios for the selected vehicle (prepared once per vehicle).
    fn ship_prepare(&mut self) -> Result<(), String> {
        let vehicle = self
            .trial_subject()
            .ok_or("Select a surface vessel in the Outliner to be own ship.")?;
        let key = format!("{}#{}", vehicle.name, vehicle.id);
        if self.ship.prepared.as_ref().is_some_and(|p| p.0 == key) {
            return Ok(());
        }
        if let Some((k, e)) = &self.ship.failed
            && *k == key
        {
            return Err(e.clone());
        }
        let (speed, length) = match colregs::service_speed(self.scene(), &vehicle) {
            Ok(v) => v,
            Err(e) => {
                self.ship.failed = Some((key, e.clone()));
                return Err(e);
            }
        };
        let rules = Rules::for_length(length);
        self.ship.safe_m = rules.safe_cpa_m;
        self.ship.prepared = Some((key, speed, length, colregs::standard(speed, length, &rules)));
        self.ship.choice = self.ship.choice.min(7);
        Ok(())
    }

    fn ship_rules(&self) -> Rules {
        let length = self.ship.prepared.as_ref().map_or(7.0, |p| p.2);
        let mut rules = Rules::for_length(length);
        if self.ship.safe_m > 0.0 {
            let k = self.ship.safe_m / rules.safe_cpa_m;
            rules.safe_cpa_m *= k;
            rules.early_range_m *= k;
            rules.may_act_range_m *= k;
            rules.extremis_range_m *= k;
        }
        rules
    }

    /// The chosen scenario (standard list, then "random").
    fn ship_scenario(&self) -> Option<Scenario> {
        let (_, speed, length, list) = self.ship.prepared.as_ref()?;
        let rules = self.ship_rules();
        Some(
            if self.ship.choice < list.len() && self.ship.safe_m <= 0.0 {
                list[self.ship.choice].clone()
            } else if self.ship.choice < list.len() {
                colregs::standard(*speed, *length, &rules).swap_remove(self.ship.choice)
            } else {
                colregs::random(*speed, *length, &rules, self.ship.seed)
            },
        )
    }

    /// Starts the chosen scenario (helm orders; the autopilot is off).
    pub(super) fn ship_start(&mut self, choice: Option<usize>) {
        if let Err(e) = self.ship_prepare() {
            return self.log(e);
        }
        if let Some(c) = choice {
            self.ship.choice = c.min(8);
        }
        let Some(vehicle) = self.trial_subject() else {
            return;
        };
        let Some(scenario) = self.ship_scenario() else {
            return;
        };
        let name = scenario.name.clone();
        match Session::new(self.scene(), &vehicle, scenario, self.ship_rules()) {
            Ok(s) => {
                self.ship.session = Some(s);
                self.ship.running = true;
                self.ship.debrief = None;
                self.ship.range_m = 0.0;
                self.log(format!(
                    "Encounter started: {name}. Steer with ←/→ (5°) and ↑/↓ (speed), or hand over to the autopilot."
                ));
            }
            Err(e) => self.log(e),
        }
    }

    /// Script hook: hand the helm to the autopilot.
    pub(super) fn ship_autopilot(&mut self, on: bool) {
        if let Some(s) = &mut self.ship.session {
            s.set_autopilot(on);
        }
    }

    /// Script hook: advance the running encounter by `seconds`.
    pub(super) fn ship_advance(&mut self, seconds: f64) {
        if let Some(s) = &mut self.ship.session {
            s.step(seconds);
        }
        self.ship_check_finished();
    }

    fn ship_check_finished(&mut self) {
        let Some(s) = &self.ship.session else {
            return;
        };
        if s.finished() && self.ship.debrief.is_none() {
            let a = s.assess();
            self.ship.running = false;
            self.log(format!(
                "Encounter finished: {} — {}.",
                a.scenario,
                if a.passed {
                    "all checks passed"
                } else {
                    "see the debrief"
                }
            ));
            self.ship.debrief = Some(a);
        }
    }

    /// Runs every standard encounter plus random ones with the autopilot.
    pub(super) fn ship_suite(&mut self) {
        if let Err(e) = self.ship_prepare() {
            return self.log(e);
        }
        let Some(vehicle) = self.trial_subject() else {
            return;
        };
        let Some((_, speed, length, _)) = self.ship.prepared.clone() else {
            return;
        };
        let rules = self.ship_rules();
        let mut scenarios = colregs::standard(speed, length, &rules);
        for seed in 1..=12 {
            scenarios.push(colregs::random(speed, length, &rules, seed));
        }
        let scene = self.scene().clone();
        let threads = pelagic::campaign::default_threads();
        self.ship.suite = None;
        self.start_job(
            "Running encounters with the autopilot…",
            move |cancel, send| {
                let results = colregs::run_suite(
                    &scene,
                    &vehicle,
                    &scenarios,
                    &rules,
                    threads,
                    &cancel,
                    |d, t| {
                        let _ = send.send(Event::Progress(format!("Encounter {d}/{t}")));
                    },
                )?;
                Ok(Output::Colregs(results))
            },
        );
    }

    pub(super) fn colregs_finished(&mut self, results: Vec<Outcome>) {
        let passed = results.iter().filter(|(a, _)| a.passed).count();
        self.log(format!(
            "Autopilot encounters: {passed} of {} passed every check.",
            results.len()
        ));
        self.ship.suite = Some(Arc::new(results));
    }

    fn export_suite(&mut self) {
        let Some(results) = self.ship.suite.clone() else {
            return;
        };
        let Some(parent) =
            native::pick_folder(self.hwnd, "Choose a folder for the encounter results")
        else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-colregs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        match colregs::write_suite(&folder, &results) {
            Ok(()) => self.log(format!("Encounter results saved: {}", folder.display())),
            Err(e) => self.log(format!("Could not save the results: {e}")),
        }
    }

    pub(super) fn ship_panel(&mut self, ui: &mut Ui) {
        // Advance the live encounter.
        if self.ship.running
            && let Some(s) = &mut self.ship.session
        {
            let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
            s.step((dt * self.ship.time_scale).min(4.0));
            ui.ctx().request_repaint();
        }
        self.ship_check_finished();
        self.ship_keys(ui);
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("ship-left")
                .show(left, |ui| self.ship_controls(ui));
            egui::ScrollArea::vertical()
                .id_salt("ship-right")
                .show(right, |ui| self.ship_radar(ui));
        });
    }

    /// Arrow keys steer when no text field has focus.
    fn ship_keys(&mut self, ui: &Ui) {
        let Some(s) = &mut self.ship.session else {
            return;
        };
        if ui.memory(|m| m.focused().is_some()) || s.finished() {
            return;
        }
        let (mut course, mut speed) = s.order();
        let mut changed = false;
        ui.input(|i| {
            if i.key_pressed(Key::ArrowLeft) {
                course -= 5.0;
                changed = true;
            }
            if i.key_pressed(Key::ArrowRight) {
                course += 5.0;
                changed = true;
            }
            if i.key_pressed(Key::ArrowUp) {
                speed += 0.5 * KNOT;
                changed = true;
            }
            if i.key_pressed(Key::ArrowDown) {
                speed = (speed - 0.5 * KNOT).max(0.0);
                changed = true;
            }
        });
        if changed {
            s.set_order(course, speed);
        }
    }

    fn ship_controls(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(4.0);
        theme::heading(ui, "Ship handling and COLREGs");
        ui.label(
            RichText::new(
                "Own ship meets other vessels under the collision regulations. Steer it with \
                 its own physics, or hand over to the reference autopilot, then read the debrief.",
            )
            .small()
            .color(theme::MUTED),
        );
        let subject = self.trial_subject();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Own ship").color(theme::MUTED));
            match &subject {
                Some(e) => ui.label(RichText::new(&e.name).strong().color(theme::STRONG)),
                None => ui.label(
                    RichText::new("select a surface vessel in the Outliner").color(theme::WARN),
                ),
            };
            if let Some((_, speed, length, _)) = &self.ship.prepared {
                ui.label(
                    RichText::new(format!(
                        "· {:.0} m · service {:.1} kn",
                        length,
                        speed / KNOT
                    ))
                    .small()
                    .color(theme::MUTED),
                );
            }
        });
        if subject.is_some()
            && let Err(e) = self.ship_prepare()
        {
            ui.label(RichText::new(e).small().color(theme::WARN));
        }
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
        let names: Vec<String> = self
            .ship
            .prepared
            .as_ref()
            .map(|p| p.3.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        egui::Grid::new("ship-setup")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("Encounter");
                let current = names
                    .get(self.ship.choice)
                    .cloned()
                    .unwrap_or_else(|| "Random encounter".into());
                let combo = egui::ComboBox::from_id_salt("ship-scenario")
                    .selected_text(current)
                    .width(280.0)
                    .show_ui(ui, |ui| {
                        for (i, n) in names.iter().enumerate() {
                            ui.selectable_value(&mut self.ship.choice, i, n);
                        }
                        ui.selectable_value(&mut self.ship.choice, names.len(), "Random encounter");
                    });
                crate::studio::a11y::name(&combo.response, "Encounter");
                ui.end_row();
                if self.ship.choice >= names.len() {
                    ui.label("Seed");
                    ui.add(egui::DragValue::new(&mut self.ship.seed).range(1..=9999));
                    ui.end_row();
                }
                ui.label("Safe passing distance");
                let r = ui.add(
                    egui::DragValue::new(&mut self.ship.safe_m)
                        .range(10.0..=5000.0)
                        .speed(5.0)
                        .suffix(" m"),
                );
                crate::studio::a11y::name(&r, "Safe passing distance");
                ui.end_row();
            });
        if let Some(sc) = self.ship_scenario() {
            ui.label(RichText::new(&sc.description).small().color(theme::MUTED));
        }
        ui.horizontal_wrapped(|ui| {
            let start = ui
                .add_enabled(
                    subject.is_some() && !busy,
                    theme::primary("▶ Start encounter"),
                )
                .on_hover_text("Own ship starts on its route; you have the helm");
            crate::studio::a11y::name(&start, "Start encounter");
            if start.clicked() {
                self.ship_start(None);
            }
            let suite = ui
                .add_enabled(
                    subject.is_some() && !busy,
                    egui::Button::new("Run all with autopilot"),
                )
                .on_hover_text(
                    "Every standard encounter and 12 random ones, with the reference autopilot",
                );
            crate::studio::a11y::name(&suite, "Run all encounters with autopilot");
            if suite.clicked() {
                self.ship_suite();
            }
        });
        if self.ship.session.is_some() {
            ui.add_space(4.0);
            theme::heading(ui, "Bridge");
            let finished = self.ship.session.as_ref().is_some_and(|s| s.finished());
            ui.horizontal_wrapped(|ui| {
                let label = if self.ship.running {
                    "⏸ Pause"
                } else {
                    "▶ Run"
                };
                if ui
                    .add_enabled(!finished, egui::Button::new(label))
                    .clicked()
                {
                    self.ship.running = !self.ship.running;
                }
                for scale in [1.0, 4.0, 16.0] {
                    ui.selectable_value(&mut self.ship.time_scale, scale, format!("×{scale:.0}"));
                }
                if ui.button("Restart").clicked() {
                    self.ship_start(None);
                }
            });
            let Some(s) = &mut self.ship.session else {
                return;
            };
            let mut auto = s.autopilot();
            let r = ui.checkbox(&mut auto, "COLREGs autopilot has the helm");
            crate::studio::a11y::name(&r, "COLREGs autopilot");
            if r.changed() {
                s.set_autopilot(auto);
            }
            let (mut course, mut speed) = s.order();
            let service = s.scenario().speed_mps;
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                ui.label("Helm");
                for (label, d) in [
                    ("◀ 20°", -20.0),
                    ("◀ 5°", -5.0),
                    ("5° ▶", 5.0),
                    ("20° ▶", 20.0),
                ] {
                    if ui.small_button(label).clicked() {
                        course += d;
                        changed = true;
                    }
                }
                let r = ui.add(
                    egui::DragValue::new(&mut course)
                        .range(0.0..=359.9)
                        .speed(1.0)
                        .max_decimals(0)
                        .suffix("°"),
                );
                crate::studio::a11y::name(&r, "Ordered course");
                changed |= r.changed();
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Engines");
                for (label, f) in [("Stop", 0.0), ("Slow", 0.35), ("Half", 0.65), ("Full", 1.0)] {
                    if ui
                        .selectable_label((speed - service * f).abs() < 0.05, label)
                        .clicked()
                    {
                        speed = service * f;
                        changed = true;
                    }
                }
                let mut kn = speed / KNOT;
                let r = ui.add(
                    egui::DragValue::new(&mut kn)
                        .range(0.0..=60.0)
                        .speed(0.1)
                        .max_decimals(1)
                        .suffix(" kn"),
                );
                crate::studio::a11y::name(&r, "Ordered speed");
                if r.changed() {
                    speed = kn * KNOT;
                    changed = true;
                }
            });
            if changed {
                s.set_order(course.rem_euclid(360.0), speed);
            }
            ui.label(
                RichText::new("Keys: ←/→ alter 5°, ↑/↓ speed ±0.5 kn.")
                    .small()
                    .color(theme::MUTED),
            );
            // Target table (radar plotting aid).
            ui.add_space(4.0);
            let rules = s.rules().clone();
            egui::Grid::new("ship-targets")
                .num_columns(6)
                .striped(true)
                .spacing([10.0, 3.0])
                .show(ui, |ui| {
                    for h in ["Vessel", "Bearing", "Range", "CPA", "TCPA", "Situation"] {
                        ui.label(RichText::new(h).small().color(theme::MUTED));
                    }
                    ui.end_row();
                    for t in s.targets() {
                        let danger = t.cpa_m < rules.safe_cpa_m && t.tcpa_s > 0.0;
                        let colour = if t.collided || danger {
                            theme::BAD
                        } else {
                            theme::TEXT
                        };
                        ui.label(RichText::new(&t.name).small().strong().color(colour));
                        ui.label(RichText::new(format!("{:03.0}°", t.bearing_deg)).small());
                        ui.label(RichText::new(format!("{:.0} m", t.range_m)).small());
                        ui.label(
                            RichText::new(format!("{:.0} m", t.cpa_m))
                                .small()
                                .color(colour),
                        );
                        ui.label(
                            RichText::new(if t.tcpa_s > 0.0 {
                                clock(t.tcpa_s)
                            } else {
                                "past".into()
                            })
                            .small(),
                        );
                        ui.label(
                            RichText::new(
                                t.situation
                                    .map_or(t.kind.name().to_string(), |x| x.short().to_string()),
                            )
                            .small()
                            .color(theme::MUTED),
                        );
                        ui.end_row();
                    }
                });
        }
        if let Some(a) = self.ship.debrief.clone() {
            ui.add_space(6.0);
            theme::heading(ui, "Debrief");
            theme::chip(
                ui,
                if a.passed { theme::GOOD } else { theme::BAD },
                &format!(
                    "{} · {}",
                    a.scenario,
                    if a.passed {
                        "all checks passed"
                    } else {
                        "rules missed"
                    }
                ),
            );
            for t in &a.targets {
                ui.label(
                    RichText::new(format!(
                        "{} — {} · closest {:.0} m",
                        t.name,
                        t.situation.map_or("no risk of collision", |s| s.label()),
                        t.min_range_m
                    ))
                    .strong(),
                );
                for c in &t.checks {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(if c.passed { "✔" } else { "✖" }).color(if c.passed {
                                theme::GOOD
                            } else {
                                theme::BAD
                            }),
                        );
                        ui.label(RichText::new(&c.rule).small().color(theme::MUTED));
                        ui.label(RichText::new(&c.text).small());
                    });
                }
            }
        }
        if let Some(results) = self.ship.suite.clone() {
            ui.add_space(6.0);
            theme::heading(ui, "Autopilot encounters");
            ui.horizontal(|ui| {
                let passed = results.iter().filter(|(a, _)| a.passed).count();
                theme::chip(
                    ui,
                    if passed == results.len() {
                        theme::GOOD
                    } else {
                        theme::WARN
                    },
                    &format!("{passed} of {} passed", results.len()),
                );
                if ui.button("💾 Export…").clicked() {
                    self.export_suite();
                }
            });
            for (a, _) in results.iter() {
                let closest = a
                    .targets
                    .iter()
                    .map(|t| t.min_range_m)
                    .fold(f64::MAX, f64::min);
                let r = ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(if a.passed { "✔" } else { "✖" }).color(if a.passed {
                            theme::GOOD
                        } else {
                            theme::BAD
                        }),
                    );
                    ui.label(RichText::new(&a.scenario).small());
                    ui.label(
                        RichText::new(format!("closest {closest:.0} m"))
                            .small()
                            .color(theme::MUTED),
                    );
                });
                r.response.on_hover_text(a.summary());
            }
        }
        ui.label(
            RichText::new(
                "Thresholds (safe distance, early, substantial ≥ 30°, head-on ±10°) are training \
                 interpretations; the Regulations give no numbers.",
            )
            .small()
            .color(theme::MUTED),
        );
    }

    fn ship_radar(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let Some(s) = &self.ship.session else {
            theme::heading(ui, "Radar plot");
            ui.label(
                RichText::new(
                    "Start an encounter to see own ship at the centre, north up, with range \
                     rings, the other vessels, their one-minute velocity vectors and trails.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        let own = s.own();
        let rules = s.rules().clone();
        let targets = s.targets();
        let (roll, pitch) = s.attitude_deg();
        ui.horizontal_wrapped(|ui| {
            theme::chip(ui, theme::MUTED, &format!("t {}", clock(s.time())));
            theme::chip(
                ui,
                theme::MUTED,
                &format!(
                    "HDG {:03.0}° · COG {:03.0}° · SOG {:.1} kn",
                    own.heading_deg,
                    own.course_deg(),
                    own.speed() / KNOT
                ),
            );
            let (c, v) = s.order();
            theme::chip(
                ui,
                if s.autopilot() {
                    theme::ACCENT
                } else {
                    theme::MUTED
                },
                &format!(
                    "{} {:03.0}° / {:.1} kn",
                    if s.autopilot() {
                        "Autopilot"
                    } else {
                        "Ordered"
                    },
                    c,
                    v / KNOT
                ),
            );
            theme::chip(
                ui,
                theme::MUTED,
                &format!("roll {roll:+.1}° pitch {pitch:+.1}°"),
            );
        });
        let width = ui.available_width().max(260.0);
        let size = width
            .min(ui.available_height().max(300.0) - 10.0)
            .max(260.0);
        let (rect, radar) = ui.allocate_exact_size(Vec2::new(width, size), Sense::hover());
        crate::studio::a11y::name(&radar, "Radar plot");
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, Color32::from_rgb(6, 16, 20));
        let centre = rect.center();
        let radius = rect.width().min(rect.height()) * 0.47;
        // Range: fit the scenario's start, then keep.
        if self.ship.range_m <= 0.0 {
            let far = targets.iter().map(|t| t.range_m).fold(0.0, f64::max);
            self.ship.range_m = nice(far * 1.1).max(4.0 * rules.safe_cpa_m);
        }
        let range = self.ship.range_m;
        let scale = radius as f64 / range;
        let screen = |p: [f64; 2]| {
            Pos2::new(
                centre.x + ((p[0] - own.position[0]) * scale) as f32,
                centre.y - ((p[1] - own.position[1]) * scale) as f32,
            )
        };
        let ring = Color32::from_rgba_unmultiplied(80, 190, 170, 70);
        for k in 1..=4 {
            let r = radius * k as f32 / 4.0;
            painter.circle_stroke(centre, r, Stroke::new(1.0, ring));
            painter.text(
                centre + Vec2::new(r * 0.71 + 3.0, -r * 0.71),
                Align2::LEFT_BOTTOM,
                format!("{:.0} m", range * k as f64 / 4.0),
                FontId::proportional(10.0),
                Color32::from_rgba_unmultiplied(120, 200, 180, 160),
            );
        }
        for deg in (0..360).step_by(30) {
            let a = (deg as f32).to_radians();
            let dir = Vec2::new(a.sin(), -a.cos());
            painter.line_segment(
                [centre + dir * (radius - 6.0), centre + dir * radius],
                Stroke::new(1.0, ring),
            );
            painter.text(
                centre + dir * (radius + 10.0),
                Align2::CENTER_CENTER,
                format!("{deg:03}"),
                FontId::proportional(9.5),
                Color32::from_rgba_unmultiplied(120, 200, 180, 150),
            );
        }
        // Planned route.
        let route = pelagic::sar::towards(1.0, s.scenario().route_course_deg);
        let far = range * 3.0;
        painter.extend(Shape::dashed_line(
            &[
                screen([-route[0] * far, -route[1] * far]),
                screen([route[0] * far, route[1] * far]),
            ],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(160, 170, 180, 90)),
            6.0,
            6.0,
        ));
        // Trails every 10 s.
        let log = s.log();
        for sample in log.iter().step_by(10) {
            painter.circle_filled(
                screen(sample.own.position),
                1.6,
                Color32::from_rgba_unmultiplied(45, 197, 210, 120),
            );
            for m in &sample.targets {
                painter.circle_filled(
                    screen(m.position),
                    1.6,
                    Color32::from_rgba_unmultiplied(220, 225, 230, 90),
                );
            }
        }
        // Safe-distance ring around own ship.
        painter.extend(Shape::dashed_line(
            &(0..=64)
                .map(|k| {
                    let a = k as f32 / 64.0 * std::f32::consts::TAU;
                    centre + Vec2::new(a.cos(), a.sin()) * (rules.safe_cpa_m * scale) as f32
                })
                .collect::<Vec<_>>(),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(242, 182, 92, 120)),
            4.0,
            4.0,
        ));
        let vector_s = 60.0;
        // Targets.
        for t in &targets {
            let p = screen(t.motion.position);
            let danger = t.cpa_m < rules.safe_cpa_m && t.tcpa_s > 0.0;
            let colour = if t.collided || danger {
                theme::BAD
            } else {
                Color32::from_rgb(220, 225, 230)
            };
            painter.circle_stroke(p, 5.0, Stroke::new(1.5, colour));
            let tip = screen([
                t.motion.position[0] + t.motion.velocity[0] * vector_s,
                t.motion.position[1] + t.motion.velocity[1] * vector_s,
            ]);
            painter.line_segment([p, tip], Stroke::new(1.5, colour));
            painter.text(
                p + Vec2::new(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                format!(
                    "{}{}",
                    t.name,
                    if danger {
                        format!(" · CPA {:.0} m in {}", t.cpa_m, clock(t.tcpa_s))
                    } else {
                        String::new()
                    }
                ),
                FontId::proportional(10.5),
                colour,
            );
            if danger {
                // Predicted positions at CPA.
                let at = |m: &pelagic::colregs::Motion| {
                    [
                        m.position[0] + m.velocity[0] * t.tcpa_s,
                        m.position[1] + m.velocity[1] * t.tcpa_s,
                    ]
                };
                let a = screen(at(&own));
                let b = screen(at(&t.motion));
                painter.line_segment([a, b], Stroke::new(1.0, theme::BAD));
                painter.circle_filled(b, 2.5, theme::BAD);
            }
        }
        // Own ship: hull outline along the heading, and its vector.
        let h = (own.heading_deg as f32).to_radians();
        let fwd = Vec2::new(h.sin(), -h.cos());
        let side = Vec2::new(fwd.y, -fwd.x);
        let len = ((s.own_length() * scale) as f32).max(9.0);
        let hull = vec![
            centre + fwd * len * 0.6,
            centre - fwd * len * 0.4 + side * len * 0.22,
            centre - fwd * len * 0.4 - side * len * 0.22,
        ];
        painter.add(Shape::convex_polygon(hull, theme::ACCENT, Stroke::NONE));
        let tip = screen([
            own.position[0] + own.velocity[0] * vector_s,
            own.position[1] + own.velocity[1] * vector_s,
        ]);
        painter.line_segment([centre, tip], Stroke::new(2.0, theme::ACCENT));
        painter.text(
            rect.left_bottom() + Vec2::new(10.0, -10.0),
            Align2::LEFT_BOTTOM,
            "north up · 1-minute true vectors · dashed ring: safe passing distance",
            FontId::proportional(10.5),
            theme::MUTED,
        );
        // Zoom buttons.
        let mut zoom = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Range").small().color(theme::MUTED));
            if ui.small_button("−").clicked() {
                zoom = Some(0.5);
            }
            if ui.small_button("+").clicked() {
                zoom = Some(2.0);
            }
        });
        if let Some(z) = zoom {
            self.ship.range_m = (self.ship.range_m / z).clamp(50.0, 100_000.0);
        }
    }
}
