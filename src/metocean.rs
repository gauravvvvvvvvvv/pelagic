//! Metocean conditions from forecasts: wind sea, swell, wind, surface current
//! and sea temperature at a location, hour by hour, and their use in scenes.
//! This module only parses and applies data; fetching it is the
//! application's job (see the desktop app's Open-Meteo client).
//!
//! Directions follow the data conventions: waves and wind come *from* a
//! compass bearing (degrees clockwise from true north), currents flow
//! *towards* one. Scenes use X east, Y north and directions of travel as
//! angles from +X towards +Y; [`travel_angle`] converts.
use crate::scene::Scene;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Conditions at one place and time. Missing values are `None`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Conditions {
    /// UTC, "YYYY-MM-DDTHH:MM".
    pub time: String,
    /// Total sea: significant height (m), mean period (s), from-bearing (°).
    pub wave_height: Option<f64>,
    pub wave_period: Option<f64>,
    pub wave_direction: Option<f64>,
    /// Wind sea.
    pub wind_wave_height: Option<f64>,
    pub wind_wave_period: Option<f64>,
    pub wind_wave_peak_period: Option<f64>,
    pub wind_wave_direction: Option<f64>,
    /// Primary swell.
    pub swell_height: Option<f64>,
    pub swell_period: Option<f64>,
    pub swell_peak_period: Option<f64>,
    pub swell_direction: Option<f64>,
    /// Surface current speed (m/s) and towards-bearing (°).
    pub current_speed: Option<f64>,
    pub current_direction: Option<f64>,
    /// Sea-surface temperature, °C.
    pub sea_temperature: Option<f64>,
    /// Wind 10 m above the sea: speed (m/s) and from-bearing (°).
    pub wind_speed: Option<f64>,
    pub wind_direction: Option<f64>,
}

/// Hourly conditions at one location.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub latitude: f64,
    pub longitude: f64,
    /// Data provider and models, for attribution and provenance.
    pub source: String,
    pub hours: Vec<Conditions>,
}

/// Wave heights over a set of locations and hours (a global field).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaveField {
    /// UTC hours, "YYYY-MM-DDTHH:MM".
    pub times: Vec<String>,
    pub points: Vec<FieldPoint>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldPoint {
    /// Requested location (degrees).
    pub latitude: f64,
    pub longitude: f64,
    /// Per hour: significant height (m), from-bearing (°), mean period (s).
    pub height: Vec<Option<f32>>,
    pub direction: Vec<Option<f32>>,
    pub period: Vec<Option<f32>>,
}

/// Marine variables requested per hour from Open-Meteo.
pub const MARINE_HOURLY: &str = "wave_height,wave_direction,wave_period,wind_wave_height,wind_wave_direction,wind_wave_period,wind_wave_peak_period,swell_wave_height,swell_wave_direction,swell_wave_period,swell_wave_peak_period,ocean_current_velocity,ocean_current_direction,sea_surface_temperature";

fn series(hourly: &Value, key: &str, len: usize, scale: f64) -> Vec<Option<f64>> {
    let list = hourly[key].as_array();
    (0..len)
        .map(|i| {
            list.and_then(|l| l.get(i))
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite())
                .map(|v| v * scale)
        })
        .collect()
}

