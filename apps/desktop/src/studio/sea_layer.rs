//! Live seas on the Earth view: a global wave-height map from the forecast
//! models (with travel-direction arrows and a forecast-hour slider), and an
//! hourly sea forecast for any point that can drive the current scene.
use super::{Globe, R, View, theme, unit};
use crate::forecast;
use egui::{Color32, Pos2, RichText, Stroke, Ui, Vec2};
use pelagic::metocean::{Conditions, Forecast, WaveField};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// Grid spacing of the global map, degrees.
const STEP: f64 = 10.0;
/// Forecast days on the global map.
const DAYS: u32 = 3;

enum Message {
    Partial(usize, usize, WaveField),
    Done(Result<WaveField, String>),
}

#[derive(Default)]
pub(super) struct SeaLayer {
    pub on: bool,
    field: Option<Arc<WaveField>>,
    loading: Option<(mpsc::Receiver<Message>, Arc<AtomicBool>)>,
    progress: (usize, usize),
    hour: usize,
    playing: bool,
    error: Option<String>,
    /// Point forecast panel.
    pub forecast: Option<Arc<Forecast>>,
    forecast_loading: Option<mpsc::Receiver<Result<Forecast, String>>>,
    forecast_hour: usize,
    forecast_error: Option<String>,
    last_tick: Option<std::time::Instant>,
}

/// Sequential colour for a wave height (one hue, darker → lighter with
/// height on the dark globe) with an alpha that lets the relief show.
pub(super) fn height_colour(hs: f32) -> Color32 {
    // Calm seas stay nearly clear so the ocean reads as ocean; colour and
    // opacity grow with height.
    let opacity = (hs / 8.0).clamp(0.0, 1.0);
    let t = opacity.sqrt();
    let stops: [(f32, [f32; 3]); 3] = [
        (0.0, [107.0, 42.0, 13.0]),
        (0.55, [217.0, 89.0, 38.0]),
        (1.0, [255.0, 222.0, 196.0]),
    ];
    let (a, b) = if t <= stops[1].0 {
        (stops[0], stops[1])
    } else {
        (stops[1], stops[2])
    };
    let w = (t - a.0) / (b.0 - a.0);
    let c: Vec<u8> = (0..3)
        .map(|i| (a.1[i] + (b.1[i] - a.1[i]) * w).round() as u8)
        .collect();
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (30.0 + 200.0 * opacity) as u8)
}

fn hour_label(time: &str, now: &str) -> String {
    let offset = hours_between(now, time);
    let rel = match offset {
        Some(0) => "now".to_string(),
        Some(h) if h > 0 => format!("+{h} h"),
        Some(h) => format!("{h} h"),
        None => String::new(),
    };
    format!("{} UTC {rel}", time.replace('T', " "))
}

/// Whole hours from `a` to `b` ("YYYY-MM-DDTHH:MM").
pub(super) fn hours_between(a: &str, b: &str) -> Option<i64> {
    let parse = |s: &str| -> Option<i64> {
        let (date, time) = s.split_once('T')?;
        let mut d = date.split('-').map(|p| p.parse::<i64>());
        let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
        let hour = time.get(..2)?.parse::<i64>().ok()?;
        // Days from civil (H. Hinnant).
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + day - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some((era * 146_097 + doe - 719_468) * 24 + hour)
    };
    Some(parse(b)? - parse(a)?)
}

impl SeaLayer {
    fn start(&mut self) {
        let (send, receive) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        std::thread::spawn(move || {
            let result = forecast::field(STEP, DAYS, |done, total, partial| {
                let _ = send.send(Message::Partial(done, total, partial.clone()));
                !stop.load(Ordering::Relaxed)
            });
            let _ = send.send(Message::Done(result));
        });
        self.loading = Some((receive, cancel));
        self.error = None;
    }

