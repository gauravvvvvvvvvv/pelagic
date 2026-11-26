//! Oil spill drift in the editor: release oil at a point, follow it under
//! the live wind and current (or steady ones), with evaporation and the
//! scene's coastline, on a map with a time slider and a mass balance.
use super::{State, native, theme};
use crate::jobs::{Event, Output};
use crate::studio::charts::{Options, SERIES, SURFACE, Series, chart};
use crate::studio::globe::SarOverlay;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, Vec2};
use pelagic::sar::{Drivers, Plane};
use pelagic::spill::{self, Balance, Land, PRODUCTS, Spill, SpillSetup};
use std::sync::Arc;

pub(super) struct Panel {
    setup: SpillSetup,
    /// The release began this many hours ago (negative: in the future).
    ago_h: f64,
    steady: bool,
    wind: [f64; 2],
    current: [f64; 2],
    /// Strand on the scene's coastline (real-world scenes).
    coastline: bool,
    result: Option<Arc<Spill>>,
    balances: Vec<Balance>,
    hour: f64,
    playing: bool,
    view: Option<(f64, [f64; 2])>,
    overlay: Option<(u64, Arc<SarOverlay>)>,
    fold: bool,
}
impl Default for Panel {
    fn default() -> Self {
        Self {
            setup: SpillSetup {
                latitude: 15.2,
                longitude: 73.2,
                ..Default::default()
            },
            ago_h: 0.0,
            steady: false,
            wind: [8.0, 270.0],
            current: [0.3, 180.0],
            coastline: true,
            result: None,
            balances: vec![],
            hour: 0.0,
            playing: false,
            view: None,
            overlay: None,
            fold: false,
        }
    }
}

const OIL: Color32 = Color32::from_rgb(0xd9, 0x59, 0x26);

impl Panel {
    pub(super) fn set_position(&mut self, lat: f64, lon: f64) {
        self.setup.latitude = lat.clamp(-80.0, 80.0);
        self.setup.longitude = (lon + 540.0).rem_euclid(360.0) - 180.0;
    }
    pub(super) fn set_steady(&mut self, wind: [f64; 2], current: [f64; 2]) {
        self.steady = true;
        self.wind = wind;
        self.current = current;
    }
    pub(super) fn show_hour(&mut self, hours: f64) {
        if let Some(s) = &self.result {
            self.hour = (s.first_hour + hours).clamp(s.first_hour, s.last_hour());
        }
    }
    /// Floating and stranded oil for the Earth view.
    pub(super) fn overlay(&mut self) -> Option<Arc<SarOverlay>> {
        let s = self.result.clone()?;
        let key = (self.hour * 60.0).round() as u64;
        if let Some((k, o)) = &self.overlay
            && *k == key
        {
            return Some(o.clone());
        }
        let particles = (0..s.parcels())
            .filter(|i| s.state(*i, self.hour) > 0)
            .map(|i| {
                let (lat, lon) = s.plane.geo(s.position(i, self.hour));
                (lat, lon, 1.0 - s.evaporated(i, self.hour) as f32)
            })
            .collect();
        let o = Arc::new(SarOverlay {
            lkp: (s.setup.latitude, s.setup.longitude),
            particles,
            track: vec![],
            area: vec![],
            colour: (40, 28, 20),
        });
        self.overlay = Some((key, o.clone()));
        Some(o)
    }
}

impl State {
    pub(super) fn spill_finished(&mut self, s: Spill) {
        let end = s.balance(s.last_hour());
        self.log(format!(
            "Spill drift ready: after {:.0} h {:.1} m³ afloat, {:.1} m³ evaporated, {:.1} m³ ashore.",
            s.setup.duration_h, end.surface_m3, end.evaporated_m3, end.stranded_m3
        ));
        for n in &s.notes {
            self.log(format!("Spill: {n}"));
        }
        let p = &mut self.spill;
        p.balances = s.balances();
        p.hour = s.last_hour();
        p.view = None;
        p.overlay = None;
        p.playing = false;
        p.fold = true;
        p.result = Some(Arc::new(s));
    }