impl Forecast {
    /// Parses an Open-Meteo marine response (single location, `hourly`
    /// with [`MARINE_HOURLY`], GMT) and optionally a weather response with
    /// `wind_speed_10m` (m/s) and `wind_direction_10m`, matched by time.
    pub fn parse_open_meteo(
        marine: &Value,
        weather: Option<&Value>,
        source: &str,
    ) -> Result<Self, String> {
        if let Some(reason) = marine["reason"].as_str() {
            return Err(format!("Forecast service: {reason}"));
        }
        let hourly = &marine["hourly"];
        let times: Vec<String> = hourly["time"]
            .as_array()
            .ok_or("The forecast has no hourly times.")?
            .iter()
            .map(|t| t.as_str().unwrap_or("").to_string())
            .collect();
        if times.is_empty() || times.len() > 24 * 40 {
            return Err("The forecast has no usable hours.".into());
        }
        let n = times.len();
        let get = |key: &str| series(hourly, key, n, 1.0);
        let (wh, wp, wd) = (
            get("wave_height"),
            get("wave_period"),
            get("wave_direction"),
        );
        let (ah, ap, apk, ad) = (
            get("wind_wave_height"),
            get("wind_wave_period"),
            get("wind_wave_peak_period"),
            get("wind_wave_direction"),
        );
        let (sh, sp, spk, sd) = (
            get("swell_wave_height"),
            get("swell_wave_period"),
            get("swell_wave_peak_period"),
            get("swell_wave_direction"),
        );
        // Open-Meteo reports current speed in km/h by default.
        let cs = series(hourly, "ocean_current_velocity", n, 1.0 / 3.6);
        let (cd, sst) = (
            get("ocean_current_direction"),
            get("sea_surface_temperature"),
        );
        let mut wind: std::collections::HashMap<&str, (Option<f64>, Option<f64>)> =
            Default::default();
        if let Some(w) = weather {
            let h = &w["hourly"];
            if let Some(t) = h["time"].as_array() {
                let speed = series(h, "wind_speed_10m", t.len(), 1.0);
                let dir = series(h, "wind_direction_10m", t.len(), 1.0);
                for (i, time) in t.iter().enumerate() {
                    if let Some(time) = time.as_str() {
                        wind.insert(time, (speed[i], dir[i]));
                    }
                }
            }
        }
        let hours = (0..n)
            .map(|i| {
                let (ws, wdir) = wind.get(times[i].as_str()).copied().unwrap_or_default();
                Conditions {
                    time: times[i].clone(),
                    wave_height: wh[i],
                    wave_period: wp[i],
                    wave_direction: wd[i],
                    wind_wave_height: ah[i],
                    wind_wave_period: ap[i],
                    wind_wave_peak_period: apk[i],
                    wind_wave_direction: ad[i],
                    swell_height: sh[i],
                    swell_period: sp[i],
                    swell_peak_period: spk[i],
                    swell_direction: sd[i],
                    current_speed: cs[i],
                    current_direction: cd[i],
                    sea_temperature: sst[i],
                    wind_speed: ws,
                    wind_direction: wdir,
                }
            })
            .collect();
        Ok(Self {
            latitude: marine["latitude"].as_f64().unwrap_or(f64::NAN),
            longitude: marine["longitude"].as_f64().unwrap_or(f64::NAN),
            source: source.into(),
            hours,
        })
    }

    /// Index of the hour `time` ("YYYY-MM-DDTHH:MM"), else the last hour not
    /// after it, else the first.
    pub fn index_at(&self, time: &str) -> usize {
        self.hours
            .iter()
            .rposition(|h| h.time.as_str() <= time)
            .unwrap_or(0)
    }
}

