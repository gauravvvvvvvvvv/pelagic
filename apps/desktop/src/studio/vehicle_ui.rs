//! Vehicle prototypes in the editor: import/export of vehicle definitions
//! (`*.pelagic-vehicle.json` + optional OBJ hull) and the Details sections
//! that define a body's hydrodynamics, thrusters and controllers.
use super::{Selection, State, native, row, section, theme, vec3_row};
use crate::library;
use crate::viewport_edit::Actor;
use egui::{RichText, Ui};
use pelagic::{
    definition::{VehicleDefinition, estimate_hydrodynamics},
    entities::Entity,
    math::Vec3,
};

const FILTER: &str = "Pelagic vehicle definition\0*.json\0";

impl State {
    pub(super) fn import_vehicle(&mut self) {
        let Some(path) = native::pick_file(
            self.hwnd,
            false,
            "Import vehicle definition",
            FILTER,
            "json",
        ) else {
            return;
        };
        self.import_vehicle_from(&path);
    }

    /// Imports a definition (and its hull mesh) from `path` near the camera.
    pub(super) fn import_vehicle_from(&mut self, path: &str) {
        let result = (|| -> Result<(crate::project::Scene, usize), String> {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let definition = VehicleDefinition::parse(&text)?;
            let mesh = match &definition.mesh_obj {
                None => None,
                Some(relative) => {
                    let file = std::path::Path::new(path)
                        .parent()
                        .unwrap_or(std::path::Path::new("."))
                        .join(relative);
                    let obj = std::fs::read_to_string(&file)
                        .map_err(|e| format!("Hull mesh {}: {e}", file.display()))?;
                    let name = file
                        .file_name()
                        .map_or("hull.obj".into(), |n| n.to_string_lossy().into_owned());
                    Some((
                        name.clone(),
                        pelagic::assets::Mesh::parse_obj(&obj, &name).map_err(str::to_string)?,
                    ))
                }
            };
            let target = self.cam.target();
            library::place_definition(
                self.scene(),
                &definition,
                mesh,
                [target.x, target.y, target.z],
            )
        })();
        match result {
            Ok((scene, index)) => {
                let id = scene.entities[index].id;
                let name = scene.entities[index].name.clone();
                if self.apply(scene, &format!("Imported vehicle {name}")) {
                    self.select(Selection::Actor(Actor::Entity(id)));
                }
            }
            Err(e) => self.log(format!("Vehicle import failed: {e}")),
        }
    }

    pub(super) fn export_vehicle(&mut self, entity: &Entity) {
        let Some(path) =
            native::pick_file(self.hwnd, true, "Export vehicle definition", FILTER, "json")
        else {
            return;
        };
        let result = (|| -> Result<String, String> {
            let path = std::path::Path::new(&path);
            let mut definition = VehicleDefinition::from_entity(entity);
            if let Some(obj) = library::hull_obj(self.scene(), entity) {
                let stem = path
                    .file_name()
                    .map_or("vehicle".into(), |n| n.to_string_lossy().into_owned());
                let stem = stem
                    .trim_end_matches(".json")
                    .trim_end_matches(".pelagic-vehicle")
                    .to_string();
                let mesh = format!("{stem}.obj");
                std::fs::write(path.with_file_name(&mesh), obj).map_err(|e| e.to_string())?;
                definition.mesh_obj = Some(mesh);
            }
            std::fs::write(path, definition.to_json()?).map_err(|e| e.to_string())?;
            Ok(path.display().to_string())
        })();
        match result {
            Ok(path) => self.log(format!("Vehicle definition saved: {path}")),
            Err(e) => self.log(format!("Vehicle export failed: {e}")),
        }
    }
}