    pub(super) fn poll(&mut self) {
        let mut done = None;
        if let Some((rx, _)) = &self.loading {
            while let Ok(message) = rx.try_recv() {
                match message {
                    Message::Partial(d, t, partial) => {
                        self.progress = (d, t);
                        if self.field.is_none() || !partial.points.is_empty() {
                            self.field = Some(Arc::new(partial));
                        }
                    }
                    Message::Done(result) => done = Some(result),
                }
            }
        }
        if let Some(result) = done {
            self.loading = None;
            match result {
                Ok(field) => {
                    let now = forecast::now_hour();
                    self.hour = field
                        .times
                        .iter()
                        .rposition(|t| t.as_str() <= now.as_str())
                        .unwrap_or(0);
                    self.field = Some(Arc::new(field));
                }
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(rx) = &self.forecast_loading
            && let Ok(result) = rx.try_recv()
        {
            self.forecast_loading = None;
            match result {
                Ok(f) => {
                    self.forecast_hour = f.index_at(&forecast::now_hour());
                    self.forecast = Some(Arc::new(f));
                    self.forecast_error = None;
                }
                Err(e) => self.forecast_error = Some(e),
            }
        }
    }

    /// A download (map or point forecast) is in progress.
    pub(super) fn downloading(&self) -> bool {
        self.loading.is_some() || self.forecast_loading.is_some()
    }

    /// The point-forecast panel is loading or has something to say.
    pub(super) fn panel_wanted(&self) -> bool {
        self.forecast_loading.is_some() || self.forecast_error.is_some()
    }

    pub(super) fn busy(&self) -> bool {
        self.loading.is_some() || self.forecast_loading.is_some() || self.playing
    }

    /// Requests the hourly forecast at a point (opens the panel).
    pub(super) fn request_point(&mut self, lat: f64, lon: f64) {
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(forecast::point(lat, lon, 7));
        });
        self.forecast_loading = Some(receive);
        self.forecast_error = None;
    }

    /// Height, travel bearing and period at (lat, lon) for the shown hour.
    pub(super) fn sample(&self, lat: f64, lon: f64) -> Option<(f32, f32, f32)> {
        let field = self.field.as_ref()?;
        let cols = (360.0 / STEP).round() as usize;
        let row = ((lat + 70.0) / STEP).round();
        let col = ((lon + 180.0) / STEP).round().rem_euclid(cols as f64);
        if !(0.0..=15.0).contains(&row) {
            return None;
        }
        let p = field.points.get(row as usize * cols + col as usize)?;
        Some((
            (*p.height.get(self.hour)?)?,
            (*p.direction.get(self.hour)?)? + 180.0,
            p.period
                .get(self.hour)
                .copied()
                .flatten()
                .unwrap_or(f32::NAN),
        ))
    }
}

