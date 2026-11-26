//! Sensor Monitor panel and mounted-camera view, ported from the legacy
//! editor's sensor view. Shows the latest observation from the live
//! simulation, or from the recording sample being played back: a sonar fan
//! plot plus navigation readouts (depth, position, DVL velocity, IMU).
use super::{State, theme};
use egui::{Align2, Color32, FontId, Pos2, RichText, Sense, Stroke, Ui, Vec2};
use pelagic::{instruments::Observation, math::Vec3};

/// Eye, target and up of the mounted sensor camera on its vehicle.
pub(super) fn mounted_camera(model: &crate::view::Model) -> Option<(Vec3, Vec3, Vec3, f64)> {
    let session = model.session.as_ref()?;
    let sensors = &model.document.as_ref()?.scene.sensors;
    let snapshot = session.view_snapshot();
    let body = snapshot
        .vehicles
        .iter()
        .find(|p| p.id == sensors.entity)
        .or_else(|| snapshot.vehicles.first())?;
    let eye = body.position + body.orientation.rotate(sensors.mount);
    let target = eye + body.orientation.rotate(Vec3::new(1.0, 0.0, -0.15));
    let up = body.orientation.rotate(Vec3::new(0.0, 0.0, 1.0));
    Some((eye, target, up, sensors.camera_fov))
}

impl State {
    fn observation(&self) -> Option<Observation> {
        self.model
            .playback
            .and_then(|i| self.model.run.as_ref()?.frames.get(i)?.observations.first())
            .or_else(|| self.model.session.as_ref()?.observations.first())
            .cloned()
    }