/// Details sections for a body's physics model. Sets `changed` on any edit.
pub(super) fn physics_sections(ui: &mut Ui, entity: &mut Entity, density: f64, changed: &mut bool) {
    section(ui, "Hydrodynamics", |ui| {
        let values = [
            ("Inertia kg·m²", entity.effective_inertia(), "", 0.01),
            ("Drag N/(m/s)²", entity.effective_drag(), "", 0.1),
            (
                "Rot. drag N·m/(rad/s)²",
                entity.effective_angular_drag(),
                "",
                0.05,
            ),
        ];
        for (i, (label, value, unit, speed)) in values.iter().enumerate() {
            let mut v = [value.x, value.y, value.z];
            if vec3_row(ui, label, &mut v, *speed, unit) {
                let next = Some(Vec3::new(v[0].max(0.0), v[1].max(0.0), v[2].max(0.0)));
                match i {
                    0 => {
                        entity.model.inertia =
                            next.map(|n| Vec3::new(n.x.max(1e-4), n.y.max(1e-4), n.z.max(1e-4)))
                    }
                    1 => entity.model.drag = next,
                    _ => entity.model.angular_drag = next,
                }
                *changed = true;
            }
        }
        let a = entity.model.added_mass;
        let mut v = [a.x, a.y, a.z];
        if vec3_row(ui, "Added mass kg", &mut v, 0.1, "") {
            entity.model.added_mass = Vec3::new(v[0].max(0.0), v[1].max(0.0), v[2].max(0.0));
            *changed = true;
        }
        let a = entity.model.added_inertia;
        let mut v = [a.x, a.y, a.z];
        if vec3_row(ui, "Added inertia kg·m²", &mut v, 0.1, "") {
            entity.model.added_inertia = Vec3::new(v[0].max(0.0), v[1].max(0.0), v[2].max(0.0));
            *changed = true;
        }
        let c = entity.model.center_buoyancy;
        let mut v = [c.x, c.y, c.z];
        if vec3_row(ui, "Centre of buoyancy m", &mut v, 0.005, "") {
            entity.model.center_buoyancy = Vec3::new(v[0], v[1], v[2]);
            *changed = true;
        }
        ui.label("Surface crossing");
        *changed |= ui
            .checkbox(&mut entity.model.partial_buoyancy, "Partial buoyancy")
            .changed();
        ui.end_row();
        ui.label("Collisions");
        *changed |= ui
            .checkbox(&mut entity.model.contacts, "Seabed and bodies")
            .changed();
        ui.end_row();
        ui.label("");
        ui.horizontal_wrapped(|ui| {
            let customised = entity.model.inertia.is_some()
                || entity.model.drag.is_some()
                || entity.model.angular_drag.is_some();
            for (label, streamlined) in [
                ("Estimate (blunt)", false),
                ("Estimate (streamlined)", true),
            ] {
                if ui
                    .button(label)
                    .on_hover_text(
                        "Inertia, drag, rotational drag, added mass and added inertia from the \
                         hull size (box hull, strip theory). Replace with measured values when \
                         available.",
                    )
                    .clicked()
                {
                    estimate_hydrodynamics(
                        entity.model.half,
                        entity.mass,
                        entity.volume,
                        density,
                        streamlined,
                    )
                    .apply(&mut entity.model);
                    *changed = true;
                }
            }
            if customised
                && ui
                    .button("Legacy defaults")
                    .on_hover_text("Mass-scaled defaults used by earlier versions")
                    .clicked()
            {
                entity.model.inertia = None;
                entity.model.drag = None;
                entity.model.angular_drag = None;
                *changed = true;
            }
        });
        ui.end_row();
    });
    section(ui, "Thrusters", |ui| {
        let mut remove = None;
        let count = entity.model.thrusters.len();
        for (i, t) in entity.model.thrusters.iter_mut().enumerate() {
            let mut p = [t.position.x, t.position.y, t.position.z];
            if vec3_row(ui, &format!("#{} position", i + 1), &mut p, 0.01, " m") {
                t.position = Vec3::new(p[0], p[1], p[2]);
                *changed = true;
            }
            let mut d = [t.direction.x, t.direction.y, t.direction.z];
            if vec3_row(ui, &format!("#{} direction", i + 1), &mut d, 0.01, "") {
                let n = Vec3::new(d[0], d[1], d[2]);
                if n.length() > 1e-3 {
                    t.direction = n.normalized();
                    *changed = true;
                }
            }
            ui.label(format!("#{} max thrust", i + 1));
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::DragValue::new(&mut t.limit)
                        .range(0.0..=1e6)
                        .speed(1.0)
                        .suffix(" N"),
                );
                crate::studio::a11y::name(&r, &format!("Thruster {} max thrust", i + 1));
                *changed |= r.changed();
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
            ui.end_row();
        }
        if let Some(i) = remove {
            entity.model.thrusters.remove(i);
            *changed = true;
        }
        ui.label(
            RichText::new(format!("{count} of 32"))
                .small()
                .color(theme::MUTED),
        );
        if ui
            .add_enabled(count < 32, egui::Button::new("Add thruster"))
            .on_hover_text("A forward thruster at the stern; edit its position and direction")
            .clicked()
        {
            entity.model.thrusters.push(pelagic::actuators::Thruster {
                direction: Vec3::new(1.0, 0.0, 0.0),
                position: Vec3::new(-entity.model.half.x, 0.0, 0.0),
                limit: entity.model.thrust_limit.clamp(1.0, 100.0),
            });
            *changed = true;
        }
        ui.end_row();
    });
    fins_section(ui, entity, changed);
    section(ui, "Controllers", |ui| {
        *changed |= row(
            ui,
            "Actuator lag",
            &mut entity.model.response_seconds,
            0.01,
            0.0..=60.0,
            " s",
        );
        let mut depth = entity.model.hold_depth.is_some();
        ui.label("Depth hold");
        ui.horizontal(|ui| {
            if ui.checkbox(&mut depth, "").changed() {
                entity.model.hold_depth = depth.then_some(-entity.position.z.min(0.0));
                *changed = true;
            }
            if let Some(d) = entity.model.hold_depth.as_mut() {
                let r = ui.add(
                    egui::DragValue::new(d)
                        .range(0.0..=11000.0)
                        .speed(0.1)
                        .suffix(" m"),
                );
                crate::studio::a11y::name(&r, "Depth hold target");
                *changed |= r.changed();
            }
        });
        ui.end_row();
        let mut heading = entity.model.hold_heading.is_some();
        ui.label("Heading hold");
        ui.horizontal(|ui| {
            if ui.checkbox(&mut heading, "").changed() {
                entity.model.hold_heading = heading.then_some(entity.heading.to_radians());
                *changed = true;
            }
            if let Some(h) = entity.model.hold_heading.as_mut() {
                let mut degrees = h.to_degrees();
                let r = ui.add(
                    egui::DragValue::new(&mut degrees)
                        .range(-360.0..=360.0)
                        .speed(0.5)
                        .suffix("°"),
                );
                crate::studio::a11y::name(&r, "Heading hold target");
                if r.changed() {
                    *h = degrees.to_radians();
                    *changed = true;
                }
            }
        });
        ui.end_row();
    });
}

