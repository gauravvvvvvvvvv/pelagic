//! What the sea is doing at a place right now: observations first (nearest
//! NOAA buoy for waves and water, nearest airport weather report (METAR)
//! for wind, visibility and cloud), filled in by the forecast models' value
//! for the present moment (Open-Meteo "current", 15-minute steps), plus the
//! place's maritime regions (Marine Regions), local time, sun and moon.
//! Forecasts for later hours stay in the Forecast panel.
use serde_json::Value;

/// An airport weather report.
#[derive(Clone, Debug, PartialEq)]
pub struct Metar {
    pub station: String,
    pub name: String,
    pub km: f64,
    pub age_min: i64,
    pub wind_ms: Option<f64>,
    pub wind_from: Option<f64>,
    pub visibility_m: Option<f64>,
    pub cloud: Option<f64>,
    pub temp: Option<f64>,
    pub raw: String,
}

#[derive(Clone, Debug, Default)]
pub struct LiveNow {
    /// Local time of the model values and the zone.
    pub local_time: String,
    pub timezone: String,
    pub utc_offset_s: i64,
    pub wave_height: Option<f64>,
    pub wave_period: Option<f64>,
    pub wave_from: Option<f64>,
    pub swell_height: Option<f64>,
    pub swell_period: Option<f64>,
    pub swell_from: Option<f64>,
    pub current_ms: Option<f64>,
    pub current_to: Option<f64>,
    pub sst: Option<f64>,
    pub air_temp: Option<f64>,
    pub wind_ms: Option<f64>,
    pub wind_from: Option<f64>,
    pub cloud: Option<f64>,
    pub visibility_m: Option<f64>,
    pub precipitation: Option<f64>,
    pub weather_code: Option<i64>,
    pub buoy: Option<(f64, super::globe::marine::Buoy)>,
    pub metar: Option<Metar>,
    pub regions: Vec<String>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(25)))
        .user_agent("Pelagic-ocean-engine/1.0")
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .proxy(ureq::Proxy::try_from_env())
        .build()
        .into()
}