    pub(super) fn start_spill(&mut self) {
        let p = &self.spill;
        let mut setup = p.setup.clone();
        if let Err(e) = setup.validate() {
            return self.log(e);
        }
        let (ago, steady) = (
            p.ago_h,
            p.steady
                .then_some([p.wind[0], p.wind[1], p.current[0], p.current[1]]),
        );
        // Coastline from a real-world scene's seabed around the release.
        let scene = self.scene();
        let o = &scene.provenance;
        let land =
            (p.coastline && (o.origin_latitude != 0.0 || o.origin_longitude != 0.0)).then(|| {
                Land {
                    grid: pelagic::survey::seabed(scene),
                    offset: Plane {
                        latitude: o.origin_latitude,
                        longitude: o.origin_longitude,
                    }
                    .local(setup.latitude, setup.longitude),
                }
            });
        self.spill.result = None;
        self.spill.overlay = None;
        self.start_job("Drifting the spill…", move |cancel, send| {
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
            setup.release_hour = now_h - ago;
            let room = (drivers.hours - 1) as f64 - setup.release_hour;
            if room < 0.5 {
                return Err("The release is outside the forecast period.".into());
            }
            setup.duration_h = setup.duration_h.min(room);
            let _ = send.send(Event::Progress("Drifting…".into()));
            Ok(Output::Spill(Box::new(spill::run(
                &drivers,
                &setup,
                land.as_ref(),
                &cancel,
            )?)))
        });
    }