    pub(super) fn sensor_monitor(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let toggle = ui
            .checkbox(&mut self.sensor_camera, "View from mounted camera")
            .on_hover_text("Show the 3D viewport from the sensor camera on its vehicle");
        if toggle.changed() {
            self.view_dirty = true;
        }
        let range = self
            .model
            .document
            .as_ref()
            .map_or(80.0, |d| d.scene.sensors.range);
        let Some(o) = self.observation() else {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Play or step the simulation to sample the mounted sensors.")
                    .color(theme::MUTED),
            );
            return;
        };
        let source = if self.model.playback.is_some() {
            "recorded"
        } else {
            "live"
        };
        // Readouts on the left, sonar fan filling the rest of the panel.
        let full = ui.available_rect_before_wrap();
        let split = (full.width() * 0.42).clamp(300.0, 540.0).min(full.width());
        let left = egui::Rect::from_min_size(full.min, Vec2::new(split, full.height()));
        let rect =
            egui::Rect::from_min_max(Pos2::new(full.min.x + split + 8.0, full.min.y), full.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            let speed = o.velocity.length();
            ui.label(
                RichText::new(format!(
                    "{source} · sample {:.2} s · delivered {:.2} s",
                    o.time, o.available_time
                ))
                .small(),
            );
            ui.label(
                RichText::new(format!("Depth {:.2} m · speed {speed:.2} m/s", o.depth)).strong(),
            );
            ui.add_space(4.0);
            egui::Grid::new("nav")
                .num_columns(2)
                .spacing([12.0, 3.0])
                .show(ui, |ui| {
                    for (label, p, unit) in [
                        ("Position", o.position, "m"),
                        ("DVL velocity", o.velocity, "m/s"),
                        ("Gyro", o.angular_velocity, "rad/s"),
                        ("Accelerometer", o.specific_force, "m/s²"),
                    ] {
                        ui.label(RichText::new(label).small().color(theme::MUTED));
                        ui.label(
                            RichText::new(format!("{:+8.2} {:+8.2} {:+8.2} {unit}", p.x, p.y, p.z))
                                .small()
                                .monospace(),
                        );
                        ui.end_row();
                    }
                });
        });
        let response = ui.allocate_rect(rect, Sense::hover());
        let returns: Vec<f64> = o.beams.iter().filter_map(|b| b.range).collect();
        let nearest = returns.iter().copied().fold(f64::INFINITY, f64::min);
        crate::studio::a11y::name(
            &response,
            &if returns.is_empty() {
                format!("Sonar fan: no returns of {} beams", o.beams.len())
            } else {
                format!(
                    "Sonar fan: {} of {} beams returned, nearest {nearest:.1} m",
                    returns.len(),
                    o.beams.len()
                )
            },
        );
        // Display scale: zoom to the returns (nice steps), never beyond the
        // configured maximum range.
        let farthest = returns.iter().copied().fold(0.0, f64::max);
        let scale = display_range(farthest, range);
        let loudest = o.beams.iter().map(|b| b.intensity).fold(1e-12, f64::max);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, theme::BG);
        painter.text(
            rect.right_top() + Vec2::new(-8.0, 6.0),
            Align2::RIGHT_TOP,
            format!("display {scale:.0} m of {range:.0} m max"),
            FontId::proportional(10.0),
            theme::MUTED,
        );
        let origin = Pos2::new(rect.center().x, rect.top() + 14.0);
        let radius = (rect.height() - 30.0).min(rect.width() * 0.47).max(20.0);
        let fan = self
            .model
            .document
            .as_ref()
            .map_or(120.0, |d| d.scene.sensors.fan_degrees)
            .to_radians()
            * 0.5;
        let point = |angle: f64, fraction: f64| {
            origin
                + Vec2::new(
                    angle.sin() as f32 * radius * fraction as f32,
                    angle.cos() as f32 * radius * fraction as f32,
                )
        };
        let grid = Stroke::new(1.0, Color32::from_gray(52));
        for fraction in [0.25, 0.5, 0.75, 1.0] {
            let arc: Vec<Pos2> = (0..=48)
                .map(|i| point(-fan + 2.0 * fan * i as f64 / 48.0, fraction))
                .collect();
            painter.add(egui::Shape::line(arc, grid));
            painter.text(
                point(fan, fraction) + Vec2::new(4.0, 0.0),
                Align2::LEFT_CENTER,
                format!("{} m", trim(scale * fraction)),
                FontId::proportional(10.0),
                theme::MUTED,
            );
        }
        for edge in [-fan, fan] {
            painter.line_segment([origin, point(edge, 1.0)], grid);
        }
        for beam in &o.beams {
            let Some(r) = beam.range else { continue };
            let fraction = (r / scale).clamp(0.0, 1.0);
            let end = point(beam.angle, fraction);
            painter.line_segment(
                [origin, end],
                Stroke::new(1.0, Color32::from_rgb(35, 70, 78)),
            );
            let glow = ((beam.intensity / loudest).clamp(0.0, 1.0) * 155.0) as u8;
            painter.circle_filled(
                end,
                2.5,
                Color32::from_rgb(100 + glow / 2, 200, 180 + glow / 3),
            );
        }
        painter.circle_filled(origin, 3.0, Color32::from_rgb(235, 190, 80));
    }
}
/// Smallest "nice" display range (1-2-5 steps) holding the farthest return
/// with some margin, capped at the sensor's maximum range.
pub(super) fn display_range(farthest: f64, max: f64) -> f64 {
    if farthest <= 0.0 {
        return max;
    }
    let target = farthest * 1.25;
    let mut step = 1.0;
    loop {
        for m in [1.0, 2.0, 5.0] {
            if step * m >= target {
                return (step * m).min(max);
            }
        }
        step *= 10.0;
        if step > max * 10.0 {
            return max;
        }
    }
}
fn trim(v: f64) -> String {
    if v.fract().abs() < 1e-6 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn display_range_zooms_to_returns_in_nice_steps() {
        use super::display_range;
        assert_eq!(display_range(5.8, 80.0), 10.0);
        assert_eq!(display_range(17.0, 80.0), 50.0);
        assert_eq!(display_range(70.0, 80.0), 80.0);
        assert_eq!(display_range(0.0, 80.0), 80.0);
        assert_eq!(display_range(0.6, 80.0), 1.0);
    }
}