fn get(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    let mut r = agent.get(url).call().map_err(|e| e.to_string())?;
    let text = r
        .body_mut()
        .with_config()
        .limit(16 << 20)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (la1, la2) = (a.0.to_radians(), b.0.to_radians());
    let h = ((la2 - la1) * 0.5).sin().powi(2)
        + la1.cos() * la2.cos() * ((b.1 - a.1).to_radians() * 0.5).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

fn cover_fraction(code: &str) -> Option<f64> {
    Some(match code {
        "SKC" | "CLR" | "NSC" | "NCD" | "CAVOK" => 0.0,
        "FEW" => 0.19,
        "SCT" => 0.44,
        "BKN" => 0.75,
        "OVC" | "OVX" => 1.0,
        _ => return None,
    })
}

/// Parses aviationweather.gov METAR JSON; returns the report nearest to
/// (lat, lon).
pub fn parse_metar(v: &Value, lat: f64, lon: f64, now: i64) -> Option<Metar> {
    v.as_array()?
        .iter()
        .filter_map(|m| {
            let (la, lo) = (m["lat"].as_f64()?, m["lon"].as_f64()?);
            let cover = m["clouds"]
                .as_array()
                .and_then(|c| {
                    c.iter()
                        .filter_map(|l| cover_fraction(l["cover"].as_str()?))
                        .reduce(f64::max)
                })
                .or_else(|| cover_fraction(m["cover"].as_str()?));
            // Visibility in statute miles, "10+" style strings allowed.
            let vis = m["visib"]
                .as_f64()
                .or_else(|| m["visib"].as_str()?.trim_end_matches('+').parse().ok())
                .map(|mi| mi * 1609.34);
            Some(Metar {
                station: m["icaoId"].as_str()?.to_string(),
                name: m["name"].as_str().unwrap_or("").to_string(),
                km: km((lat, lon), (la, lo)),
                age_min: (now - m["obsTime"].as_i64().unwrap_or(now)) / 60,
                wind_ms: m["wspd"].as_f64().map(|kt| kt * 0.514_444),
                wind_from: m["wdir"].as_f64(),
                visibility_m: vis,
                cloud: cover,
                temp: m["temp"].as_f64(),
                raw: m["rawOb"].as_str().unwrap_or("").to_string(),
            })
        })
        .min_by(|a, b| a.km.total_cmp(&b.km))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Live conditions at a place (several requests; call off the UI thread).
pub fn fetch(lat: f64, lon: f64) -> Result<LiveNow, String> {
    let agent = agent();
    let now = now_unix();
    let mut live = LiveNow::default();
    let marine = get(
        &agent,
        &format!(
            "https://marine-api.open-meteo.com/v1/marine?latitude={lat:.4}&longitude={lon:.4}&current=wave_height,wave_direction,wave_period,swell_wave_height,swell_wave_period,swell_wave_direction,ocean_current_velocity,ocean_current_direction,sea_surface_temperature&cell_selection=sea"
        ),
    );
    if let Ok(m) = &marine {
        let c = &m["current"];
        live.wave_height = c["wave_height"].as_f64();
        live.wave_period = c["wave_period"].as_f64();
        live.wave_from = c["wave_direction"].as_f64();
        live.swell_height = c["swell_wave_height"].as_f64();
        live.swell_period = c["swell_wave_period"].as_f64();
        live.swell_from = c["swell_wave_direction"].as_f64();
        live.current_ms = c["ocean_current_velocity"].as_f64().map(|k| k / 3.6);
        live.current_to = c["ocean_current_direction"].as_f64();
        live.sst = c["sea_surface_temperature"].as_f64();
    }
    let weather = get(
        &agent,
        &format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}&current=temperature_2m,wind_speed_10m,wind_direction_10m,cloud_cover,visibility,precipitation,weather_code&timezone=auto&wind_speed_unit=ms"
        ),
    );
    if let Ok(w) = &weather {
        let c = &w["current"];
        live.local_time = c["time"].as_str().unwrap_or("").replace('T', " ");
        live.timezone = w["timezone"].as_str().unwrap_or("").to_string();
        live.utc_offset_s = w["utc_offset_seconds"].as_i64().unwrap_or(0);
        live.air_temp = c["temperature_2m"].as_f64();
        live.wind_ms = c["wind_speed_10m"].as_f64();
        live.wind_from = c["wind_direction_10m"].as_f64();
        live.cloud = c["cloud_cover"].as_f64().map(|p| p / 100.0);
        live.visibility_m = c["visibility"].as_f64();
        live.precipitation = c["precipitation"].as_f64();
        live.weather_code = c["weather_code"].as_i64();
    }
    if marine.is_err() && weather.is_err() {
        return Err(marine.err().unwrap_or_default());
    }
    // Observations: nearest airport within 150 km, nearest buoy within 200 km.
    let d = 1.5;
    if let Ok(v) = get(
        &agent,
        &format!(
            "https://aviationweather.gov/api/data/metar?bbox={:.2},{:.2},{:.2},{:.2}&format=json",
            lat - d,
            lon - d,
            lat + d,
            lon + d
        ),
    ) {
        live.metar = parse_metar(&v, lat, lon, now).filter(|m| m.km < 150.0 && m.age_min < 180);
    }
    if let Some(buoys) = super::globe::marine::latest_buoys() {
        live.buoy = buoys
            .into_iter()
            .map(|b| (km((lat, lon), (b.lat, b.lon)), b))
            .filter(|(k, b)| *k < 200.0 && b.wave_height.is_some())
            .min_by(|a, b| a.0.total_cmp(&b.0));
    }
    if let Ok(v) = get(
        &agent,
        &format!(
            "https://www.marineregions.org/rest/getGazetteerRecordsByLatLong.json/{lat:.4}/{lon:.4}/"
        ),
    ) && let Some(list) = v.as_array()
    {
        for r in list {
            let name = r["preferredGazetteerName"].as_str().unwrap_or("");
            let kind = r["placeType"].as_str().unwrap_or("");
            if !name.is_empty() && !live.regions.iter().any(|x| x.starts_with(name)) {
                live.regions.push(format!("{name} ({kind})"));
            }
        }
        live.regions.truncate(6);
    }
    Ok(live)
}

impl LiveNow {
    /// Sea conditions to apply to a scene: observed where an observation
    /// exists, else the model value for now.
    pub fn conditions(&self) -> (pelagic::metocean::Conditions, String) {
        let mut c = pelagic::metocean::Conditions {
            time: crate::forecast::now_minute(),
            wave_height: self.wave_height,
            wave_period: self.wave_period,
            wave_direction: self.wave_from,
            swell_height: self.swell_height,
            swell_period: self.swell_period,
            swell_direction: self.swell_from,
            current_speed: self.current_ms,
            current_direction: self.current_to,
            sea_temperature: self.sst,
            wind_speed: self.wind_ms,
            wind_direction: self.wind_from,
            ..Default::default()
        };
        let mut sources = vec!["model now (Open-Meteo)".to_string()];
        if let Some((km, b)) = &self.buoy {
            c.wave_height = b.wave_height.or(c.wave_height);
            c.wave_period = b.period.or(c.wave_period);
            c.wave_direction = b.wave_from.or(c.wave_direction);
            c.sea_temperature = b.water_temp.or(c.sea_temperature);
            sources.push(format!("waves observed at NOAA buoy {} ({km:.0} km)", b.id));
        }
        if let Some(m) = &self.metar {
            if m.wind_ms.is_some() {
                c.wind_speed = m.wind_ms;
                c.wind_direction = m.wind_from.or(c.wind_direction);
            }
            sources.push(format!(
                "wind observed at {} ({:.0} km, {} min ago)",
                m.station, m.km, m.age_min
            ));
        }
        (c, format!("Live: {}", sources.join(" · ")))
    }
    /// Cloud cover 0-1: observed (METAR) else model.
    pub fn cloud_cover(&self) -> Option<f64> {
        self.metar.as_ref().and_then(|m| m.cloud).or(self.cloud)
    }
}

