//! Details panel when nothing is selected: a scene overview with the
//! environment at a glance, what the scene contains and quick actions.
use super::{State, header, section, theme};
use egui::{RichText, Ui};

fn value(ui: &mut Ui, label: &str, text: String) {
    ui.add(egui::Label::new(RichText::new(label).color(theme::MUTED)).wrap());
    ui.add(egui::Label::new(RichText::new(text).color(theme::STRONG)).wrap());
    ui.end_row();
}

/// Douglas sea-state code for a significant wave height (m).
fn sea_state(hs: f64) -> &'static str {
    match hs {
        h if h <= 0.0 => "calm (glassy)",
        h if h <= 0.1 => "calm (rippled)",
        h if h <= 0.5 => "smooth",
        h if h <= 1.25 => "slight",
        h if h <= 2.5 => "moderate",
        h if h <= 4.0 => "rough",
        h if h <= 6.0 => "very rough",
        h if h <= 9.0 => "high",
        h if h <= 14.0 => "very high",
        _ => "phenomenal",
    }
}

fn bearing_text(b: f64) -> String {
    format!("{:.0}° {}", b.rem_euclid(360.0), super::compass_point(b))
}

impl State {
    /// What is happening at this real place now: observations and their
    /// sources, sun and moon, maritime zones, nearest land, ships, species.
    fn here_and_now(&mut self, ui: &mut Ui, scene: &crate::project::Scene) {
        let geo = super::super::world::Geo {
            lat: scene.provenance.origin_latitude,
            lon: scene.provenance.origin_longitude,
        };
        let (lat, lon) = geo.to_latlon(self.cam.eye.x, self.cam.eye.y);
        let live = self.live.clone();
        let loading = self.live_rx.is_some();
        let mode = if self.live_on {
            "Live".to_string()
        } else if let Some(h) = self.forecast_ahead {
            format!("Forecast +{h} h")
        } else {
            "Manual (Live is off)".to_string()
        };
        section(ui, "Here and now", |ui| {
            value(
                ui,
                "Position",
                format!(
                    "{:.4}° {} · {:.4}° {}",
                    lat.abs(),
                    if lat >= 0.0 { "N" } else { "S" },
                    lon.abs(),
                    if lon >= 0.0 { "E" } else { "W" }
                ),
            );
            value(ui, "Showing", mode);
            match &live {
                None if loading => value(ui, "Conditions", "loading observations…".into()),
                None => value(
                    ui,
                    "Conditions",
                    "turn on ● Live on the viewport bar".into(),
                ),
                Some(l) => {
                    if !l.regions.is_empty() {
                        value(ui, "Waters", l.regions.join("\n"));
                    }
                    if !l.local_time.is_empty() {
                        value(
                            ui,
                            "Local time",
                            format!("{} ({})", l.local_time, l.timezone),
                        );
                    }
                    let mut weather = vec![];
                    if let Some(code) = l.weather_code {
                        weather.push(super::super::live::weather_words(code).to_string());
                    }
                    if let Some(t) = l.metar.as_ref().and_then(|m| m.temp).or(l.air_temp) {
                        weather.push(format!("{t:.0} °C air"));
                    }
                    if let Some(c) = l.cloud_cover() {
                        weather.push(format!("{:.0}% cloud", c * 100.0));
                    }
                    if let Some(p) = l.precipitation.filter(|p| *p > 0.0) {
                        weather.push(format!("{p:.1} mm rain"));
                    }
                    value(ui, "Weather", weather.join(" · "));
                    let vis = l
                        .metar
                        .as_ref()
                        .and_then(|m| m.visibility_m)
                        .or(l.visibility_m);
                    if let Some(v) = vis {
                        value(ui, "Visibility", format!("{:.1} km", v / 1000.0));
                    }
                    let (wind, wfrom, wsrc) = match &l.metar {
                        Some(m) if m.wind_ms.is_some() => (
                            m.wind_ms,
                            m.wind_from,
                            format!(
                                "observed at {} ({:.0} km, {} min ago)",
                                m.station, m.km, m.age_min
                            ),
                        ),
                        _ => (l.wind_ms, l.wind_from, "model, now".to_string()),
                    };
                    if let Some(w) = wind {
                        value(
                            ui,
                            "Wind",
                            format!(
                                "{w:.1} m/s ({:.0} kn) from {} · {wsrc}",
                                w * 1.943_844,
                                wfrom.map_or("—".to_string(), bearing_text)
                            ),
                        );
                    }
                    match &l.buoy {
                        Some((km, b)) => value(
                            ui,
                            "Waves (observed)",
                            format!(
                                "Hs {:.1} m · {} s · from {} · NOAA buoy {} ({km:.0} km, {})",
                                b.wave_height.unwrap_or(0.0),
                                b.period.map_or("—".to_string(), |p| format!("{p:.0}")),
                                b.wave_from.map_or("—".to_string(), bearing_text),
                                b.id,
                                b.time
                            ),
                        ),
                        None => {
                            if let Some(h) = l.wave_height {
                                value(
                                    ui,
                                    "Waves (model, now)",
                                    format!(
                                        "Hs {h:.1} m · {} s · from {}",
                                        l.wave_period
                                            .map_or("—".to_string(), |p| format!("{p:.1}")),
                                        l.wave_from.map_or("—".to_string(), bearing_text)
                                    ),
                                );
                            }
                        }
                    }
                    if let Some(h) = l.swell_height {
                        value(
                            ui,
                            "Swell",
                            format!(
                                "{h:.1} m · {} s · from {}",
                                l.swell_period
                                    .map_or("—".to_string(), |p| format!("{p:.0}")),
                                l.swell_from.map_or("—".to_string(), bearing_text)
                            ),
                        );
                    }
                    if let Some(t) = l.buoy.as_ref().and_then(|b| b.1.water_temp).or(l.sst) {
                        value(ui, "Sea temperature", format!("{t:.1} °C"));
                    }
                    if let Some(c) = l.current_ms {
                        value(
                            ui,
                            "Current",
                            format!(
                                "{c:.2} m/s ({:.1} kn) towards {}",
                                c * 1.943_844,
                                l.current_to.map_or("—".to_string(), bearing_text)
                            ),
                        );
                    }
                    let unix = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64);
                    let (day, _) = super::super::globe::utc_day_hours(unix + l.utc_offset_s);
                    value(
                        ui,
                        "Sun",
                        match super::super::live::sun_times(lat, lon, day, l.utc_offset_s) {
                            Some((rise, set)) => format!("rises {rise} · sets {set} (local)"),
                            None => "up all day or down all day (polar)".into(),
                        },
                    );
                    let (phase, lit) = super::super::live::moon_phase(unix);
                    value(ui, "Moon", format!("{phase} · {:.0}% lit", lit * 100.0));
                }
            }
            if let Some((d, x, y)) = self.nearest_land {
                let b = (x - self.cam.eye.x).atan2(y - self.cam.eye.y).to_degrees();
                value(
                    ui,
                    "Nearest land",
                    format!("{:.1} km {}", d / 1000.0, bearing_text(b)),
                );
            }
            if self.live_on {
                let near = self.ships.len();
                let status = if self.globe.marine.ais_key.is_empty() {
                    "add a free aisstream.io key (Earth → Layers) for ships outside the Baltic"
                        .to_string()
                } else {
                    self.globe.marine.world_text()
                };
                value(
                    ui,
                    "Ships (AIS)",
                    if near == 0 {
                        format!("none reporting within 80 km yet · {status}")
                    } else {
                        let names: Vec<&str> =
                            self.ships.iter().take(3).map(|s| s.name.as_str()).collect();
                        format!("{near} within 80 km: {}", names.join(", "))
                    },
                );
            }
            if !self.fauna.species.is_empty() {
                let names: Vec<String> = self
                    .fauna
                    .species
                    .iter()
                    .take(6)
                    .map(|s| s.common.clone())
                    .collect();
                value(
                    ui,
                    "Species here",
                    if self.fauna_recorded > 0 {
                        format!(
                            "{} recorded (OBIS); shown: {}",
                            self.fauna_recorded,
                            names.join(", ")
                        )
                    } else {
                        format!("representative: {}", names.join(", "))
                    },
                );
            }
        });
        ui.label(
            RichText::new("Observations: NOAA NDBC buoys, airport weather reports (aviationweather.gov METAR); otherwise model values for now: Open-Meteo (CC BY 4.0). Waters: Marine Regions (VLIZ, CC BY 4.0). Species: OBIS. Ships: AIS (Digitraffic, aisstream.io).")
                .small()
                .color(theme::MUTED),
        );
    }

    pub(super) fn scene_overview(&mut self, ui: &mut Ui) {
        let scene = self.scene().clone();
        header(ui, "◆", &scene.name, "Scene overview");
        if super::real_place(&scene) {
            self.here_and_now(ui, &scene);
        }
        section(ui, "Environment", |ui| {
            value(ui, "Water depth", format!("{:.0} m", scene.bottom_depth));
            value(
                ui,
                "Sea state",
                format!(
                    "Hs {:.1} m · {:.1} s · {}",
                    scene.wave_height,
                    scene.wave_period,
                    sea_state(scene.wave_height)
                ),
            );
            let current = scene.current.iter().map(|c| c * c).sum::<f64>().sqrt();
            value(
                ui,
                "Current",
                format!("{current:.2} m/s ({:.1} kn)", current * 1.943_844),
            );
            value(
                ui,
                "Water temperature",
                format!(
                    "{:.1} °C surface · {:.1} °C deep",
                    scene.temperature, scene.deep_temperature
                ),
            );
            value(
                ui,
                "Optical attenuation",
                format!("{:.3} /m", scene.attenuation),
            );
            value(ui, "Sun elevation", format!("{:.0}°", scene.sun_elevation));
        });
        section(ui, "Contents", |ui| {
            let bodies = scene.entities.len() + usize::from(scene.vehicle);
            value(ui, "Vehicles and bodies", bodies.to_string());
            value(
                ui,
                "Marine life",
                if self.fauna.species.is_empty() {
                    format!("{} simulated fish", scene.school.count)
                } else {
                    format!(
                        "{} species · {} animals around the camera",
                        self.fauna.species.len(),
                        self.fauna.len()
                    )
                },
            );
            value(ui, "Tracers", scene.particles.to_string());
            value(
                ui,
                "Structures",
                if scene.structure {
                    "harbour structure"
                } else {
                    "none"
                }
                .into(),
            );
            value(
                ui,
                "Library",
                format!(
                    "{} meshes · {} materials",
                    scene.library.meshes.len(),
                    scene.library.materials.len()
                ),
            );
            value(ui, "Origin", scene.provenance.classification.clone());
        });
        section(ui, "Quick actions", |ui| {
            ui.label("");
            ui.vertical(|ui| {
                if ui
                    .add(theme::primary("Import vehicle definition…"))
                    .on_hover_text("Bring a prototype in from a .pelagic-vehicle.json file")
                    .clicked()
                {
                    self.import_vehicle();
                }
                if ui
                    .button("Run standard trials")
                    .on_hover_text("Select a vehicle first: decay, speed, turning, zig-zag, waves")
                    .clicked()
                {
                    self.focus_trials = true;
                }
                if ui
                    .button("Seakeeping and live seas")
                    .on_hover_text(
                        "Motion limits in waves and a go/no-go outlook from the live forecast",
                    )
                    .clicked()
                {
                    self.focus_seakeeping = true;
                }
                if ui
                    .button("Search and rescue planning")
                    .on_hover_text(
                        "Drift of a person or craft from its last known position and the best search plan",
                    )
                    .clicked()
                {
                    self.focus_sar = true;
                }
                if ui
                    .button("Plan a seabed survey")
                    .on_hover_text("Multibeam or side-scan lines, coverage and IHO S-44 checks")
                    .clicked()
                {
                    self.focus_survey = true;
                }
                if ui
                    .button("Ship handling and COLREGs")
                    .on_hover_text("Steer a surface vessel through encounters under the collision regulations")
                    .clicked()
                {
                    self.focus_ship = true;
                }
                if ui
                    .button("Hull and harbour inspection")
                    .on_hover_text("Camera passes over pilings or a hull, imaged to a sharpness requirement")
                    .clicked()
                {
                    self.focus_inspection = true;
                }
                if ui
                    .button("Oil spill drift")
                    .on_hover_text("Where a spill drifts, how much evaporates and where it comes ashore")
                    .clicked()
                {
                    self.focus_spill = true;
                }
                if ui.button("Run a test campaign").clicked() {
                    self.focus_experiments = true;
                }
                if ui.button("Open the Sensor Monitor").clicked() {
                    self.focus_sensors = true;
                }
                if ui.button("Pick a real-world location").clicked() {
                    self.globe.focus = true;
                }
            });
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("Select an actor to view its details.")
                .small()
                .color(theme::MUTED),
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn douglas_sea_state_bands() {
        use super::sea_state;
        assert_eq!(sea_state(0.0), "calm (glassy)");
        assert_eq!(sea_state(1.8), "moderate");
        assert_eq!(sea_state(4.5), "very rough");
        assert_eq!(sea_state(20.0), "phenomenal");
    }
}
