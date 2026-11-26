//! Place card: click anywhere on the Earth view to see what is there — a
//! Sentinel-2 satellite view (about 2.4 km across, 10 m pixels), depth or
//! height, the live sea, the nearest port — and open it in 3D.
use super::{Globe, Layer, fetch, http_agent, theme};
use egui::{RichText, Vec2};
use std::sync::mpsc;

pub(super) struct PlaceData {
    image: Option<(Vec<u8>, usize, usize)>,
    sea: Vec<(String, String)>,
    port: Option<(String, f64)>,
    error: Option<String>,
}

pub(super) struct PlaceCard {
    pub lat: f64,
    pub lon: f64,
    rx: Option<mpsc::Receiver<PlaceData>>,
    data: Option<PlaceData>,
    texture: Option<egui::TextureHandle>,
}

/// Sentinel-2 mosaic of 3×3 zoom-15 tiles around a point, cropped to 512²
/// pixels centred on it.
fn imagery(lat: f64, lon: f64) -> Result<(Vec<u8>, usize, usize), String> {
    let z = 15u8;
    let n = 2f64.powi(z as i32);
    let fx = (lon + 180.0) / 360.0 * n;
    let r = lat.to_radians();
    let fy = (1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
    let (tx, ty) = (fx.floor() as i64, fy.floor() as i64);
    let agent = http_agent(30);
    let cache = crate::project::data_dir().join("tiles");
    let mut mosaic = vec![0u8; 768 * 768 * 4];
    for dy in -1..=1i64 {
        for dx in -1..=1i64 {
            let x = (tx + dx).rem_euclid(n as i64) as u32;
            let y = (ty + dy).clamp(0, n as i64 - 1) as u32;
            let bytes = fetch(&agent, &cache, (Layer::Sentinel, z, x, y))?;
            let mut decoder = zune_jpeg::JpegDecoder::new(&bytes);
            let rgb = decoder.decode().map_err(|e| format!("{e:?}"))?;
            let (w, h) = decoder.dimensions().ok_or("jpeg size")?;
            if w != 256 || h != 256 || rgb.len() != 256 * 256 * 3 {
                return Err("unexpected imagery tile".into());
            }
            let (ox, oy) = (((dx + 1) * 256) as usize, ((dy + 1) * 256) as usize);
            for row in 0..256 {
                for col in 0..256 {
                    let s = (row * 256 + col) * 3;
                    let d = ((oy + row) * 768 + ox + col) * 4;
                    mosaic[d..d + 4].copy_from_slice(&[rgb[s], rgb[s + 1], rgb[s + 2], 255]);
                }
            }
        }
    }
    let cx = ((fx - tx as f64 + 1.0) * 256.0) as usize;
    let cy = ((fy - ty as f64 + 1.0) * 256.0) as usize;
    let (x0, y0) = (
        cx.saturating_sub(256).min(256),
        cy.saturating_sub(256).min(256),
    );
    let mut out = Vec::with_capacity(512 * 512 * 4);
    for row in 0..512 {
        let s = ((y0 + row) * 768 + x0) * 4;
        out.extend_from_slice(&mosaic[s..s + 512 * 4]);
    }
    Ok((out, 512, 512))
}

fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2) = (a.0.to_radians(), b.0.to_radians());
    let dla = la2 - la1;
    let dlo = (b.1 - a.1).to_radians();
    let h = (dla * 0.5).sin().powi(2) + la1.cos() * la2.cos() * (dlo * 0.5).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

impl PlaceCard {
    pub fn new(lat: f64, lon: f64, water: bool) -> Self {
        let (send, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let image = imagery(lat, lon);
            let mut sea = vec![];
            if water && let Ok(f) = crate::forecast::point(lat, lon, 1) {
                let h = &f.hours[f.index_at(&crate::forecast::now_hour())];
                let opt = |v: Option<f64>, unit: &str| {
                    v.map_or("—".to_string(), |x| format!("{x:.1} {unit}"))
                };
                sea.push((
                    "Waves now".into(),
                    format!("{} · {}", opt(h.wave_height, "m"), opt(h.wave_period, "s")),
                ));
                sea.push(("Swell".into(), opt(h.swell_height, "m")));
                sea.push(("Wind".into(), opt(h.wind_speed, "m/s")));
                sea.push(("Current".into(), opt(h.current_speed, "m/s")));
                sea.push(("Sea temperature".into(), opt(h.sea_temperature, "°C")));
            }
            let port = super::marine::nearest_port(lat, lon)
                .map(|(name, plat, plon)| (name, haversine_km((lat, lon), (plat, plon))));
            let _ = send.send(PlaceData {
                error: image.as_ref().err().cloned(),
                image: image.ok(),
                sea,
                port,
            });
        });
        Self {
            lat,
            lon,
            rx: Some(rx),
            data: None,
            texture: None,
        }
    }
    pub fn loading(&self) -> bool {
        self.rx.is_some()
    }
}