    fn export_spill(&mut self) {
        let Some(s) = self.spill.result.clone() else {
            return;
        };
        let Some(parent) = native::pick_folder(self.hwnd, "Choose a folder for the spill") else {
            return;
        };
        let folder = std::path::Path::new(&parent).join(format!(
            "Pelagic-spill-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        match s.write(&folder) {
            Ok(()) => self.log(format!(
                "Spill saved (summary, mass balance, particles, GeoJSON): {}",
                folder.display()
            )),
            Err(e) => self.log(format!("Could not save the spill: {e}")),
        }
    }

    pub(super) fn spill_panel(&mut self, ui: &mut Ui) {
        ui.columns(2, |columns| {
            let [left, right] = columns else {
                unreachable!()
            };
            egui::ScrollArea::vertical()
                .id_salt("spill-left")
                .show(left, |ui| self.spill_controls(ui));
            let room = right.available_height();
            egui::ScrollArea::vertical()
                .id_salt("spill-right")
                .show(right, |ui| self.spill_results(ui, room));
        });
    }

    fn spill_controls(&mut self, ui: &mut Ui) {
        let busy = self.job.is_some();
        ui.add_space(4.0);
        theme::heading(ui, "Oil spill drift");
        ui.label(
            RichText::new(
                "Where a surface oil spill drifts under the live wind and current, how much \
                 evaporates, and where and when it reaches the shore.",
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
            let run = ui.add_enabled(!busy, theme::primary("▶ Drift spill"));
            crate::studio::a11y::name(&run, "Drift spill");
            if run.clicked() {
                self.start_spill();
            }
            if self.spill.result.is_some()
                && ui
                    .add_enabled(!busy, egui::Button::new("💾 Export…"))
                    .on_hover_text("summary.txt, mass_balance.csv, particles.csv, slick.geojson")
                    .clicked()
            {
                self.export_spill();
            }
        });
        let earth = self.globe.center();
        let origin = {
            let o = &self.scene().provenance;
            (o.origin_latitude != 0.0 || o.origin_longitude != 0.0)
                .then_some((o.origin_latitude, o.origin_longitude))
        };
        let temp = self.scene().temperature;
        let fold = std::mem::take(&mut self.spill.fold).then_some(false);
        egui::CollapsingHeader::new(RichText::new("Release").strong())
            .id_salt("spill-release")
            .default_open(true)
            .open(fold)
            .show(ui, |ui| {
                let p = &mut self.spill;
                egui::Grid::new("spill-grid")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Oil");
                        let name = PRODUCTS
                            .iter()
                            .find(|x| x.id == p.setup.product)
                            .map_or("Crude oil by distillation".to_string(), |x| x.name.to_string());
                        let combo = egui::ComboBox::from_id_salt("spill-oil")
                            .selected_text(name)
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for x in &PRODUCTS {
                                    ui.selectable_value(&mut p.setup.product, x.id.to_string(), x.name);
                                }
                                ui.selectable_value(
                                    &mut p.setup.product,
                                    "CUSTOM".to_string(),
                                    "Crude oil by distillation",
                                );
                            });
                        crate::studio::a11y::name(&combo.response, "Oil");
                        ui.end_row();
                        if p.setup.product == "CUSTOM" {
                            ui.label("Distilled at 180 °C");
                            ui.add(
                                egui::DragValue::new(&mut p.setup.percent_distilled)
                                    .range(0.0..=60.0)
                                    .speed(0.5)
                                    .suffix(" %"),
                            );
                            ui.end_row();
                        }
                        ui.label("Volume");
                        let r = ui.add(
                            egui::DragValue::new(&mut p.setup.volume_m3)
                                .range(0.01..=1.0e6)
                                .speed(1.0)
                                .suffix(" m³"),
                        );
                        crate::studio::a11y::name(&r, "Spill volume");
                        ui.end_row();
                        ui.label("Position");
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::DragValue::new(&mut p.setup.latitude)
                                    .range(-80.0..=80.0)
                                    .speed(0.001)
                                    .max_decimals(4)
                                    .suffix("° N"),
                            );
                            crate::studio::a11y::name(&r, "Spill latitude");
                            let r = ui.add(
                                egui::DragValue::new(&mut p.setup.longitude)
                                    .range(-180.0..=180.0)
                                    .speed(0.001)
                                    .max_decimals(4)
                                    .suffix("° E"),
                            );
                            crate::studio::a11y::name(&r, "Spill longitude");
                        });
                        ui.end_row();
                        ui.label("");
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .small_button("Earth view centre")
                                .on_hover_text("Or right-click the sea on the Earth tab → Oil spill here")
                                .clicked()
                            {
                                p.setup.latitude = earth.0;
                                p.setup.longitude = earth.1;
                            }
                            if let Some((la, lo)) = origin
                                && ui.small_button("Scene location").clicked()
                            {
                                p.setup.latitude = la;
                                p.setup.longitude = lo;
                            }
                        });
                        ui.end_row();
                        ui.label("Released");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut p.ago_h)
                                    .range(-48.0..=46.0)
                                    .speed(0.1)
                                    .suffix(" h ago"),
                            )
                            .on_hover_text("Negative: a release in the future");
                            ui.add(
                                egui::DragValue::new(&mut p.setup.release_h)
                                    .range(0.0..=48.0)
                                    .speed(0.1)
                                    .prefix("over ")
                                    .suffix(" h"),
                            )
                            .on_hover_text("0: all at once; otherwise a continuous release");
                        });
                        ui.end_row();
                        ui.label("Follow for");
                        let r = ui.add(
                            egui::DragValue::new(&mut p.setup.duration_h)
                                .range(1.0..=96.0)
                                .speed(0.5)
                                .suffix(" h"),
                        );
                        crate::studio::a11y::name(&r, "Spill duration");
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
                                ui.add(egui::DragValue::new(&mut p.wind[0]).range(0.0..=40.0).speed(0.1).prefix("wind ").suffix(" m/s"));
                                ui.add(egui::DragValue::new(&mut p.wind[1]).range(0.0..=359.0).speed(1.0).prefix("from ").suffix("°"));
                            });
                            ui.end_row();
                            ui.label("");
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut p.current[0]).range(0.0..=4.0).speed(0.01).prefix("current ").suffix(" m/s"));
                                ui.add(egui::DragValue::new(&mut p.current[1]).range(0.0..=359.0).speed(1.0).prefix("towards ").suffix("°"));
                            });
                            ui.end_row();
                        }
                        ui.label("Coastline");
                        ui.checkbox(&mut p.coastline, "strand on this scene's shore")
                            .on_hover_text("Uses the seabed of a real-world scene (created on the Earth tab) around the release");
                        ui.end_row();
                    });
                egui::CollapsingHeader::new("Model settings")
                    .id_salt("spill-model")
                    .show(ui, |ui| {
                        egui::Grid::new("spill-model-grid")
                            .num_columns(2)
                            .spacing([10.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Wind drift");
                                let mut pct = p.setup.wind_factor * 100.0;
                                ui.add(egui::DragValue::new(&mut pct).range(0.0..=10.0).speed(0.1).suffix(" % of wind"));
                                p.setup.wind_factor = pct / 100.0;
                                ui.end_row();
                                ui.label("Deflection");
                                ui.add(egui::DragValue::new(&mut p.setup.deflection_deg).range(-45.0..=45.0).speed(0.5).suffix("° right of wind"));
                                ui.end_row();
                                ui.label("Diffusivity");
                                ui.add(egui::DragValue::new(&mut p.setup.diffusivity).range(0.0..=1000.0).speed(0.5).suffix(" m²/s"));
                                ui.end_row();
                                ui.label("Water temperature");
                                ui.horizontal(|ui| {
                                    ui.add(egui::DragValue::new(&mut p.setup.water_temp_c).range(-2.0..=40.0).speed(0.1).suffix(" °C"));
                                    if ui.small_button("Scene").clicked() {
                                        p.setup.water_temp_c = temp;
                                    }
                                });
                                ui.end_row();
                                ui.label("Parcels");
                                ui.add(egui::DragValue::new(&mut p.setup.particles).range(200..=20_000).speed(10.0));
                                ui.end_row();
                            });
                        ui.label(
                            RichText::new(
                                "The live current already includes wave (Stokes) drift, so 2–3 % of the wind is usual; open-ocean models often deflect the wind drift 0–15° to the right in the northern hemisphere.",
                            )
                            .small()
                            .color(theme::MUTED),
                        );
                    });
            });
        let Some(s) = self.spill.result.clone() else {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Set the release and press Drift spill. With a real-world scene open, the oil \
                     strands on its coastline.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        ui.add_space(6.0);
        theme::heading(ui, "Mass balance");
        let b = s.balance(self.spill.hour);
        ui.horizontal_wrapped(|ui| {
            theme::chip(ui, theme::MUTED, &format!("afloat {:.1} m³", b.surface_m3));
            theme::chip(
                ui,
                theme::MUTED,
                &format!("evaporated {:.1} m³", b.evaporated_m3),
            );
            theme::chip(
                ui,
                if b.stranded_m3 > 0.0 {
                    theme::BAD
                } else {
                    theme::GOOD
                },
                &format!("ashore {:.1} m³", b.stranded_m3),
            );
            theme::chip(
                ui,
                theme::MUTED,
                &format!("slick ≈ {:.2} km², {:.1} µm", b.area_km2, b.thickness_um),
            );
        });
        match s.first_landfall() {
            Some((h, p)) => {
                let (lat, lon) = s.plane.geo(p);
                ui.label(
                    RichText::new(format!(
                        "First oil ashore {:.1} h after the release ({} UTC) at {lat:.4}°, {lon:.4}°.",
                        h - s.first_hour,
                        s.time_of(h).replace('T', " ")
                    ))
                    .color(theme::WARN),
                );
            }
            None => {
                ui.label(RichText::new("No oil ashore in this period.").small());
            }
        }
        let x = |b: &Balance| b.hour - s.first_hour;
        chart(
            ui,
            "Oil (m³)",
            &[
                Series {
                    name: "Afloat",
                    colour: SERIES[0],
                    points: self
                        .spill
                        .balances
                        .iter()
                        .map(|b| [x(b), b.surface_m3])
                        .collect(),
                },
                Series {
                    name: "Evaporated",
                    colour: SERIES[1],
                    points: self
                        .spill
                        .balances
                        .iter()
                        .map(|b| [x(b), b.evaporated_m3])
                        .collect(),
                },
                Series {
                    name: "Ashore",
                    colour: SERIES[2],
                    points: self
                        .spill
                        .balances
                        .iter()
                        .map(|b| [x(b), b.stranded_m3])
                        .collect(),
                },
            ],
            "hours since the release",
            "m³",
            Options {
                origin: true,
                room: 240.0,
                marker_x: Some(self.spill.hour - s.first_hour),
                ..Default::default()
            },
        );
        for n in &s.notes {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("⚠").color(theme::WARN));
                ui.label(RichText::new(n).small());
            });
        }
        ui.label(
            RichText::new(format!(
                "{} · evaporation: Fingas empirical equations · {}",
                s.oil.0, s.source
            ))
            .small()
            .color(theme::MUTED),
        );
        if s.source.contains("Open-Meteo") {
            ui.label(
                RichText::new(crate::forecast::ATTRIBUTION)
                    .small()
                    .color(theme::MUTED),
            );
        }
    }

    fn spill_results(&mut self, ui: &mut Ui, room: f32) {
        ui.add_space(4.0);
        let Some(s) = self.spill.result.clone() else {
            theme::heading(ui, "Spill map");
            ui.label(
                RichText::new(
                    "Each dot is a parcel of oil: brown afloat (fainter as it evaporates), red \
                     ashore.",
                )
                .small()
                .color(theme::MUTED),
            );
            return;
        };
        let width = ui.available_width().max(240.0);
        let height = (width * 0.75)
            .clamp(240.0, 600.0)
            .min((room - 60.0).max(240.0));
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        crate::studio::a11y::name(&response, "Spill map");
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, SURFACE);
        let hour = self.spill.hour;
        let fit = || {
            let mut xs = vec![0.0];
            let mut ys = vec![0.0];
            for i in 0..s.parcels() {
                if s.state(i, s.last_hour()) > 0 {
                    let p = s.position(i, s.last_hour());
                    xs.push(p[0]);
                    ys.push(p[1]);
                }
            }
            xs.sort_by(f64::total_cmp);
            ys.sort_by(f64::total_cmp);
            let q = |v: &[f64], f: f64| v[((v.len() - 1) as f64 * f) as usize];
            let (x0, x1) = (q(&xs, 0.01).min(0.0), q(&xs, 0.99).max(0.0));
            let (y0, y1) = (q(&ys, 0.01).min(0.0), q(&ys, 0.99).max(0.0));
            let scale = ((x1 - x0) / (rect.width() as f64 - 48.0))
                .max((y1 - y0) / (rect.height() as f64 - 48.0))
                .max(0.5);
            (scale * 1.1, [(x0 + x1) * 0.5, (y0 + y1) * 0.5])
        };
        if self.spill.view.is_none() || response.double_clicked() {
            self.spill.view = Some(fit());
        }
        let (mut scale, mut centre) = self.spill.view.unwrap_or((1.0, [0.0, 0.0]));
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
                scale = (scale * (-scroll as f64 * 0.002).exp()).clamp(0.05, 50_000.0);
                centre = [
                    before[0] - (pointer.x - rect.center().x) as f64 * scale,
                    before[1] + (pointer.y - rect.center().y) as f64 * scale,
                ];
            }
        }
        self.spill.view = Some((scale, centre));
        let c = rect.center();
        let screen = |p: [f64; 2]| {
            Pos2::new(
                c.x + ((p[0] - centre[0]) / scale) as f32,
                c.y - ((p[1] - centre[1]) / scale) as f32,
            )
        };
        // Centre track of the floating oil.
        let track: Vec<Pos2> = self
            .spill
            .balances
            .iter()
            .filter(|b| b.hour <= hour && b.surface_m3 > 0.0)
            .map(|b| screen(b.centre))
            .collect();
        if track.len() > 1 {
            painter.add(Shape::line(
                track,
                Stroke::new(1.2, Color32::from_rgba_unmultiplied(230, 236, 242, 140)),
            ));
        }
        for i in 0..s.parcels() {
            let state = s.state(i, hour);
            if state == 0 {
                continue;
            }
            let q = screen(s.position(i, hour));
            if !rect.contains(q) {
                continue;
            }
            let colour = if state == 2 {
                theme::BAD
            } else {
                let left = 1.0 - s.evaporated(i, hour) as f32;
                Color32::from_rgba_unmultiplied(
                    OIL.r(),
                    OIL.g(),
                    OIL.b(),
                    (60.0 + 170.0 * left) as u8,
                )
            };
            painter.rect_filled(Rect::from_center_size(q, Vec2::splat(2.2)), 0.0, colour);
        }
        let r0 = screen([0.0, 0.0]);
        painter.circle_stroke(r0, 6.0, Stroke::new(2.0, theme::STRONG));
        painter.text(
            r0 + Vec2::new(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            "release",
            FontId::proportional(11.0),
            theme::STRONG,
        );
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
            .find(|v| *v >= raw)
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
        painter.text(
            Pos2::new(rect.left() + 10.0, rect.bottom() - 10.0),
            Align2::LEFT_BOTTOM,
            "drag to pan · scroll to zoom · double-click to fit",
            FontId::proportional(10.5),
            theme::MUTED,
        );
        ui.horizontal_wrapped(|ui| {
            for (colour, text) in [
                (OIL, "afloat (fainter as it evaporates)"),
                (theme::BAD, "ashore"),
            ] {
                let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, 10.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, colour);
                ui.label(RichText::new(text).small());
            }
            ui.label(RichText::new("— centre track").small().color(theme::MUTED));
        });
        let (first, last) = (s.first_hour, s.last_hour());
        ui.horizontal(|ui| {
            let b = ui.button(if self.spill.playing { "⏸" } else { "▶" });
            crate::studio::a11y::name(&b, "Play spill");
            if b.clicked() {
                if !self.spill.playing && self.spill.hour >= last - 1e-6 {
                    self.spill.hour = first;
                }
                self.spill.playing = !self.spill.playing;
            }
            ui.spacing_mut().slider_width = (ui.available_width() - 200.0).max(80.0);
            let r = ui.add(egui::Slider::new(&mut self.spill.hour, first..=last).show_value(false));
            crate::studio::a11y::name(&r, "Spill time");
            if r.dragged() {
                self.spill.playing = false;
            }
            ui.label(
                RichText::new(format!(
                    "{} UTC · +{:.1} h",
                    s.time_of(self.spill.hour).replace('T', " "),
                    self.spill.hour - first
                ))
                .small(),
            );
        });
        if self.spill.playing {
            let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
            self.spill.hour += dt * (last - first) / 20.0;
            if self.spill.hour >= last {
                self.spill.hour = last;
                self.spill.playing = false;
            }
            ui.ctx().request_repaint();
        }
    }
}
