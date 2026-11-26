//! Hull and harbour inspection in the editor: camera passes over the
//! harbour pilings or a vessel's hull, the stand-off set by image sharpness
//! and water clarity, coverage of every surface patch, and a run with a
//! hovering vehicle's physics.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use crate::studio::charts::SURFACE;
use egui::{Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2};
use pelagic::inspection::{self, Report, Setup, Target};
use std::sync::Arc;

pub(super) struct Panel {
    setup: Setup,
    /// 0: harbour pilings; otherwise the id of the entity whose hull is
    /// inspected.
    target: u64,
    attenuation_from_scene: bool,
    result: Option<Arc<Report>>,
    flown: bool,
    fold: bool,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            setup: Setup::default(),
            target: 0,
            attenuation_from_scene: true,
            result: None,
            flown: true,
            fold: false,
        }
    }
}

const MET: Color32 = Color32::from_rgb(0x39, 0x87, 0xe5);
const COARSE: Color32 = Color32::from_rgb(0x8a, 0x3f, 0x22);
const GAP: Color32 = Color32::from_rgb(0xd9, 0x59, 0x26);
const TRACK: Color32 = Color32::from_rgb(242, 182, 92);

impl State {
    fn inspection_target(&self) -> Result<Target, String> {
        let scene = self.scene();
        if self.inspection.target == 0 {
            inspection::pilings(scene)
        } else {
            let e = scene
                .entities
                .iter()
                .find(|e| e.id == self.inspection.target)
                .ok_or("The chosen vessel is no longer in the scene.")?;
            inspection::hull(e, scene.density)
        }
    }

    fn inspection_setup(&self) -> Setup {
        let mut s = self.inspection.setup.clone();
        if self.inspection.attenuation_from_scene {
            // Scene attenuation (clear water can be very low): at least 0.05/m.
            s.attenuation = self.scene().attenuation.max(0.05);
        }
        s
    }

    pub(super) fn inspection_finished(&mut self, report: Report) {
        let flown = report.simulated.as_ref();
        self.log(format!(
            "Inspection of {}: planned {:.1} % imaged to {:.1} mm/px{}.",
            report.target.name,
            report.ideal.covered_percent,
            report.setup.gsd_mm,
            flown.map_or(String::new(), |r| format!(
                ", flown by {} {:.1} % ({:.2} m RMS)",
                report.vehicle.clone().unwrap_or_default(),
                r.covered_percent,
                r.position_rms_m
            ))
        ));
        for n in report
            .notes
            .iter()
            .chain(flown.iter().flat_map(|r| &r.notes))
        {
            self.log(format!("Inspection: {n}"));
        }
        self.inspection.flown = report.simulated.is_some();
        self.inspection.fold = true;
        self.inspection.result = Some(Arc::new(report));
    }

    pub(super) fn start_inspection(&mut self, physics: bool) {
        let target = match self.inspection_target() {
            Ok(t) => t,
            Err(e) => return self.log(e),
        };
        let setup = self.inspection_setup();
        if let Err(e) = setup.validate() {
            return self.log(e);
        }
        let vehicle = if physics {
            match self.trial_subject() {
                Some(v) if v.id != self.inspection.target || self.inspection.target == 0 => Some(v),
                Some(_) => {
                    return self.log("Select a different vehicle from the one being inspected.");
                }
                None => {
                    return self.log("Select a hovering vehicle (an ROV) in the Outliner.");
                }
            }
        } else {
            None
        };
        let mut scene = self.scene().clone();
        // The inspected hull is geometry here, not a moving body.
        let target_id = self.inspection.target;
        scene.entities.retain(|e| e.id != target_id);
        self.inspection.result = None;
        let label = match &vehicle {
            Some(v) => format!("Inspecting with {}…", v.name),
            None => "Planning the inspection…".into(),
        };
        self.start_job(&label, move |cancel, send| {
            let report = inspection::run(&scene, target, &setup, vehicle.as_ref(), &cancel, |f| {
                let _ = send.send(Event::Progress(format!(
                    "Flying the passes: {:.0} %",
                    f * 100.0
                )));
            })?;
            Ok(Output::Inspection(Box::new(report)))
        });
    }