/// Fins, keels, rudders and hydroplanes.
fn fins_section(ui: &mut Ui, entity: &mut Entity, changed: &mut bool) {
    use pelagic::fins::{self, Fin};
    section(ui, "Fins and rudders", |ui| {
        let mut remove = None;
        let count = entity.model.fins.len();
        for (i, f) in entity.model.fins.iter_mut().enumerate() {
            let n = i + 1;
            ui.label(format!("#{n} role"));
            ui.horizontal(|ui| {
                let roles = [
                    (fins::FIXED, "Fixed fin / keel"),
                    (fins::YAW, "Rudder (yaw)"),
                    (fins::PITCH, "Hydroplane (pitch)"),
                    (fins::ROLL, "Roll fin"),
                ];
                let current = roles
                    .iter()
                    .find(|r| r.0 == f.channel)
                    .map_or("Fixed fin / keel", |r| r.1);
                let combo = egui::ComboBox::from_id_salt(("fin-role", i))
                    .selected_text(current)
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for (channel, label) in roles {
                            *changed |= ui
                                .selectable_value(&mut f.channel, channel, label)
                                .changed();
                        }
                    });
                crate::studio::a11y::name(&combo.response, &format!("Fin {n} role"));
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
            ui.end_row();
            let mut p = [f.position.x, f.position.y, f.position.z];
            if vec3_row(ui, &format!("#{n} position"), &mut p, 0.01, " m") {
                f.position = Vec3::new(p[0], p[1], p[2]);
                *changed = true;
            }
            // Orientation: angle of the lift direction about body X
            // (0° vertical fin lifting to port, 90° horizontal lifting up).
            let mut angle = f.normal.z.atan2(f.normal.y).to_degrees();
            ui.label(format!("#{n} orientation"))
                .on_hover_text("0° vertical (rudder, keel), 90° horizontal (hydroplane)");
            let r = ui.add(
                egui::DragValue::new(&mut angle)
                    .range(-180.0..=180.0)
                    .speed(1.0)
                    .suffix("°"),
            );
            crate::studio::a11y::name(&r, &format!("Fin {n} orientation"));
            if r.changed() {
                let a = angle.to_radians();
                f.normal = Vec3::new(0.0, a.cos(), a.sin());
                *changed = true;
            }
            ui.end_row();
            let mut value = |ui: &mut Ui,
                             label: &str,
                             v: &mut f64,
                             range: std::ops::RangeInclusive<f64>,
                             speed: f64,
                             unit: &str| {
                ui.label(format!("#{n} {label}"));
                let r = ui.add(
                    egui::DragValue::new(v)
                        .range(range)
                        .speed(speed)
                        .suffix(unit)
                        .max_decimals(4),
                );
                crate::studio::a11y::name(&r, &format!("Fin {n} {label}"));
                *changed |= r.changed();
                ui.end_row();
            };
            value(ui, "area", &mut f.area, 0.0001..=100.0, 0.001, " m²");
            value(
                ui,
                "lift slope",
                &mut f.lift_slope,
                0.0..=10.0,
                0.01,
                " /rad",
            );
            value(ui, "stall", &mut f.stall_deg, 1.0..=90.0, 0.5, "°");
            if f.channel != fins::FIXED {
                value(
                    ui,
                    "max deflection",
                    &mut f.max_deflection_deg,
                    0.0..=60.0,
                    0.5,
                    "°",
                );
            }
        }
        if let Some(i) = remove {
            entity.model.fins.remove(i);
            *changed = true;
        }
        ui.label(
            RichText::new(format!("{count} of 16"))
                .small()
                .color(theme::MUTED),
        );
        ui.horizontal_wrapped(|ui| {
            let h = entity.model.half;
            // A plate of span ≈ hull half-depth and aspect ratio 1.5.
            let span = h.z.min(h.y) * 1.2;
            let area = span * span / 1.5;
            let slope = fins::lift_slope(1.5);
            let mut add = |fin: Fin| {
                entity.model.fins.push(fin);
                *changed = true;
            };
            let room = count < 16;
            if ui
                .add_enabled(room, egui::Button::new("Add rudder"))
                .on_hover_text("A vertical rudder at the stern, 30° deflection")
                .clicked()
            {
                add(Fin {
                    position: Vec3::new(-h.x, 0.0, -h.z * 0.5),
                    normal: Vec3::new(0.0, 1.0, 0.0),
                    area,
                    lift_slope: slope,
                    stall_deg: 25.0,
                    channel: fins::YAW,
                    max_deflection_deg: 30.0,
                });
            }
            if ui
                .add_enabled(count + 1 < 16, egui::Button::new("Add hydroplanes"))
                .on_hover_text("A pair of horizontal stern planes for pitch control")
                .clicked()
            {
                for side in [1.0, -1.0] {
                    add(Fin {
                        position: Vec3::new(-h.x * 0.9, side * (h.y + span * 0.5), 0.0),
                        normal: Vec3::new(0.0, 0.0, 1.0),
                        area,
                        lift_slope: slope,
                        stall_deg: 25.0,
                        channel: fins::PITCH,
                        max_deflection_deg: 25.0,
                    });
                }
            }
            if ui
                .add_enabled(room, egui::Button::new("Add fixed fin"))
                .on_hover_text("A vertical stabiliser or keel aft of the centre")
                .clicked()
            {
                add(Fin {
                    position: Vec3::new(-h.x * 0.8, 0.0, -h.z),
                    normal: Vec3::new(0.0, 1.0, 0.0),
                    area,
                    lift_slope: slope,
                    stall_deg: 22.0,
                    channel: fins::FIXED,
                    max_deflection_deg: 0.0,
                });
            }
        });
        ui.end_row();
    });
}