impl WaveField {
    /// Parses an Open-Meteo multi-location marine response (a list with one
    /// object per requested location, in request order, `hourly` with
    /// `wave_height`, `wave_direction`, `wave_period`).
    pub fn parse_open_meteo(response: &Value, requested: &[(f64, f64)]) -> Result<Self, String> {
        if let Some(reason) = response["reason"].as_str() {
            return Err(format!("Forecast service: {reason}"));
        }
        let single = [response.clone()];
        let list = match response.as_array() {
            Some(list) => list.as_slice(),
            None => &single[..],
        };
        if list.len() != requested.len() {
            return Err(format!(
                "Expected {} locations, received {}.",
                requested.len(),
                list.len()
            ));
        }
        let mut field = WaveField::default();
        for (item, &(latitude, longitude)) in list.iter().zip(requested) {
            let hourly = &item["hourly"];
            let times: Vec<String> = hourly["time"]
                .as_array()
                .map(|t| {
                    t.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if field.times.is_empty() {
                field.times = times.clone();
            }
            let n = field.times.len();
            let narrow = |v: Vec<Option<f64>>| v.into_iter().map(|x| x.map(|x| x as f32)).collect();
            field.points.push(FieldPoint {
                latitude,
                longitude,
                height: narrow(series(hourly, "wave_height", n, 1.0)),
                direction: narrow(series(hourly, "wave_direction", n, 1.0)),
                period: narrow(series(hourly, "wave_period", n, 1.0)),
            });
        }
        Ok(field)
    }
    /// Merges another batch of the same hours.
    pub fn extend(&mut self, other: WaveField) {
        if self.times.is_empty() {
            self.times = other.times;
        }
        self.points.extend(other.points);
    }
}

/// Scene angle (degrees from +X/east towards +Y/north) of travel towards a
/// compass bearing.
pub fn travel_angle(bearing_towards: f64) -> f64 {
    (90.0 - bearing_towards + 180.0).rem_euclid(360.0) - 180.0
}

/// Peak period from a mean period when no peak period is given: for a
/// JONSWAP spectrum the energy period is about 0.9 Tp.
fn peak_period(peak: Option<f64>, mean: Option<f64>) -> Option<f64> {
    peak.filter(|t| *t > 0.0)
        .or_else(|| mean.filter(|t| *t > 0.0).map(|t| t / 0.9))
}

/// Sets the scene's sea (wind sea + swell), wind, surface current and
/// surface temperature from `c`. Values outside the scene's linear-wave,
/// depth-limited envelope are clamped; each adjustment is returned as a note.
pub fn apply(scene: &mut Scene, c: &Conditions, source: &str) -> Vec<String> {
    let mut notes = vec![];
    let depth_limit = 0.25 * scene.bottom_depth;
    let steep_limit = |t: f64| 0.1 * 9.80665 * t * t / std::f64::consts::TAU * 0.98;
    // Wind sea: the wind-wave partition if given, else the whole sea.
    let (height, period, from) = match c.wind_wave_height {
        Some(h) => (
            Some(h),
            peak_period(c.wind_wave_peak_period, c.wind_wave_period),
            c.wind_wave_direction.or(c.wave_direction),
        ),
        None => (
            c.wave_height,
            peak_period(None, c.wave_period),
            c.wave_direction,
        ),
    };
    if let (Some(h), Some(t)) = (height, period) {
        let t = t.clamp(2.0, 20.0);
        let mut h = h.max(0.0);
        let cap = steep_limit(t).min(depth_limit).min(15.0);
        if h > cap {
            notes.push(format!(
                "Wind-sea height {h:.2} m clamped to {cap:.2} m (steepness / water depth)."
            ));
            h = cap;
        }
        scene.wave_height = h;
        scene.wave_period = t;
    }
    if let Some(from) = from {
        scene.wave_direction = travel_angle(from + 180.0);
    }
    // Swell only when the data separate it from the wind sea.
    match (c.wind_wave_height, c.swell_height) {
        (Some(_), Some(h)) if h > 0.0 => {
            let t = peak_period(c.swell_peak_period, c.swell_period)
                .unwrap_or(11.0)
                .clamp(4.0, 30.0);
            let room = (depth_limit * depth_limit - scene.wave_height * scene.wave_height)
                .max(0.0)
                .sqrt();
            let cap = steep_limit(t).min(room).min(12.0);
            let mut h = h;
            if h > cap {
                notes.push(format!(
                    "Swell height {h:.2} m clamped to {cap:.2} m (steepness / water depth)."
                ));
                h = cap;
            }
            scene.swell_height = h;
            scene.swell_period = t;
            if let Some(from) = c.swell_direction {
                scene.swell_direction = travel_angle(from + 180.0);
            }
        }
        _ => scene.swell_height = 0.0,
    }
    if let (Some(speed), Some(towards)) = (c.current_speed, c.current_direction) {
        let speed = speed.clamp(0.0, 4.0);
        let a = travel_angle(towards).to_radians();
        scene.current = [speed * a.cos(), speed * a.sin(), 0.0];
    }
    if let Some(t) = c.sea_temperature {
        scene.temperature = t.clamp(-2.0, 35.0);
        scene.deep_temperature = scene.deep_temperature.min(scene.temperature);
    }
    match (c.wind_speed, c.wind_direction) {
        (Some(speed), Some(from)) => {
            scene.wind_speed = speed.clamp(0.0, 60.0);
            scene.wind_direction = travel_angle(from + 180.0);
        }
        _ => scene.wind_speed = 0.0,
    }
    scene.sea_source = format!("{source}, {} UTC", c.time)
        .chars()
        .take(200)
        .collect();
    notes
}

/// Unix time of a UTC time "YYYY-MM-DDTHH:MM" (minutes optional).
pub fn parse_utc(time: &str) -> Option<i64> {
    let (date, clock) = time.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let hour = clock.get(..2)?.parse::<i64>().ok()?;
    let minute = clock
        .get(3..5)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    // Days from civil (H. Hinnant).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hour * 3600 + minute * 60)
}

/// UTC time of a Unix time as "YYYY-MM-DDTHH:MM".
pub fn format_utc(unix_seconds: i64) -> String {
    let minute = unix_seconds.rem_euclid(3600) / 60;
    let hour = utc_hour(unix_seconds);
    format!("{}{minute:02}", &hour[..14])
}

/// UTC hour of a Unix time as "YYYY-MM-DDTHH:00" (proleptic Gregorian).
pub fn utc_hour(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let hour = unix_seconds.rem_euclid(86_400) / 3600;
    // Civil-from-days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:00")
}