impl Globe {
    /// Toolbar row for the live map (shown while it is on).
    pub(super) fn sea_toolbar(&mut self, ui: &mut Ui) {
        self.sea.poll();
        if self.sea.on
            && self.sea.field.is_none()
            && self.sea.loading.is_none()
            && self.sea.error.is_none()
        {
            self.sea.start();
        }
        let now = forecast::now_hour();
        ui.horizontal_wrapped(|ui| {
            let waves = ui
                .selectable_label(self.sea.on, "〰 Live waves")
                .on_hover_text("Global wave-height forecast (Open-Meteo wave models), 3 days hourly");
            crate::studio::a11y::name(&waves, "Live waves");
            if waves.clicked() {
                self.sea.on = !self.sea.on;
            }
            let point = ui
                .button("Sea forecast")
                .on_hover_text("7-day hourly sea forecast at the view centre");
            if point.clicked() {
                self.sea.request_point(self.lat, self.lon);
            }
            ui.checkbox(&mut self.live_sea, "Live sea in new scenes")
                .on_hover_text("New ocean scenes start with this hour's forecast wind sea, swell, wind, current and sea temperature");
            if !self.sea.on {
                return;
            }
            ui.separator();
            if let Some(field) = self.sea.field.clone()
                && !field.times.is_empty()
            {
                let last = field.times.len() - 1;
                self.sea.hour = self.sea.hour.min(last);
                let play = ui
                    .button(if self.sea.playing { "⏸" } else { "▶" })
                    .on_hover_text("Animate the forecast");
                crate::studio::a11y::name(
                    &play,
                    if self.sea.playing {
                        "Pause forecast"
                    } else {
                        "Play forecast"
                    },
                );
                if play.clicked() {
                    self.sea.playing = !self.sea.playing;
                }
                if self.sea.playing {
                    let due = self
                        .sea
                        .last_tick
                        .is_none_or(|t| t.elapsed().as_millis() >= 250);
                    if due {
                        self.sea.hour = (self.sea.hour + 1) % (last + 1);
                        self.sea.last_tick = Some(std::time::Instant::now());
                    }
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(60));
                }
                let slider = ui.add(
                    egui::Slider::new(&mut self.sea.hour, 0..=last)
                        .show_value(false)
                        .text(""),
                );
                crate::studio::a11y::name(&slider, "Forecast hour");
                ui.label(
                    RichText::new(hour_label(&field.times[self.sea.hour], &now))
                        .color(theme::STRONG),
                );
            }
            ui.separator();
            ui.label(RichText::new("Hs").small().color(theme::MUTED));
            let (rect, _) = ui.allocate_exact_size(Vec2::new(120.0, 10.0), egui::Sense::hover());
            let mut mesh = egui::Mesh::default();
            for i in 0..=16 {
                let x = rect.left() + rect.width() * i as f32 / 16.0;
                let c = height_colour(8.0 * (i as f32 / 16.0).powi(2));
                let c = Color32::from_rgb(c.r(), c.g(), c.b());
                mesh.colored_vertex(Pos2::new(x, rect.top()), c);
                mesh.colored_vertex(Pos2::new(x, rect.bottom()), c);
                if i > 0 {
                    let k = (2 * i) as u32;
                    mesh.add_triangle(k - 2, k - 1, k);
                    mesh.add_triangle(k - 1, k + 1, k);
                }
            }
            ui.painter().add(egui::Shape::mesh(mesh));
            ui.label(RichText::new("0 · 2 · 8+ m").small().color(theme::MUTED));
            if let Some((_, cancel)) = &self.sea.loading {
                ui.spinner();
                ui.label(
                    RichText::new(format!(
                        "Loading live seas {}/{} points",
                        self.sea.progress.0, self.sea.progress.1
                    ))
                    .small(),
                );
                if ui.small_button("Stop").clicked() {
                    cancel.store(true, Ordering::Relaxed);
                }
            } else if self.sea.field.is_some() && ui.small_button("Refresh").clicked() {
                self.sea.start();
            }
            if let Some(e) = &self.sea.error {
                ui.label(
                    RichText::new(format!("Live seas: {e}"))
                        .small()
                        .color(theme::WARN),
                );
                if ui.small_button("Retry").clicked() {
                    self.sea.start();
                }
            }
        });
    }

    /// Draws the wave-height map and direction arrows over the globe.
    pub(super) fn draw_sea(&self, view: &View, painter: &egui::Painter) {
        if !self.sea.on {
            return;
        }
        let Some(field) = self.sea.field.as_ref() else {
            return;
        };
        let cols = (360.0 / STEP).round() as usize;
        let rows = field.points.len() / cols;
        let hour = self.sea.hour;
        let value = |r: usize, c: usize| -> Option<f32> {
            field
                .points
                .get(r * cols + (c % cols))
                .and_then(|p| p.height.get(hour).copied().flatten())
        };
        let n = 4usize;
        let mut mesh = egui::Mesh::default();
        for r in 0..rows.saturating_sub(1) {
            for c in 0..cols {
                let corners = [
                    value(r, c),
                    value(r, c + 1),
                    value(r + 1, c),
                    value(r + 1, c + 1),
                ];
                let present: Vec<f32> = corners.iter().flatten().copied().collect();
                if present.len() < 2 {
                    continue;
                }
                let fill = present.iter().sum::<f32>() / present.len() as f32;
                let v: Vec<f32> = corners.iter().map(|x| x.unwrap_or(fill)).collect();
                let lat0 = -70.0 + r as f64 * STEP;
                let lon0 = -180.0 + c as f64 * STEP;
                let base = mesh.vertices.len() as u32;
                let mut ok = vec![false; (n + 1) * (n + 1)];
                for a in 0..=n {
                    for b in 0..=n {
                        let (fa, fb) = (a as f32 / n as f32, b as f32 / n as f32);
                        let lat = lat0 + STEP * fa as f64;
                        let lon = lon0 + STEP * fb as f64;
                        let hs = v[0] * (1.0 - fa) * (1.0 - fb)
                            + v[1] * (1.0 - fa) * fb
                            + v[2] * fa * (1.0 - fb)
                            + v[3] * fa * fb;
                        let land = self.elevation(lat, lon).is_some_and(|(e, _)| e > 0.0);
                        let p = unit(lat, lon);
                        let at = view.project([p[0] * R, p[1] * R, p[2] * R]);
                        let (pos, facing) = at.unwrap_or((Pos2::ZERO, false));
                        ok[a * (n + 1) + b] = facing;
                        let colour = if land {
                            Color32::TRANSPARENT
                        } else {
                            height_colour(hs)
                        };
                        mesh.colored_vertex(pos, colour);
                    }
                }
                for a in 0..n {
                    for b in 0..n {
                        let i = a * (n + 1) + b;
                        let (j, k, l) = (i + 1, i + n + 1, i + n + 2);
                        if ok[i] && ok[j] && ok[k] && ok[l] {
                            let (i, j, k, l) = (
                                base + i as u32,
                                base + j as u32,
                                base + k as u32,
                                base + l as u32,
                            );
                            mesh.add_triangle(i, j, l);
                            mesh.add_triangle(i, l, k);
                        }
                    }
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
        // Direction of travel at each sea point.
        let stroke = Stroke::new(1.2, Color32::from_white_alpha(170));
        for p in &field.points {
            let (Some(Some(_)), Some(Some(from))) = (p.height.get(hour), p.direction.get(hour))
            else {
                continue;
            };
            if self
                .elevation(p.latitude, p.longitude)
                .is_some_and(|(e, _)| e > 0.0)
            {
                continue;
            }
            let bearing = ((*from as f64) + 180.0).to_radians();
            let la = p.latitude.to_radians();
            let lo = p.longitude.to_radians();
            let east = [-lo.sin(), lo.cos(), 0.0];
            let north = [-la.sin() * lo.cos(), -la.sin() * lo.sin(), la.cos()];
            let centre = unit(p.latitude, p.longitude);
            let len = STEP.to_radians() * 0.32;
            let tip = |s: f64| {
                [
                    (centre[0] + (north[0] * bearing.cos() + east[0] * bearing.sin()) * len * s)
                        * R,
                    (centre[1] + (north[1] * bearing.cos() + east[1] * bearing.sin()) * len * s)
                        * R,
                    (centre[2] + (north[2] * bearing.cos() + east[2] * bearing.sin()) * len * s)
                        * R,
                ]
            };
            let (Some((a, fa)), Some((b, fb))) = (view.project(tip(-1.0)), view.project(tip(1.0)))
            else {
                continue;
            };
            if !(fa && fb) || a.distance(b) < 6.0 {
                continue;
            }
            painter.line_segment([a, b], stroke);
            let d = (b - a).normalized();
            let side = Vec2::new(-d.y, d.x);
            let head = (a.distance(b) * 0.3).min(7.0);
            painter.line_segment([b, b - d * head + side * head * 0.6], stroke);
            painter.line_segment([b, b - d * head - side * head * 0.6], stroke);
        }
    }

    /// Point forecast side panel: conditions by hour, a chart, and buttons
    /// to apply the sea to the current scene or build a scene with it.
    pub(super) fn forecast_panel(&mut self, ui: &mut Ui) {
        ui.set_max_width(ui.available_width());
        let now = forecast::now_hour();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Sea forecast").strong().color(theme::STRONG));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close = ui.small_button("✕");
                crate::studio::a11y::name(&close, "Close sea forecast");
                if close.clicked() {
                    self.sea.forecast = None;
                    self.sea.forecast_error = None;
                    self.sea.forecast_loading = None;
                }
            });
        });
        if self.sea.forecast_loading.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading the forecast…");
            });
        }
        if let Some(e) = &self.sea.forecast_error {
            ui.label(RichText::new(e).color(theme::WARN));
        }
        let Some(f) = self.sea.forecast.clone() else {
            return;
        };
        ui.label(
            RichText::new(format!(
                "{:.3}°{} {:.3}°{} · 7 days hourly",
                f.latitude.abs(),
                if f.latitude >= 0.0 { "N" } else { "S" },
                f.longitude.abs(),
                if f.longitude >= 0.0 { "E" } else { "W" }
            ))
            .small()
            .color(theme::MUTED),
        );
        let last = f.hours.len().saturating_sub(1);
        self.sea.forecast_hour = self.sea.forecast_hour.min(last);
        let slider = ui.add(
            egui::Slider::new(&mut self.sea.forecast_hour, 0..=last)
                .show_value(false)
                .text(""),
        );
        crate::studio::a11y::name(&slider, "Forecast hour at point");
        let c = &f.hours[self.sea.forecast_hour];
        ui.label(RichText::new(hour_label(&c.time, &now)).color(theme::STRONG));
        let fmt = |v: Option<f64>, unit: &str, d: usize| {
            v.map_or_else(|| "—".to_string(), |v| format!("{v:.d$}{unit}"))
        };
        let from = |v: Option<f64>| v.map_or_else(String::new, |d| format!(" · {}", compass(d)));
        egui::Grid::new("sea-forecast-now")
            .num_columns(2)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                let mut row = |label: &str, value: String| {
                    ui.label(RichText::new(label).color(theme::MUTED));
                    ui.label(value);
                    ui.end_row();
                };
                row(
                    "Total sea",
                    format!(
                        "Hs {} · {}{}",
                        fmt(c.wave_height, " m", 2),
                        fmt(c.wave_period, " s", 1),
                        from(c.wave_direction)
                    ),
                );
                row(
                    "Wind sea",
                    format!(
                        "{} · Tp {}{}",
                        fmt(c.wind_wave_height, " m", 2),
                        fmt(c.wind_wave_peak_period.or(c.wind_wave_period), " s", 1),
                        from(c.wind_wave_direction)
                    ),
                );
                row(
                    "Swell",
                    format!(
                        "{} · Tp {}{}",
                        fmt(c.swell_height, " m", 2),
                        fmt(c.swell_peak_period.or(c.swell_period), " s", 1),
                        from(c.swell_direction)
                    ),
                );
                row(
                    "Wind",
                    format!("{}{}", fmt(c.wind_speed, " m/s", 1), from(c.wind_direction)),
                );
                row(
                    "Current",
                    format!(
                        "{}{}",
                        fmt(c.current_speed, " m/s", 2),
                        c.current_direction
                            .map_or_else(String::new, |d| format!(" → {}", compass(d)))
                    ),
                );
                row("Sea temperature", fmt(c.sea_temperature, " °C", 1));
            });
        let start = &f.hours[0].time;
        let x = |t: &str| hours_between(start, t).unwrap_or(0) as f64;
        let series = |g: fn(&Conditions) -> Option<f64>| {
            f.hours
                .iter()
                .filter_map(|h| g(h).map(|v| [x(&h.time), v]))
                .collect::<Vec<_>>()
        };
        super::super::charts::chart(
            ui,
            "Significant wave height",
            &[
                super::super::charts::Series {
                    name: "Total",
                    colour: super::super::charts::SERIES[0],
                    points: series(|h| h.wave_height),
                },
                super::super::charts::Series {
                    name: "Wind sea",
                    colour: super::super::charts::SERIES[1],
                    points: series(|h| h.wind_wave_height),
                },
                super::super::charts::Series {
                    name: "Swell",
                    colour: super::super::charts::SERIES[2],
                    points: series(|h| h.swell_height),
                },
            ],
            "hours from the first forecast hour",
            "Hs (m)",
            super::super::charts::Options {
                origin: true,
                room: 330.0,
                marker_x: Some(x(&c.time)),
                ..Default::default()
            },
        );
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let apply = ui
                .add(theme::primary("Apply to current scene"))
                .on_hover_text("Wind sea, swell, wind, current and sea temperature at this hour");
            crate::studio::a11y::name(&apply, "Apply forecast to current scene");
            if apply.clicked() {
                self.apply_sea = Some((c.clone(), forecast::SOURCE.to_string()));
            }
            if ui
                .add_enabled(
                    self.building.is_none(),
                    egui::Button::new("Create scene here"),
                )
                .on_hover_text("Real seabed at this point with this hour's sea")
                .clicked()
            {
                let (lat, lon) = (f.latitude, f.longitude);
                self.start_build_with(lat, lon, Some((c.clone(), forecast::SOURCE.to_string())));
            }
        });
        ui.label(
            RichText::new(forecast::ATTRIBUTION)
                .small()
                .color(theme::MUTED),
        );
        ui.label(
            RichText::new("https://open-meteo.com")
                .small()
                .color(theme::ACCENT),
        );
    }
}

/// Compass point for a bearing.
fn compass(degrees: f64) -> String {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let i = ((degrees.rem_euclid(360.0) / 22.5).round() as usize) % 16;
    POINTS[i].to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn hours_and_compass_points() {
        assert_eq!(
            super::hours_between("2026-11-23T00:00", "2026-11-24T06:00"),
            Some(30)
        );
        assert_eq!(
            super::hours_between("2024-02-28T23:00", "2024-03-01T01:00"),
            Some(26)
        );
        assert_eq!(super::compass(225.0), "SW");
        assert_eq!(super::compass(359.0), "N");
        let low = super::height_colour(0.5);
        let high = super::height_colour(8.0);
        assert!(high.r() as u32 + high.g() as u32 > low.r() as u32 + low.g() as u32);
    }
}