impl Globe {
    pub(super) fn open_place(&mut self, lat: f64, lon: f64) {
        let water = self.elevation(lat, lon).is_none_or(|(e, _)| e < 0.0);
        self.place = Some(PlaceCard::new(lat, lon, water));
    }

    /// The place card window. Returns true when "Explore in 3D" was chosen.
    pub(super) fn place_window(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let Some(card) = self.place.as_mut() else {
            return;
        };
        if let Some(rx) = &card.rx
            && let Ok(data) = rx.try_recv()
        {
            card.rx = None;
            if let Some((rgba, w, h)) = &data.image {
                card.texture = Some(ctx.load_texture(
                    "place-imagery",
                    egui::ColorImage::from_rgba_unmultiplied([*w, *h], rgba),
                    egui::TextureOptions::LINEAR,
                ));
            }
            card.data = Some(data);
        }
        if card.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let (lat, lon) = (card.lat, card.lon);
        let height = self.elevation(lat, lon).map(|(e, _)| e);
        let mut open = true;
        let mut explore = false;
        let mut forecast = false;
        let mut vessel = false;
        let title = format!(
            "{:.4}°{} {:.4}°{}",
            lat.abs(),
            if lat >= 0.0 { "N" } else { "S" },
            lon.abs(),
            if lon >= 0.0 { "E" } else { "W" }
        );
        let building = self.building.is_some();
        egui::Window::new(RichText::new(format!("📍 {title}")).strong())
            .id(egui::Id::NULL.with("place-card"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(rect.right_top() + Vec2::new(-372.0, 56.0))
            .default_width(340.0)
            .show(ctx, |ui| {
                let card = self.place.as_ref().unwrap();
                match (&card.texture, &card.data) {
                    (Some(t), _) => {
                        ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(330.0)));
                        ui.label(RichText::new("Sentinel-2 cloudless (2016), ~2.4 km across").small().color(theme::MUTED));
                    }
                    (None, None) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Loading satellite view and live sea…");
                        });
                    }
                    (None, Some(d)) => {
                        ui.label(RichText::new(format!("Satellite view unavailable: {}", d.error.clone().unwrap_or_default())).small().color(theme::MUTED));
                    }
                }
                egui::Grid::new("place-grid").num_columns(2).show(ui, |ui| {
                    if let Some(e) = height {
                        ui.label(RichText::new(if e < 0.0 { "Depth" } else { "Height" }).color(theme::MUTED));
                        ui.label(format!("{:.0} m", e.abs()));
                        ui.end_row();
                    }
                    if let Some(d) = &card.data {
                        for (k, v) in &d.sea {
                            ui.label(RichText::new(k).color(theme::MUTED));
                            ui.label(v);
                            ui.end_row();
                        }
                        if let Some((name, km)) = &d.port {
                            ui.label(RichText::new("Nearest port").color(theme::MUTED));
                            ui.label(format!("{name} · {km:.0} km"));
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(6.0);
                let go = ui.add_enabled(
                    !building,
                    egui::Button::new(RichText::new("🌊 Explore in 3D").strong())
                        .fill(theme::ACCENT.gamma_multiply(0.55))
                        .min_size(Vec2::new(320.0, 30.0)),
                )
                .on_hover_text("Build this place in the 3D viewport from real seafloor, coast, reefs, live sea and sun. On land or in a port, the view is taken from the nearest open water.");
                crate::studio::a11y::name(&go, "Explore in 3D");
                explore = go.clicked();
                if building {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Building the 3D scene…");
                    });
                }
                ui.horizontal(|ui| {
                    forecast = ui.button("Sea forecast").clicked();
                    vessel = ui.button("Add my vessel here").clicked();
                });
            });
        if explore {
            self.start_build(lat, lon);
        }
        if forecast {
            self.sea.request_point(lat, lon);
        }
        if vessel {
            self.add_vessel_at(lat, lon);
        }
        if !open {
            self.place = None;
        }
    }
}