    fn export_inspection(&mut self) {
        let Some(report) = self.inspection.result.clone() else {
            return;
        };
        let Some(parent) = native::pick_folder(self.hwnd, "Choose a folder for the inspection")
        else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-inspection-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        match report.write(&folder) {
            Ok(()) => self.log(format!(
                "Inspection saved (summary, faces, path, track): {}",
                folder.display()
            )),
            Err(e) => self.log(format!("Could not save the inspection: {e}")),
        }
    }

    pub(super) fn inspection_panel(&mut self, ui: &mut Ui) {
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("inspect-left")
                .show(left, |ui| self.inspection_controls(ui));
            egui::ScrollArea::vertical()
                .id_salt("inspect-right")
                .show(right, |ui| self.inspection_results(ui));
        });
    }

    fn inspection_controls(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(4.0);
        theme::heading(ui, "Hull and harbour inspection");
        ui.label(
            RichText::new(
                "Camera passes over underwater surfaces at a stand-off set by image sharpness \
                 and water clarity, and how much is imaged well enough, planned and flown.",
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
            let plan = ui.add_enabled(!busy, theme::primary("▶ Plan inspection"));
            crate::studio::a11y::name(&plan, "Plan inspection");
            if plan.clicked() {
                self.start_inspection(false);
            }
            let fly = ui
                .add_enabled(
                    !busy && subject.is_some(),
                    egui::Button::new("▶ Fly with vehicle physics"),
                )
                .on_hover_text(
                    "The selected vehicle flies the passes in the scene's sea and current",
                );
            crate::studio::a11y::name(&fly, "Fly inspection with vehicle physics");
            if fly.clicked() {
                self.start_inspection(true);
            }
            if self.inspection.result.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new("💾 Export…"))
                    .clicked()
            {
                self.export_inspection();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Vehicle").color(theme::MUTED));
            match &subject {
                Some(e) => ui.label(RichText::new(&e.name).strong().color(theme::STRONG)),
                None => ui.label(
                    RichText::new("select an ROV in the Outliner to fly it").color(theme::MUTED),
                ),
            };
        });
        let scene = self.scene().clone();
        let fold = std::mem::take(&mut self.inspection.fold).then_some(false);
        egui::CollapsingHeader::new(RichText::new("Target, camera and water").strong())
            .id_salt("inspect-setup")
            .default_open(true)
            .open(fold)
            .show(ui, |ui| {
                let p = &mut self.inspection;
                egui::Grid::new("inspect-grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Target");
                        let name = if p.target == 0 {
                            "Harbour pilings".to_string()
                        } else {
                            scene
                                .entities
                                .iter()
                                .find(|e| e.id == p.target)
                                .map_or("—".into(), |e| format!("{} hull", e.name))
                        };
                        let combo = egui::ComboBox::from_id_salt("inspect-target")
                            .selected_text(name)
                            .width(240.0)
                            .show_ui(ui, |ui| {
                                if scene.structure {
                                    ui.selectable_value(&mut p.target, 0, "Harbour pilings");
                                }
                                for e in scene.entities.iter().filter(|e| e.kind == 1) {
                                    ui.selectable_value(
                                        &mut p.target,
                                        e.id,
                                        format!("{} hull", e.name),
                                    );
                                }
                            });
                        crate::studio::a11y::name(&combo.response, "Inspection target");
                        ui.end_row();
                        let s = &mut p.setup;
                        let field = |ui: &mut Ui,
                                     label: &str,
                                     v: &mut f64,
                                     lo: f64,
                                     hi: f64,
                                     speed: f64,
                                     suffix: &str| {
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
                        field(
                            ui,
                            "Required sharpness",
                            &mut s.gsd_mm,
                            0.05,
                            100.0,
                            0.05,
                            " mm/px",
                        );
                        field(
                            ui,
                            "Field of view",
                            &mut s.camera.fov_deg,
                            5.0,
                            170.0,
                            0.5,
                            "°",
                        );
                        ui.label("Image");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut s.camera.pixels[0])
                                    .range(64.0..=20_000.0)
                                    .speed(10.0)
                                    .suffix(" ×"),
                            );
                            ui.add(
                                egui::DragValue::new(&mut s.camera.pixels[1])
                                    .range(64.0..=20_000.0)
                                    .speed(10.0)
                                    .suffix(" px"),
                            );
                        });
                        ui.end_row();
                        let mut pct = s.overlap * 100.0;
                        field(ui, "Overlap", &mut pct, 0.0, 90.0, 1.0, " %");
                        s.overlap = pct / 100.0;
                        field(ui, "Speed", &mut s.speed_mps, 0.02, 3.0, 0.01, " m/s");
                        field(ui, "Clearance", &mut s.clearance_m, 0.1, 20.0, 0.05, " m");
                        ui.label("Water clarity");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut p.attenuation_from_scene, "scene")
                                .on_hover_text("Beam attenuation from the scene's water");
                            let mut c = if p.attenuation_from_scene {
                                scene.attenuation.max(0.05)
                            } else {
                                p.setup.attenuation
                            };
                            let r = ui.add_enabled(
                                !p.attenuation_from_scene,
                                egui::DragValue::new(&mut c)
                                    .range(0.001..=20.0)
                                    .speed(0.01)
                                    .suffix(" /m"),
                            );
                            crate::studio::a11y::name(&r, "Water attenuation");
                            if !p.attenuation_from_scene {
                                p.setup.attenuation = c;
                            }
                        });
                        ui.end_row();
                        let mut contrast = p.setup.min_contrast * 100.0;
                        ui.label("Least contrast");
                        ui.add(
                            egui::DragValue::new(&mut contrast)
                                .range(1.0..=95.0)
                                .speed(0.5)
                                .suffix(" %"),
                        );
                        p.setup.min_contrast = contrast / 100.0;
                        ui.end_row();
                    });
            });
        let s = self.inspection_setup();
        let (d, notes) = s.standoff();
        ui.label(
            RichText::new(format!(
                "{:.2} mm/px reached within {:.2} m; contrast ≥ {:.0} % to {:.1} m → stand-off {:.2} m.",
                s.gsd_mm,
                s.gsd_range_m(),
                s.min_contrast * 100.0,
                s.visibility_m(),
                d
            ))
            .small()
            .color(theme::MUTED),
        );
        for n in notes {
            ui.label(RichText::new(n).small().color(theme::WARN));
        }
        let Some(r) = self.inspection.result.clone() else {
            return;
        };
        ui.add_space(6.0);
        theme::heading(ui, "Results");
        let row = |ui: &mut Ui, name: &str, run: &inspection::Run| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(name).strong());
                theme::chip(
                    ui,
                    if run.covered_percent >= 95.0 {
                        theme::GOOD
                    } else {
                        theme::WARN
                    },
                    &format!("{:.1} % imaged to spec", run.covered_percent),
                );
                theme::chip(
                    ui,
                    theme::MUTED,
                    &format!("{:.0} m · {:.0} min", run.path_m, run.minutes),
                );
            });
            if !run.track.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "position error {:.2} m RMS · least clearance {:.2} m · {} frames",
                        run.position_rms_m, run.min_clearance_m, run.frames
                    ))
                    .small()
                    .color(if run.min_clearance_m < 0.0 {
                        theme::BAD
                    } else {
                        theme::MUTED
                    }),
                );
            }
        };
        row(ui, "Planned", &r.ideal);
        if let Some(sim) = &r.simulated {
            row(
                ui,
                &format!("Flown by {}", r.vehicle.clone().unwrap_or_default()),
                sim,
            );
        }
        let run = if self.inspection.flown {
            r.simulated.as_ref().unwrap_or(&r.ideal)
        } else {
            &r.ideal
        };
        egui::Grid::new("inspect-faces")
            .num_columns(4)
            .striped(true)
            .spacing([10.0, 2.0])
            .show(ui, |ui| {
                for h in ["Face", "Area", "To spec", "Best"] {
                    ui.label(RichText::new(h).small().color(theme::MUTED));
                }
                ui.end_row();
                for f in &run.faces {
                    ui.label(RichText::new(&f.name).small());
                    ui.label(RichText::new(format!("{:.1} m²", f.area_m2)).small());
                    ui.label(
                        RichText::new(format!("{:.0} %", f.covered_percent))
                            .small()
                            .color(if f.covered_percent >= 95.0 {
                                theme::TEXT
                            } else {
                                theme::WARN
                            }),
                    );
                    ui.label(
                        RichText::new(
                            f.best_gsd_mm
                                .map_or("—".into(), |g| format!("{g:.2} mm/px")),
                        )
                        .small(),
                    );
                    ui.end_row();
                }
            });
        for n in r
            .notes
            .iter()
            .chain(r.simulated.iter().flat_map(|x| &x.notes))
        {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("⚠").color(theme::WARN));
                ui.label(RichText::new(n).small());
            });
        }
    }

    fn inspection_results(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let Some(r) = self.inspection.result.clone() else {
            theme::heading(ui, "Path and coverage");
            ui.label(
                RichText::new(
                    "Plan the inspection to see the passes from above and from the side, and each \
                     face's coverage: blue imaged to spec, brown seen but too coarse or dim, \
                     orange missed.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        if r.simulated.is_some() {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.inspection.flown, false, "Planned");
                ui.selectable_value(
                    &mut self.inspection.flown,
                    true,
                    format!("Flown by {}", r.vehicle.clone().unwrap_or_default()),
                );
            });
        }
        let run = if self.inspection.flown {
            r.simulated.as_ref().unwrap_or(&r.ideal)
        } else {
            &r.ideal
        };
        let width = ui.available_width().max(240.0);
        // Plan (x, y) and side (horizontal along the target's long axis, z).
        let mut boxes_xy = vec![];
        for b in &r.target.boxes {
            let (s, c) = (b.yaw.sin(), b.yaw.cos());
            let corner = |sx: f64, sy: f64| {
                [
                    b.centre.x + sx * b.half.x * c - sy * b.half.y * s,
                    b.centre.y + sx * b.half.x * s + sy * b.half.y * c,
                ]
            };
            boxes_xy.push([
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ]);
        }
        let mut pts: Vec<[f64; 3]> = r
            .path
            .iter()
            .map(|w| [w.position.x, w.position.y, w.position.z])
            .collect();
        pts.extend(run.track.iter().map(|p| [p[1], p[2], p[3]]));
        for q in boxes_xy.iter().flatten() {
            pts.push([q[0], q[1], 0.0]);
        }
        let draw =
            |ui: &mut Ui, title: &str, project: &dyn Fn([f64; 3]) -> [f64; 2], side: bool| {
                ui.label(RichText::new(title).small().color(theme::MUTED));
                let h = (width * 0.42).clamp(140.0, 300.0);
                let (rect, response) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
                crate::studio::a11y::name(&response, title);
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 6.0, SURFACE);
                let proj: Vec<[f64; 2]> = pts.iter().map(|p| project(*p)).collect();
                let (x0, x1) = proj
                    .iter()
                    .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[0]), a.1.max(p[0])));
                let (y0, y1) = proj
                    .iter()
                    .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p[1]), a.1.max(p[1])));
                let scale = ((x1 - x0) / (rect.width() as f64 - 24.0))
                    .max((y1 - y0) / (rect.height() as f64 - 24.0))
                    .max(0.01);
                let mid = [(x0 + x1) * 0.5, (y0 + y1) * 0.5];
                let screen = |p: [f64; 2]| {
                    Pos2::new(
                        rect.center().x + ((p[0] - mid[0]) / scale) as f32,
                        rect.center().y - ((p[1] - mid[1]) / scale) as f32,
                    )
                };
                if side {
                    let y = screen([0.0, 0.0]).y;
                    painter.line_segment(
                        [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                        Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 200, 230, 120)),
                    );
                }
                for (k, b) in r.target.boxes.iter().enumerate() {
                    if side {
                        let a = project([b.centre.x - b.half.x, b.centre.y, b.centre.z - b.half.z]);
                        let c = project([b.centre.x + b.half.x, b.centre.y, b.centre.z + b.half.z]);
                        painter.rect_filled(
                            Rect::from_two_pos(screen(a), screen(c)),
                            1.0,
                            Color32::from_rgb(70, 78, 86),
                        );
                    } else {
                        let poly: Vec<Pos2> = boxes_xy[k].iter().map(|q| screen(*q)).collect();
                        // Structure above the water (a deck): outline only.
                        if b.centre.z - b.half.z > -0.1 {
                            let mut ring = poly.clone();
                            ring.push(poly[0]);
                            painter.add(Shape::line(
                                ring,
                                Stroke::new(1.0, Color32::from_rgb(110, 120, 130)),
                            ));
                        } else {
                            painter.add(Shape::convex_polygon(
                                poly,
                                Color32::from_rgb(70, 78, 86),
                                Stroke::NONE,
                            ));
                        }
                    }
                }
                for w in r.path.windows(2) {
                    let a = screen(project([w[0].position.x, w[0].position.y, w[0].position.z]));
                    let b = screen(project([w[1].position.x, w[1].position.y, w[1].position.z]));
                    let colour = if w[1].capture {
                        MET
                    } else {
                        Color32::from_rgba_unmultiplied(160, 170, 180, 110)
                    };
                    painter.line_segment([a, b], Stroke::new(1.0, colour));
                }
                if !run.track.is_empty() {
                    let line: Vec<Pos2> = run
                        .track
                        .iter()
                        .map(|p| screen(project([p[1], p[2], p[3]])))
                        .collect();
                    painter.add(Shape::line(line, Stroke::new(1.2, TRACK)));
                }
            };
        let long = r
            .target
            .boxes
            .iter()
            .max_by(|a, b| (a.half.x * a.half.y).total_cmp(&(b.half.x * b.half.y)))
            .map_or(0.0, |b| b.yaw);
        let (ls, lc) = (long.sin(), long.cos());
        draw(
            ui,
            "From above (passes blue, transits grey, vehicle amber)",
            &|p| [p[0], p[1]],
            false,
        );
        draw(
            ui,
            "From the side (water surface line)",
            &|p| [p[0] * lc + p[1] * ls, p[2]],
            true,
        );
        // Face atlas.
        ui.label(
            RichText::new("Faces (blue to spec, brown too coarse or dim, orange missed)")
                .small()
                .color(theme::MUTED),
        );
        ui.horizontal_wrapped(|ui| {
            for (f, c) in r.target.faces.iter().zip(&run.coverage) {
                let aspect = (f.half[0] / f.half[1]) as f32;
                let h = 90.0f32;
                let w = (h * aspect).clamp(14.0, 260.0);
                let (rect, response) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
                let painter = ui.painter_at(rect);
                let (cw, ch) = (rect.width() / c.cols as f32, rect.height() / c.rows as f32);
                for j in 0..c.rows {
                    for i in 0..c.cols {
                        let colour = match c.state[j * c.cols + i] {
                            2 => MET,
                            1 => COARSE,
                            _ => GAP,
                        };
                        let x = rect.left() + i as f32 * cw;
                        let y = rect.bottom() - (j + 1) as f32 * ch;
                        painter.rect_filled(
                            Rect::from_min_size(Pos2::new(x, y), Vec2::new(cw + 0.5, ch + 0.5)),
                            0.0,
                            colour,
                        );
                    }
                }
                response.on_hover_text(f.name.clone());
            }
        });
    }
}