/// Sunrise and sunset (local "HH:MM") for a day, or None in polar day/night.
pub fn sun_times(lat: f64, lon: f64, day: f64, utc_offset_s: i64) -> Option<(String, String)> {
    let elevation = |h: f64| pelagic_render_gpu::sky::solar_position(lat, lon, day, h).0;
    let local_noon = (12.0 - lon / 15.0).rem_euclid(24.0);
    let cross = |from: f64, to: f64| {
        let (mut a, mut b) = (from, to);
        let up = elevation(b) > elevation(a);
        for _ in 0..40 {
            let m = 0.5 * (a + b);
            if (elevation(m) > -0.833) == up {
                b = m;
            } else {
                a = m;
            }
        }
        0.5 * (a + b)
    };
    if elevation(local_noon) < -0.833 || elevation(local_noon + 12.0) > -0.833 {
        return None;
    }
    let rise = cross(local_noon - 12.0, local_noon);
    let set = cross(local_noon, local_noon + 12.0);
    let fmt = |utc: f64| {
        let t = (utc + utc_offset_s as f64 / 3600.0).rem_euclid(24.0);
        format!("{:02}:{:02}", t as u32, ((t.fract()) * 60.0) as u32 % 60)
    };
    Some((fmt(rise), fmt(set)))
}

/// Moon phase name and illuminated fraction for a Unix time.
pub fn moon_phase(unix: i64) -> (&'static str, f64) {
    let synodic = 29.530_588_853;
    let days = (unix as f64 - 947_182_440.0) / 86_400.0; // new moon 2000-01-06 18:14 UTC
    let age = days.rem_euclid(synodic);
    let illum = 0.5 * (1.0 - (age / synodic * std::f64::consts::TAU).cos());
    let name = match age / synodic {
        f if f < 0.03 || f > 0.97 => "New moon",
        f if f < 0.22 => "Waxing crescent",
        f if f < 0.28 => "First quarter",
        f if f < 0.47 => "Waxing gibbous",
        f if f < 0.53 => "Full moon",
        f if f < 0.72 => "Waning gibbous",
        f if f < 0.78 => "Last quarter",
        _ => "Waning crescent",
    };
    (name, illum)
}

/// WMO weather code in words.
pub fn weather_words(code: i64) -> &'static str {
    match code {
        0 => "clear",
        1 => "mainly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "fog",
        51..=57 => "drizzle",
        61..=67 => "rain",
        71..=77 => "snow",
        80..=82 => "rain showers",
        85 | 86 => "snow showers",
        95..=99 => "thunderstorm",
        _ => "—",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metar_sun_and_moon() {
        let v: Value = serde_json::from_str(r#"[{"icaoId":"VOAT","obsTime":1795519800,"wdir":360,"wspd":5,"visib":3.73,"lat":10.824,"lon":72.176,"name":"Agatti Arpt","cover":"SCT","clouds":[{"cover":"SCT","base":1500}],"temp":31,"rawOb":"METAR VOAT"},{"icaoId":"FAR","lat":0.0,"lon":0.0,"obsTime":1795519800}]"#).unwrap();
        let m = parse_metar(&v, 10.55, 72.6, 1_795_520_400).unwrap();
        assert_eq!(m.station, "VOAT");
        assert_eq!(m.age_min, 10);
        assert!((m.wind_ms.unwrap() - 2.572).abs() < 0.01);
        assert!((m.visibility_m.unwrap() - 6003.0).abs() < 2.0);
        assert_eq!(m.cloud, Some(0.44));
        assert!(m.km > 40.0 && m.km < 60.0, "{}", m.km);
        // Equator at the March equinox, longitude 0: sunrise and sunset about
        // 06:05 and 18:10 UTC (equation of time about -7 min, refraction).
        let (rise, set) = sun_times(0.0, 0.0, 80.0, 0).unwrap();
        let minutes = |t: &str| {
            let (h, m) = t.split_once(':').unwrap();
            h.parse::<i32>().unwrap() * 60 + m.parse::<i32>().unwrap()
        };
        assert!((minutes(&rise) - 365).abs() <= 8, "{rise}");
        assert!((minutes(&set) - 610 - 480).abs() <= 8, "{set}");
        assert!(sun_times(85.0, 0.0, 172.0, 0).is_none(), "midnight sun");
        // 2000-01-21 04:40 UTC was a full moon.
        let (name, illum) = moon_phase(948_429_600);
        assert_eq!(name, "Full moon");
        assert!(illum > 0.95);
        assert_eq!(weather_words(95), "thunderstorm");
    }
}
