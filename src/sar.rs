//! Search and rescue (SAR) planning: where a search object drifts, how to
//! search for it, and the chance a search finds it.
//!
//! 1. **Drift** — a Monte Carlo leeway model (Allen & Plourde 1999, USCG
//!    report CG-D-08-99; Breivik & Allen 2008), the method behind SAROPS and
//!    OpenDrift's Leeway model. Each particle is one hypothesis of the
//!    object: it enters the water inside the last known position's
//!    uncertainty circle at a time inside the incident window and moves with
//!    the surface current plus its leeway, a downwind and a crosswind
//!    component that grow with the 10 m wind. Every particle draws its own
//!    leeway error (applied as ε/20 to the slope and ε/2 to the offset, as
//!    in the published model), its own wind and current error and its side
//!    of the downwind direction, and jibes (changes side) at random.
//! 2. **Search plans** — expanding square, sector, parallel track and
//!    creeping line, laid out on the drift's principal axes and optionally
//!    moving with the drifting datum (the searcher navigates relative to the
//!    drift).
//! 3. **Probability of success** — the searcher runs its track through the
//!    drifting particles and each particle's probability of detection grows
//!    with a lateral-range model of sweep width W: the inverse cube law
//!    (Koopman 1946; detection rate W²V/(8πr³), whose parallel-sweep POD is
//!    erf(√π·C/2) for coverage C = W/S) or definite range (W = 2R).
//!    POS is the mean detection probability over all particles, so it
//!    already includes the chance that the object is outside the area.
//!
//! Positions are metres east (x) and north (y) of the last known position on
//! a local tangent plane (good to a few hundred kilometres).
use crate::{
    life::random,
    math::Det,
    metocean::{Forecast, format_utc, parse_utc},
};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const FORMAT: &str = "pelagic-sar";
const EARTH_RADIUS: f64 = 6_371_008.8;
/// Drift integration step and frame interval, hours (10 minutes).
pub const STEP_H: f64 = 1.0 / 6.0;
/// Smoothing length for blending forecast locations (m).
const BLEND_M: f64 = 3_000.0;

/// A search-object class with its leeway coefficients: slope in % of the
/// 10 m wind speed, offset and standard deviation in cm/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchObject {
    pub id: &'static str,
    pub name: &'static str,
    /// Typical visible size (m) for the camera sweep-width estimate. An
    /// approximation, not part of the leeway data.
    pub size_m: f64,
    /// Downwind: slope (%), offset (cm/s), standard deviation (cm/s).
    pub downwind: [f64; 3],
    /// Crosswind to the right and to the left of downwind.
    pub right: [f64; 3],
    pub left: [f64; 3],
}

const fn object(
    id: &'static str,
    name: &'static str,
    size_m: f64,
    downwind: [f64; 3],
    right: [f64; 3],
    left: [f64; 3],
) -> SearchObject {
    SearchObject {
        id,
        name,
        size_m,
        downwind,
        right,
        left,
    }
}

/// Leeway classes from the US Coast Guard leeway taxonomy (Allen & Plourde
/// 1999; Allen 2005), as tabulated for SAROPS and OpenDrift
/// (OBJECTPROP.DAT, B. Breivik, 2011). Use the general (mean) class unless
/// more is known about the object.
pub const OBJECTS: [SearchObject; 21] = [
    object(
        "PIW-1",
        "Person in water, unknown state (mean)",
        0.5,
        [0.96, 0.0, 12.0],
        [0.54, 0.0, 9.4],
        [-0.54, 0.0, 9.4],
    ),
    object(
        "PIW-2",
        "Person in water, vertical, conscious, PFD type III",
        0.5,
        [0.48, 0.0, 8.3],
        [0.15, 0.0, 6.7],
        [-0.15, 0.0, 6.7],
    ),
    object(
        "PIW-3",
        "Person in water, sitting (HELP position), PFD type I/II",
        0.6,
        [1.60, -3.98, 2.42],
        [0.13, 0.33, 2.11],
        [-0.13, -0.33, 2.11],
    ),
    object(
        "PIW-4",
        "Person in water, survival suit, face up",
        1.0,
        [1.71, 1.12, 3.93],
        [1.36, -3.30, 1.71],
        [-0.13, -2.65, 1.62],
    ),
    object(
        "PIW-5",
        "Person in water, scuba suit, face up",
        1.0,
        [0.63, 0.0, 5.3],
        [0.31, 0.0, 4.5],
        [-0.31, 0.0, 4.5],
    ),
    object(
        "PIW-6",
        "Person in water, deceased, face down",
        1.0,
        [1.117, 10.2, 3.04],
        [0.04, 3.90, 4.05],
        [-0.04, -3.90, 4.05],
    ),
    object(
        "LIFE-RAFT-NB-1",
        "Life raft, no ballast, general (mean)",
        2.0,
        [3.70, 0.0, 12.0],
        [1.98, 0.0, 9.4],
        [-1.98, 0.0, 9.4],
    ),
    object(
        "AVIATION-1",
        "Life raft, 4–6 person, no ballast, canopy, no drogue",
        2.0,
        [3.39, 0.0, 2.4],
        [1.49, 0.0, 2.4],
        [-1.49, 0.0, 2.4],
    ),
    object(
        "LIFE-RAFT-DB-11",
        "Life raft, 4–14 person, deep ballast, canopy (mean)",
        2.5,
        [3.50, -1.80, 6.4],
        [0.78, -3.60, 3.6],
        [-0.47, -0.10, 3.9],
    ),
    object(
        "LIFE-RAFT-DB-12",
        "Life raft, 4–14 person, deep ballast, no drogue",
        2.5,
        [3.75, -2.30, 4.4],
        [0.78, -3.60, 3.6],
        [-0.47, -0.10, 3.9],
    ),
    object(
        "LIFE-RAFT-DB-15",
        "Life raft, 4–14 person, deep ballast, canopy, drogue",
        2.5,
        [1.91, 0.90, 1.6],
        [0.78, -3.60, 3.6],
        [-0.47, -0.10, 3.9],
    ),
    object(
        "PERSON-POWERED-VESSEL-1",
        "Sea kayak with person on aft deck",
        4.5,
        [1.16, 11.12, 4.12],
        [0.41, 0.0, 4.39],
        [-0.41, 0.0, 4.39],
    ),
    object(
        "PERSON-POWERED-VESSEL-2",
        "Surfboard with person",
        2.0,
        [1.93, 0.0, 8.3],
        [0.51, 0.0, 6.7],
        [-0.51, 0.0, 6.7],
    ),
    object(
        "PERSON-POWERED-VESSEL-3",
        "Windsurfer, mast and sail in water",
        2.5,
        [2.25, 5.03, 2.5],
        [0.69, -1.30, 2.96],
        [-0.69, 1.30, 2.96],
    ),
    object(
        "SKIFF-1",
        "Skiff, modified-V runabout",
        5.0,
        [3.15, 0.0, 2.2],
        [1.29, 0.0, 2.2],
        [-1.29, 0.0, 2.2],
    ),
    object(
        "SPORT-BOAT",
        "Sport boat, cuddy cabin, modified-V hull",
        7.0,
        [6.54, 0.0, 3.0],
        [2.19, 0.0, 2.8],
        [-2.19, 0.0, 2.8],
    ),
    object(
        "FISHING-VESSEL-1",
        "Fishing vessel, general (mean)",
        15.0,
        [2.47, 0.0, 12.0],
        [2.76, 0.0, 9.4],
        [-2.76, 0.0, 9.4],
    ),
    object(
        "SAILBOAT-1",
        "Sailboat, monohull (mean)",
        10.0,
        [4.5, 0.0, 19.4],
        [4.95, 0.0, 18.42],
        [-2.82, 0.0, 24.95],
    ),
    object(
        "CONTAINER-2",
        "20-ft container, 80 % submerged",
        6.0,
        [1.25, 3.96, 2.81],
        [0.19, 1.14, 4.36],
        [-0.19, -1.14, 4.36],
    ),
    object(
        "BAIT-BOX-1",
        "Bait or wharf box (mean)",
        1.2,
        [0.72, 15.18, 5.59],
        [1.86, -5.26, 4.2],
        [-1.86, 5.26, 4.2],
    ),
    object(
        "FV-DEBRIS",
        "Fishing vessel debris",
        1.0,
        [1.97, 0.0, 8.3],
        [0.36, 0.0, 6.7],
        [-0.36, 0.0, 6.7],
    ),
];

/// The search object with this id (case-insensitive).
pub fn find_object(id: &str) -> Option<&'static SearchObject> {
    OBJECTS
        .iter()
        .find(|o| o.id.eq_ignore_ascii_case(id.trim()))
}

pub(crate) fn gaussian(state: &mut u64) -> f64 {
    let u1 = random(state).max(1e-300);
    let u2 = random(state);
    (-2.0 * u1.dln()).sqrt() * (std::f64::consts::TAU * u2).dcos()
}

/// East/north components of `speed` towards compass `bearing_deg`.
pub fn towards(speed: f64, bearing_deg: f64) -> [f64; 2] {
    let (s, c) = bearing_deg.to_radians().dsin_cos();
    [speed * s, speed * c]
}

/// Compass bearing (0–360°) of an east/north vector.
pub fn bearing(v: [f64; 2]) -> f64 {
    v[0].datan2(v[1]).to_degrees().rem_euclid(360.0)
}

/// Wind and current over time at one location.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Station {
    pub latitude: f64,
    pub longitude: f64,
    /// Per hour: 10 m wind blowing towards (east, north), m/s.
    pub wind: Vec<[f64; 2]>,
    /// Per hour: surface current flowing towards (east, north), m/s.
    pub current: Vec<[f64; 2]>,
}

/// Hourly wind and surface current at one or more locations, blended in
/// space by inverse distance and linearly in time.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Drivers {
    /// UTC time of hour 0, "YYYY-MM-DDTHH:MM".
    pub start: String,
    pub hours: usize,
    pub stations: Vec<Station>,
    pub source: String,
    pub notes: Vec<String>,
}

/// Fills gaps from the nearest earlier value, then the nearest later one.
fn fill(series: &[Option<[f64; 2]>]) -> Option<Vec<[f64; 2]>> {
    let first = series.iter().flatten().next().copied()?;
    let mut last = first;
    Some(
        series
            .iter()
            .map(|v| {
                if let Some(v) = v {
                    last = *v;
                }
                last
            })
            .collect(),
    )
}

impl Drivers {
    /// Steady wind (speed m/s, from-bearing) and current (speed m/s,
    /// towards-bearing) for `hours` hours from `start`.
    #[allow(clippy::too_many_arguments)]
    pub fn steady(
        start: &str,
        hours: usize,
        latitude: f64,
        longitude: f64,
        wind_speed: f64,
        wind_from: f64,
        current_speed: f64,
        current_towards: f64,
    ) -> Self {
        let hours = hours.max(2);
        Self {
            start: start.to_string(),
            hours,
            stations: vec![Station {
                latitude,
                longitude,
                wind: vec![towards(wind_speed, wind_from + 180.0); hours],
                current: vec![towards(current_speed, current_towards); hours],
            }],
            source: format!(
                "steady wind {wind_speed:.1} m/s from {wind_from:.0}°, current {current_speed:.2} m/s towards {current_towards:.0}°"
            ),
            notes: vec![],
        }
    }

    /// From hourly forecasts at one or more locations (same hours). Places
    /// without current data (land, ice) are left out; wind is required.
    pub fn from_forecasts(list: &[Forecast]) -> Result<Self, String> {
        let first = list.first().ok_or("No forecast to drift with.")?;
        let n = first.hours.len();
        if n < 2 {
            return Err("The forecast needs at least two hours.".into());
        }
        let mut stations = vec![];
        let (mut land, mut windless) = (0, 0);
        for f in list {
            if f.hours.len() != n
                || f.hours
                    .iter()
                    .zip(&first.hours)
                    .any(|(a, b)| a.time != b.time)
            {
                return Err("The forecast locations disagree on their hours.".into());
            }
            let wind: Vec<_> = f
                .hours
                .iter()
                .map(|h| match (h.wind_speed, h.wind_direction) {
                    (Some(s), Some(d)) => Some(towards(s.clamp(0.0, 70.0), d + 180.0)),
                    _ => None,
                })
                .collect();
            let current: Vec<_> = f
                .hours
                .iter()
                .map(|h| match (h.current_speed, h.current_direction) {
                    (Some(s), Some(d)) => Some(towards(s.clamp(0.0, 5.0), d)),
                    _ => None,
                })
                .collect();
            let Some(current) = fill(&current) else {
                land += 1;
                continue;
            };
            let Some(wind) = fill(&wind) else {
                windless += 1;
                continue;
            };
            stations.push(Station {
                latitude: f.latitude,
                longitude: f.longitude,
                wind,
                current,
            });
        }
        if stations.is_empty() {
            return Err(if windless > 0 {
                "The forecast has no wind here; leeway needs the 10 m wind.".into()
            } else {
                "No surface current or wind forecast near this position. Pick a point at sea."
                    .into()
            });
        }
        let mut notes = vec![];
        if land > 0 {
            notes.push(format!(
                "{land} of {} forecast locations are land or ice and were left out.",
                list.len()
            ));
        }
        if windless > 0 {
            notes.push(format!(
                "{windless} forecast locations had no wind and were left out."
            ));
        }
        Ok(Self {
            start: first.hours[0].time.clone(),
            hours: n,
            stations,
            source: first.source.clone(),
            notes,
        })
    }

    /// Hour index of a UTC time ("YYYY-MM-DDTHH:MM"), fractional.
    pub fn hour_of(&self, time: &str) -> Option<f64> {
        Some((parse_utc(time)? - parse_utc(&self.start)?) as f64 / 3600.0)
    }

    /// UTC time of an hour index.
    pub fn time_of(&self, hour: f64) -> String {
        parse_utc(&self.start).map_or_else(
            || format!("hour {hour:.2}"),
            |t| format_utc(t + (hour * 3600.0).round() as i64),
        )
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.hours < 2 || self.stations.is_empty() {
            return Err("Drift needs at least two hours of wind and current.".into());
        }
        for s in &self.stations {
            if s.wind.len() != self.hours
                || s.current.len() != self.hours
                || !s.latitude.is_finite()
                || !s.longitude.is_finite()
                || s.wind
                    .iter()
                    .chain(&s.current)
                    .flatten()
                    .any(|v| !v.is_finite())
            {
                return Err("The wind and current data are incomplete.".into());
            }
        }
        Ok(())
    }
}

/// Local tangent plane at a reference position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub latitude: f64,
    pub longitude: f64,
}
impl Plane {
    /// Metres east/north of the reference.
    pub fn local(&self, latitude: f64, longitude: f64) -> [f64; 2] {
        let dlon = (longitude - self.longitude + 540.0).rem_euclid(360.0) - 180.0;
        [
            EARTH_RADIUS * self.latitude.to_radians().dcos() * dlon.to_radians(),
            EARTH_RADIUS * (latitude - self.latitude).to_radians(),
        ]
    }
    /// Latitude and longitude of a local position.
    pub fn geo(&self, p: [f64; 2]) -> (f64, f64) {
        let lat = self.latitude + (p[1] / EARTH_RADIUS).to_degrees();
        let lon = self.longitude
            + (p[0] / (EARTH_RADIUS * self.latitude.to_radians().dcos().max(0.01))).to_degrees();
        (lat, (lon + 540.0).rem_euclid(360.0) - 180.0)
    }
}

/// The incident and the drift model's uncertainties.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DriftSetup {
    /// Search-object class id (see [`OBJECTS`]).
    pub object: String,
    /// Last known position (degrees).
    pub latitude: f64,
    pub longitude: f64,
    /// Radius of the circle holding 90 % of the possible start positions (m).
    pub radius_m: f64,
    /// The object entered the water between `incident_hour` and
    /// `incident_hour + window_h` (hours after the drivers' hour 0).
    pub incident_hour: f64,
    pub window_h: f64,
    pub particles: usize,
    /// Standard deviation of each particle's wind and current error, per
    /// component (m/s).
    pub wind_error_mps: f64,
    pub current_error_mps: f64,
    /// Chance per hour that the object changes its crosswind side.
    pub jibe_per_hour: f64,
    pub seed: u64,
}
impl Default for DriftSetup {
    fn default() -> Self {
        Self {
            object: "PIW-1".into(),
            latitude: 0.0,
            longitude: 0.0,
            radius_m: 1000.0,
            incident_hour: 0.0,
            window_h: 1.0,
            particles: 4000,
            wind_error_mps: 1.0,
            current_error_mps: 0.1,
            jibe_per_hour: 0.04,
            seed: 1,
        }
    }
}
impl DriftSetup {
    pub fn validate(&self) -> Result<(), String> {
        if find_object(&self.object).is_none() {
            return Err(format!("Unknown search object '{}'.", self.object));
        }
        if !(-90.0..=90.0).contains(&self.latitude) || !self.longitude.is_finite() {
            return Err("The last known position needs a valid latitude and longitude.".into());
        }
        let checks = [
            (self.radius_m, 0.0, 200_000.0, "Position uncertainty"),
            (self.window_h, 0.0, 72.0, "Incident window"),
            (self.wind_error_mps, 0.0, 10.0, "Wind error"),
            (self.current_error_mps, 0.0, 2.0, "Current error"),
            (self.jibe_per_hour, 0.0, 1.0, "Jibe chance"),
            (self.incident_hour, -1e6, 1e6, "Incident time"),
        ];
        for (v, lo, hi, name) in checks {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{name} must be between {lo} and {hi}."));
            }
        }
        if !(100..=50_000).contains(&self.particles) {
            return Err("Use 100 to 50,000 particles.".into());
        }
        Ok(())
    }
}

/// Spread of the particle cloud at one time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Spread {
    pub hour: f64,
    /// Mean position (m east/north of the last known position).
    pub datum: [f64; 2],
    /// Compass bearing (0–180°) of the major axis.
    pub axis_deg: f64,
    /// Standard deviations along and across the major axis (m).
    pub major_m: f64,
    pub minor_m: f64,
    /// Particles in the water at this time.
    pub released: usize,
}

/// Predicted positions of the search object.
#[derive(Clone, Debug, PartialEq)]
pub struct Drift {
    pub object: SearchObject,
    pub plane: Plane,
    /// UTC time of hour 0.
    pub start: String,
    pub source: String,
    /// Time of `frames[0]` (hours) and the frame interval.
    pub first_hour: f64,
    pub step_h: f64,
    /// Positions per frame per particle (m east/north of the last known
    /// position).
    pub frames: Vec<Vec<[f32; 2]>>,
    /// When each particle entered the water (hours).
    pub release: Vec<f32>,
    pub notes: Vec<String>,
}

/// One particle's draws.
struct Hypothesis {
    position: [f64; 2],
    release: f64,
    downwind_eps: f64,
    /// Normalised crosswind error (× the side's standard deviation).
    crosswind_n: f64,
    right: bool,
    wind_error: [f64; 2],
    current_error: [f64; 2],
}

/// Field sampler: stations' wind and current at one time, blended by
/// inverse distance.
pub(crate) struct Field {
    positions: Vec<[f64; 2]>,
    wind: Vec<[f64; 2]>,
    current: Vec<[f64; 2]>,
}
impl Field {
    pub(crate) fn new(drivers: &Drivers, plane: &Plane) -> Self {
        let n = drivers.stations.len();
        Self {
            positions: drivers
                .stations
                .iter()
                .map(|s| plane.local(s.latitude, s.longitude))
                .collect(),
            wind: vec![[0.0; 2]; n],
            current: vec![[0.0; 2]; n],
        }
    }
    pub(crate) fn at_time(&mut self, drivers: &Drivers, hour: f64) {
        let h = hour.clamp(0.0, (drivers.hours - 1) as f64);
        let k = (h.floor() as usize).min(drivers.hours - 2);
        let f = h - k as f64;
        let lerp = |a: [f64; 2], b: [f64; 2]| [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
        for (i, s) in drivers.stations.iter().enumerate() {
            self.wind[i] = lerp(s.wind[k], s.wind[k + 1]);
            self.current[i] = lerp(s.current[k], s.current[k + 1]);
        }
    }
    /// (wind, current) at a position.
    /// (wind, current) at a position.
    pub(crate) fn sample(&self, p: [f64; 2]) -> ([f64; 2], [f64; 2]) {
        if self.positions.len() == 1 {
            return (self.wind[0], self.current[0]);
        }
        let (mut w, mut c, mut total) = ([0.0; 2], [0.0; 2], 0.0);
        for (i, s) in self.positions.iter().enumerate() {
            let d2 = (p[0] - s[0]).powi(2) + (p[1] - s[1]).powi(2);
            let weight = 1.0 / (d2 + BLEND_M * BLEND_M);
            total += weight;
            for k in 0..2 {
                w[k] += weight * self.wind[i][k];
                c[k] += weight * self.current[i][k];
            }
        }
        ([w[0] / total, w[1] / total], [c[0] / total, c[1] / total])
    }
}

impl Hypothesis {
    /// Drift velocity (m/s) at position `p` in the sampled field.
    fn velocity(&self, object: &SearchObject, field: &Field, p: [f64; 2]) -> [f64; 2] {
        let (wind, current) = field.sample(p);
        let w = [wind[0] + self.wind_error[0], wind[1] + self.wind_error[1]];
        let speed = (w[0] * w[0] + w[1] * w[1]).sqrt();
        let down = if speed > 1e-9 {
            [w[0] / speed, w[1] / speed]
        } else {
            [0.0, 0.0]
        };
        let right_of = [down[1], -down[0]];
        let d = object.downwind;
        let e = self.downwind_eps;
        let downwind = ((d[0] + e / 20.0) * speed + d[1] + e / 2.0) * 0.01;
        let c = if self.right {
            object.right
        } else {
            object.left
        };
        let e = self.crosswind_n * c[2];
        let crosswind = ((c[0] + e / 20.0) * speed + c[1] + e / 2.0) * 0.01;
        [
            current[0] + self.current_error[0] + downwind * down[0] + crosswind * right_of[0],
            current[1] + self.current_error[1] + downwind * down[1] + crosswind * right_of[1],
        ]
    }
}

/// Simulates the drift from the incident until `until_hour`.
pub fn drift(
    drivers: &Drivers,
    setup: &DriftSetup,
    until_hour: f64,
    cancel: &AtomicBool,
) -> Result<Drift, String> {
    drivers.validate()?;
    setup.validate()?;
    let object = *find_object(&setup.object).expect("validated");
    let last = (drivers.hours - 1) as f64;
    if setup.incident_hour < 0.0 || setup.incident_hour > last {
        return Err(format!(
            "The incident time is outside the wind and current data ({} to {} UTC).",
            drivers.time_of(0.0).replace('T', " "),
            drivers.time_of(last).replace('T', " ")
        ));
    }
    if until_hour.is_nan() || until_hour <= setup.incident_hour {
        return Err("The search must end after the incident.".into());
    }
    if until_hour > last + 1e-9 {
        return Err(format!(
            "The search ends after the last forecast hour ({} UTC); shorten it or start it earlier.",
            drivers.time_of(last).replace('T', " ")
        ));
    }
    if until_hour - setup.incident_hour > 24.0 * 10.0 {
        return Err("Drift is limited to 10 days.".into());
    }
    let plane = Plane {
        latitude: setup.latitude,
        longitude: setup.longitude,
    };
    let mut state = setup.seed ^ 0x5a5_0000_0000_0001;
    let sigma = setup.radius_m / (2.0 * 10f64.dln()).sqrt();
    let jibe_step = 1.0 - (1.0 - setup.jibe_per_hour).max(1e-12).dpowf(STEP_H);
    let mut particles: Vec<Hypothesis> = (0..setup.particles)
        .map(|_| {
            let position = [gaussian(&mut state) * sigma, gaussian(&mut state) * sigma];
            let release = setup.incident_hour + random(&mut state) * setup.window_h;
            // Resample errors that would make the downwind slope negative.
            let mut downwind_eps = 0.0;
            for _ in 0..50 {
                let e = gaussian(&mut state) * object.downwind[2];
                if object.downwind[0] + e / 20.0 >= 0.0 {
                    downwind_eps = e;
                    break;
                }
            }
            Hypothesis {
                position,
                release,
                downwind_eps,
                crosswind_n: gaussian(&mut state),
                right: random(&mut state) < 0.5,
                wind_error: [
                    gaussian(&mut state) * setup.wind_error_mps,
                    gaussian(&mut state) * setup.wind_error_mps,
                ],
                current_error: [
                    gaussian(&mut state) * setup.current_error_mps,
                    gaussian(&mut state) * setup.current_error_mps,
                ],
            }
        })
        .collect();
    let mut field = Field::new(drivers, &plane);
    let snapshot = |ps: &[Hypothesis]| -> Vec<[f32; 2]> {
        ps.iter()
            .map(|p| [p.position[0] as f32, p.position[1] as f32])
            .collect()
    };
    let steps = ((until_hour - setup.incident_hour) / STEP_H).ceil() as usize;
    let mut frames = Vec::with_capacity(steps + 1);
    frames.push(snapshot(&particles));
    let dt_s = STEP_H * 3600.0;
    for step in 0..steps {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".into());
        }
        let t0 = setup.incident_hour + step as f64 * STEP_H;
        let t1 = t0 + STEP_H;
        // Wind and current at the step's midpoint time.
        field.at_time(drivers, t0 + 0.5 * STEP_H);
        for p in particles.iter_mut() {
            if p.release >= t1 {
                continue;
            }
            // Only the part of the step after the object entered the water;
            // midpoint (RK2) in space.
            let begin = p.release.max(t0);
            let span = (t1 - begin) / STEP_H * dt_s;
            let v1 = p.velocity(&object, &field, p.position);
            let mid = [
                p.position[0] + v1[0] * span * 0.5,
                p.position[1] + v1[1] * span * 0.5,
            ];
            let v2 = p.velocity(&object, &field, mid);
            p.position[0] += v2[0] * span;
            p.position[1] += v2[1] * span;
            if random(&mut state) < jibe_step {
                p.right = !p.right;
            }
        }
        frames.push(snapshot(&particles));
    }
    let mut notes = drivers.notes.clone();
    if drivers.stations.len() == 1 {
        notes.push(
            "Wind and current come from one location and are taken as uniform over the drift."
                .into(),
        );
    }
    let drift = Drift {
        object,
        plane,
        start: drivers.start.clone(),
        source: drivers.source.clone(),
        first_hour: setup.incident_hour,
        step_h: STEP_H,
        frames,
        release: particles.iter().map(|p| p.release as f32).collect(),
        notes,
    };
    Ok(drift)
}

impl Drift {
    /// A drift that holds fixed positions (tests, what-if studies).
    pub fn stationary(
        object: &SearchObject,
        plane: Plane,
        positions: Vec<[f64; 2]>,
        hours: f64,
    ) -> Self {
        let frame: Vec<[f32; 2]> = positions
            .iter()
            .map(|p| [p[0] as f32, p[1] as f32])
            .collect();
        let n = (hours / STEP_H).ceil() as usize + 1;
        Self {
            object: *object,
            plane,
            start: "2000-01-01T00:00".into(),
            source: "fixed positions".into(),
            first_hour: 0.0,
            step_h: STEP_H,
            release: vec![0.0; frame.len()],
            frames: vec![frame; n.max(2)],
            notes: vec![],
        }
    }

    pub fn particles(&self) -> usize {
        self.release.len()
    }
    pub fn last_hour(&self) -> f64 {
        self.first_hour + (self.frames.len() - 1) as f64 * self.step_h
    }
    /// UTC time of an hour index.
    pub fn time_of(&self, hour: f64) -> String {
        parse_utc(&self.start).map_or_else(
            || format!("hour {hour:.2}"),
            |t| format_utc(t + (hour * 3600.0).round() as i64),
        )
    }
    fn frame_at(&self, hour: f64) -> (usize, f64) {
        let x = ((hour - self.first_hour) / self.step_h).clamp(0.0, (self.frames.len() - 1) as f64);
        let k = (x.floor() as usize).min(self.frames.len().saturating_sub(2));
        (k, x - k as f64)
    }
    /// Particle `i` at `hour` (interpolated between frames).
    pub fn position(&self, i: usize, hour: f64) -> [f64; 2] {
        let (k, f) = self.frame_at(hour);
        let a = self.frames[k][i];
        let b = self.frames[(k + 1).min(self.frames.len() - 1)][i];
        [
            a[0] as f64 + (b[0] - a[0]) as f64 * f,
            a[1] as f64 + (b[1] - a[1]) as f64 * f,
        ]
    }
    /// All particles at `hour`, with whether each is in the water.
    pub fn positions(&self, hour: f64) -> Vec<([f64; 2], bool)> {
        (0..self.particles())
            .map(|i| (self.position(i, hour), f64::from(self.release[i]) <= hour))
            .collect()
    }
    /// Datum and spread of the particles in the water at `hour`.
    pub fn spread(&self, hour: f64) -> Spread {
        let points: Vec<[f64; 2]> = self
            .positions(hour)
            .into_iter()
            .filter(|(_, wet)| *wet)
            .map(|(p, _)| p)
            .collect();
        let n = points.len();
        if n == 0 {
            return Spread {
                hour,
                ..Default::default()
            };
        }
        let mean = points
            .iter()
            .fold([0.0; 2], |m, p| [m[0] + p[0], m[1] + p[1]]);
        let mean = [mean[0] / n as f64, mean[1] / n as f64];
        let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
        for p in &points {
            let (dx, dy) = (p[0] - mean[0], p[1] - mean[1]);
            a += dx * dx;
            b += dx * dy;
            c += dy * dy;
        }
        let (a, b, c) = (a / n as f64, b / n as f64, c / n as f64);
        let half = (((a - c) * 0.5).powi(2) + b * b).sqrt();
        let major = ((a + c) * 0.5 + half).max(0.0).sqrt();
        let minor = ((a + c) * 0.5 - half).max(0.0).sqrt();
        // Major-axis angle from east (counter-clockwise) → compass bearing.
        let phi = (2.0 * b).datan2(a - c) * 0.5;
        Spread {
            hour,
            datum: mean,
            axis_deg: (90.0 - phi.to_degrees()).rem_euclid(180.0),
            major_m: major,
            minor_m: minor,
            released: n,
        }
    }
    /// Share of all particles inside `area` at `hour` (not yet released
    /// particles count as outside).
    pub fn containment(&self, hour: f64, area: &Area) -> f64 {
        let inside = self
            .positions(hour)
            .iter()
            .filter(|(p, wet)| *wet && area.contains(*p))
            .count();
        inside as f64 / self.particles().max(1) as f64
    }
    /// Rectangle on the principal axes, centred on the datum, holding
    /// `fraction` of the particles in the water at `hour`.
    pub fn area(&self, hour: f64, fraction: f64) -> Area {
        let s = self.spread(hour);
        let base = Area {
            centre: s.datum,
            axis_deg: s.axis_deg,
            length_m: 2.0 * s.major_m.max(1.0),
            width_m: 2.0 * s.minor_m.max(1.0),
            round: false,
        };
        let points: Vec<[f64; 2]> = self
            .positions(hour)
            .into_iter()
            .filter(|(_, w)| *w)
            .map(|(p, _)| p)
            .collect();
        let wet = |a: &Area| {
            points.iter().filter(|p| a.contains(**p)).count() as f64 / points.len().max(1) as f64
        };
        let (mut lo, mut hi) = (0.1, 0.5);
        let scaled = |k: f64| Area {
            length_m: base.length_m * k,
            width_m: base.width_m * k,
            ..base
        };
        while wet(&scaled(hi)) < fraction && hi < 1e4 {
            hi *= 2.0;
        }
        for _ in 0..30 {
            let mid = 0.5 * (lo + hi);
            if wet(&scaled(mid)) < fraction {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        scaled(hi)
    }
}

/// A search area: a rectangle on a compass axis or, when `round`, a circle
/// of diameter `length_m`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub centre: [f64; 2],
    /// Compass bearing of the long side.
    pub axis_deg: f64,
    pub length_m: f64,
    pub width_m: f64,
    pub round: bool,
}
impl Area {
    /// Unit vectors along the axis and to its right.
    pub fn axes(&self) -> ([f64; 2], [f64; 2]) {
        let along = towards(1.0, self.axis_deg);
        (along, [along[1], -along[0]])
    }
    pub fn contains(&self, p: [f64; 2]) -> bool {
        let d = [p[0] - self.centre[0], p[1] - self.centre[1]];
        if self.round {
            return d[0] * d[0] + d[1] * d[1] <= (self.length_m * 0.5).powi(2);
        }
        let (a, r) = self.axes();
        (d[0] * a[0] + d[1] * a[1]).abs() <= self.length_m * 0.5
            && (d[0] * r[0] + d[1] * r[1]).abs() <= self.width_m * 0.5
    }
    pub fn size_m2(&self) -> f64 {
        if self.round {
            PI * (self.length_m * 0.5).powi(2)
        } else {
            self.length_m * self.width_m
        }
    }
    /// Corners (rectangles) in order.
    pub fn corners(&self) -> [[f64; 2]; 4] {
        let (a, r) = self.axes();
        let (l, w) = (self.length_m * 0.5, self.width_m * 0.5);
        let at = |u: f64, v: f64| {
            [
                self.centre[0] + a[0] * u + r[0] * v,
                self.centre[1] + a[1] * u + r[1] * v,
            ]
        };
        [at(-l, -w), at(l, -w), at(l, w), at(-l, w)]
    }
}

/// Standard search patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pattern {
    ExpandingSquare,
    Sector,
    ParallelTrack,
    CreepingLine,
}
impl Pattern {
    pub const ALL: [Pattern; 4] = [
        Pattern::ExpandingSquare,
        Pattern::Sector,
        Pattern::ParallelTrack,
        Pattern::CreepingLine,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Pattern::ExpandingSquare => "Expanding square",
            Pattern::Sector => "Sector",
            Pattern::ParallelTrack => "Parallel track",
            Pattern::CreepingLine => "Creeping line",
        }
    }
    /// Short code used in charts and on the command line.
    pub fn code(self) -> &'static str {
        match self {
            Pattern::ExpandingSquare => "SS",
            Pattern::Sector => "VS",
            Pattern::ParallelTrack => "PS",
            Pattern::CreepingLine => "CS",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        let t = text.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|p| {
            p.code().eq_ignore_ascii_case(&t)
                || p.name().to_ascii_lowercase().replace(' ', "-") == t
        })
    }
}

/// The search unit and its time on scene.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Searcher {
    pub name: String,
    /// Search speed (m/s).
    pub speed_mps: f64,
    /// Sweep width for this object and these conditions (m).
    pub sweep_width_m: f64,
    /// Definite-range detection (certain inside W/2, nothing outside)
    /// instead of the inverse cube law.
    pub definite_range: bool,
    /// Arrival on scene (hours after the drivers' hour 0) and time
    /// available for searching (h).
    pub start_hour: f64,
    pub endurance_h: f64,
    /// The pattern moves with the drifting datum.
    pub follow_drift: bool,
}
impl Default for Searcher {
    fn default() -> Self {
        Self {
            name: "Search unit".into(),
            speed_mps: 10.0 * 0.514_444,
            sweep_width_m: 400.0,
            definite_range: false,
            start_hour: 0.0,
            endurance_h: 4.0,
            follow_drift: true,
        }
    }
}
impl Searcher {
    pub fn validate(&self) -> Result<(), String> {
        let checks = [
            (self.speed_mps, 0.2, 100.0, "Search speed (m/s)"),
            (self.sweep_width_m, 1.0, 50_000.0, "Sweep width (m)"),
            (self.endurance_h, 0.1, 72.0, "Time on scene (h)"),
            (self.start_hour, -1e6, 1e6, "Arrival time"),
        ];
        for (v, lo, hi, name) in checks {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{name} must be between {lo} and {hi}."));
            }
        }
        Ok(())
    }
    /// Track length the searcher can cover (m).
    pub fn budget_m(&self) -> f64 {
        self.speed_mps * self.endurance_h * 3600.0
    }
}

/// Geometric range limit (m) for spotting an object of height `object_m`
/// from an eye or camera `eye_m` above the sea (standard refraction).
pub fn horizon_m(eye_m: f64, object_m: f64) -> f64 {
    3570.0 * (eye_m.max(0.0).sqrt() + object_m.max(0.0).sqrt())
}

/// Sweep width estimate (m) for a camera: the object of size `size_m` must
/// span `pixels_needed` pixels of a sensor `pixels` wide over a
/// `fov_deg` field of view, within the horizon; definite range, W = 2R.
pub fn camera_sweep_width(
    size_m: f64,
    eye_m: f64,
    fov_deg: f64,
    pixels: f64,
    pixels_needed: f64,
) -> f64 {
    let ifov = fov_deg.clamp(0.1, 180.0).to_radians() / pixels.max(1.0);
    let range = size_m / (pixels_needed.max(1.0) * ifov);
    2.0 * range.min(horizon_m(eye_m, 0.3 * size_m))
}

/// A planned search.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub pattern: Pattern,
    /// Track spacing (m); for sector searches the leg length (radius).
    pub spacing_m: f64,
    pub area: Area,
    /// Velocity of the pattern (m/s): the datum's drift when following it.
    pub drift_velocity: [f64; 2],
    pub start_hour: f64,
    /// Waypoints at `start_hour` (m east/north of the last known position).
    pub waypoints: Vec<[f64; 2]>,
}
impl Plan {
    pub fn track_m(&self) -> f64 {
        self.waypoints
            .windows(2)
            .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
            .sum()
    }
    /// Position after `distance` metres of track, `seconds` after the
    /// start (the pattern itself drifts).
    pub fn at(&self, distance: f64, seconds: f64) -> [f64; 2] {
        let mut left = distance.max(0.0);
        // Past the end of the track the searcher holds its last waypoint.
        let mut p = *self.waypoints.last().unwrap_or(&[0.0, 0.0]);
        for w in self.waypoints.windows(2) {
            let leg = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
            if left <= leg {
                let f = if leg > 0.0 { left / leg } else { 0.0 };
                p = [
                    w[0][0] + (w[1][0] - w[0][0]) * f,
                    w[0][1] + (w[1][1] - w[0][1]) * f,
                ];
                break;
            }
            left -= leg;
        }
        [
            p[0] + self.drift_velocity[0] * seconds,
            p[1] + self.drift_velocity[1] * seconds,
        ]
    }
    /// Ground position and time (hours) of each waypoint at `speed` m/s.
    pub fn schedule(&self, speed: f64) -> Vec<(f64, [f64; 2])> {
        let mut distance = 0.0;
        let mut out = vec![];
        for (i, w) in self.waypoints.iter().enumerate() {
            if i > 0 {
                let a = self.waypoints[i - 1];
                distance += ((w[0] - a[0]).powi(2) + (w[1] - a[1]).powi(2)).sqrt();
            }
            let seconds = distance / speed;
            out.push((
                self.start_hour + seconds / 3600.0,
                [
                    w[0] + self.drift_velocity[0] * seconds,
                    w[1] + self.drift_velocity[1] * seconds,
                ],
            ));
        }
        out
    }
}

fn along(origin: [f64; 2], dir: [f64; 2], d: f64) -> [f64; 2] {
    [origin[0] + dir[0] * d, origin[1] + dir[1] * d]
}

/// Appends legs of the given lengths along headings until `budget` metres
/// are used (the last leg is shortened to fit).
fn walk(start: [f64; 2], legs: impl Iterator<Item = (f64, f64)>, budget: f64) -> Vec<[f64; 2]> {
    let mut points = vec![start];
    let mut left = budget;
    for (heading, length) in legs {
        if left <= 1e-6 {
            break;
        }
        let d = length.min(left);
        let p = along(*points.last().expect("start"), towards(1.0, heading), d);
        points.push(p);
        left -= d;
    }
    points
}

/// Expanding square from `centre`: legs S, S, 2S, 2S, 3S… turning right,
/// the first along `axis_deg`.
pub fn expanding_square(
    centre: [f64; 2],
    axis_deg: f64,
    spacing: f64,
    budget: f64,
) -> Vec<[f64; 2]> {
    let legs =
        (0..100_000).map(move |n| (axis_deg + 90.0 * n as f64, (n / 2 + 1) as f64 * spacing));
    walk(centre, legs, budget)
}

/// Sector search from `centre` with legs of `radius`: three triangles with
/// 120° right turns (passing through the centre between triangles), then
/// further passes rotated by 30°, 15° and 45°.
pub fn sector(centre: [f64; 2], axis_deg: f64, radius: f64, budget: f64) -> Vec<[f64; 2]> {
    let offsets = [0.0, 30.0, 15.0, 45.0];
    let legs = (0..36).map(move |k| {
        let pass = k / 9;
        let leg = k % 9;
        // Headings: out, +120, +240 (back to centre), then continue.
        let turns = leg - leg / 3;
        (axis_deg + offsets[pass] + 120.0 * turns as f64, radius)
    });
    walk(centre, legs, budget)
}

/// Parallel track (legs along the area's long side, stepping across it) or
/// creeping line (legs across, stepping along the axis from its far end
/// back towards its start). Legs are `spacing` apart, the outer ones half a
/// spacing inside the edges.
pub fn parallel(area: &Area, spacing: f64, budget: f64, creeping: bool) -> Vec<[f64; 2]> {
    let (a, r) = area.axes();
    let (leg_dir, step_dir, leg_len, extent) = if creeping {
        (r, [-a[0], -a[1]], area.width_m, area.length_m)
    } else {
        (a, r, area.length_m, area.width_m)
    };
    let n = ((extent / spacing).round() as usize).max(1);
    let first = -(n as f64 - 1.0) * spacing * 0.5;
    let mut points: Vec<[f64; 2]> = vec![];
    let mut left = budget;
    for j in 0..n {
        let offset = first + j as f64 * spacing;
        let sign = if j % 2 == 0 { -1.0 } else { 1.0 };
        let base = along(area.centre, step_dir, offset);
        let start = along(base, leg_dir, sign * leg_len * 0.5);
        let end = along(base, leg_dir, -sign * leg_len * 0.5);
        if let Some(&last) = points.last() {
            let gap = (start[0] - last[0]).hypot(start[1] - last[1]);
            if gap > left {
                let f = left / gap;
                points.push([
                    last[0] + (start[0] - last[0]) * f,
                    last[1] + (start[1] - last[1]) * f,
                ]);
                return points;
            }
            left -= gap;
        }
        points.push(start);
        if leg_len > left {
            points.push(along(start, leg_dir, -sign * left));
            return points;
        }
        left -= leg_len;
        points.push(end);
    }
    points
}

/// Result of running one plan through the drift.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub plan: Plan,
    /// Coverage factor C = W · track length / area.
    pub coverage: f64,
    pub track_m: f64,
    pub duration_h: f64,
    /// Probability of success: the chance this search finds the object.
    pub pos: f64,
    /// Probability the object is inside the area at mid-search (POC) and of
    /// detecting it there (POD).
    pub poc: f64,
    pub pod: f64,
    /// (hours since the search started, cumulative POS) every 10 minutes.
    pub pos_curve: Vec<[f64; 2]>,
    /// Detection probability per particle at the same times.
    #[serde(skip)]
    pub detection: Vec<Vec<f32>>,
}

/// Runs `plan` through `drift` and accumulates each particle's detection
/// probability.
pub fn evaluate(drift: &Drift, searcher: &Searcher, plan: &Plan) -> Outcome {
    let v = searcher.speed_mps;
    let w = searcher.sweep_width_m;
    let track = plan.track_m();
    let duration_s = (track / v).min(searcher.endurance_h * 3600.0);
    // Definite range needs fine steps so no one slips between samples.
    let dt = if searcher.definite_range {
        w / (10.0 * v)
    } else {
        w / (4.0 * v)
    }
    .clamp(0.2, 10.0);
    let rate = w * w * v / (8.0 * PI);
    let cutoff2 = (8.0 * w).powi(2);
    let r_min = w / 20.0;
    let n = drift.particles();
    let mut hazard = vec![0.0f64; n];
    let steps = (duration_s / dt).ceil() as usize;
    let record_every = (STEP_H * 3600.0 / dt).round().max(1.0) as usize;
    let mut detection = vec![vec![0.0f32; n]];
    let mut pos_curve = vec![[0.0, 0.0]];
    let probability = |h: f64| 1.0 - (-h).dexp();
    for s in 0..steps {
        let t0 = s as f64 * dt;
        let span = dt.min(duration_s - t0);
        let t = t0 + span * 0.5;
        let hour = plan.start_hour + t / 3600.0;
        let me = plan.at(t * v, t);
        let (k, f) = drift.frame_at(hour);
        let (fa, fb) = (
            &drift.frames[k],
            &drift.frames[(k + 1).min(drift.frames.len() - 1)],
        );
        let f = f as f32;
        for i in 0..n {
            if f64::from(drift.release[i]) > hour {
                continue;
            }
            let px = f64::from(fa[i][0] + (fb[i][0] - fa[i][0]) * f);
            let py = f64::from(fa[i][1] + (fb[i][1] - fa[i][1]) * f);
            let d2 = (px - me[0]).powi(2) + (py - me[1]).powi(2);
            if d2 > cutoff2 {
                continue;
            }
            if searcher.definite_range {
                if d2 <= (w * 0.5).powi(2) {
                    hazard[i] = 50.0;
                }
            } else {
                let r = d2.sqrt().max(r_min);
                hazard[i] = (hazard[i] + rate / (r * r * r) * span).min(50.0);
            }
        }
        if (s + 1) % record_every == 0 || s + 1 == steps {
            let p: Vec<f32> = hazard.iter().map(|h| probability(*h) as f32).collect();
            pos_curve.push([
                (t0 + span) / 3600.0,
                p.iter().map(|x| f64::from(*x)).sum::<f64>() / n.max(1) as f64,
            ]);
            detection.push(p);
        }
    }
    let pos = pos_curve.last().map_or(0.0, |p| p[1]);
    let mid = plan.start_hour + duration_s / 7200.0;
    // The area drifts with the pattern.
    let mut area = plan.area;
    area.centre[0] += plan.drift_velocity[0] * duration_s * 0.5;
    area.centre[1] += plan.drift_velocity[1] * duration_s * 0.5;
    let (mut inside, mut found) = (0usize, 0.0);
    for (i, h) in hazard.iter().enumerate() {
        if f64::from(drift.release[i]) <= mid && area.contains(drift.position(i, mid)) {
            inside += 1;
            found += probability(*h);
        }
    }
    let poc = inside as f64 / n.max(1) as f64;
    Outcome {
        coverage: w * track.min(duration_s * v) / area.size_m2().max(1.0),
        track_m: track,
        duration_h: duration_s / 3600.0,
        pos,
        poc,
        pod: if inside > 0 {
            found / inside as f64
        } else {
            0.0
        },
        pos_curve,
        detection,
        plan: plan.clone(),
    }
}

/// Candidate plans for the searcher: each pattern at several coverage
/// factors, centred on the datum and laid along the drift.
pub fn candidates(drift: &Drift, searcher: &Searcher, patterns: &[Pattern]) -> Vec<Plan> {
    let start = searcher.start_hour;
    let end = (start + searcher.endurance_h).min(drift.last_hour());
    let mid = 0.5 * (start + end);
    let s_mid = drift.spread(mid);
    let s_start = drift.spread(start);
    let s_end = drift.spread(end);
    let seconds = ((end - start) * 3600.0).max(1.0);
    let datum_velocity = [
        (s_end.datum[0] - s_start.datum[0]) / seconds,
        (s_end.datum[1] - s_start.datum[1]) / seconds,
    ];
    let (velocity, centre) = if searcher.follow_drift {
        (datum_velocity, s_start.datum)
    } else {
        ([0.0, 0.0], s_mid.datum)
    };
    // Long axis pointing down-drift.
    let mut axis = s_mid.axis_deg;
    let dir = towards(1.0, axis);
    if dir[0] * datum_velocity[0] + dir[1] * datum_velocity[1] < 0.0 {
        axis = (axis + 180.0).rem_euclid(360.0);
    }
    let ratio = (s_mid.major_m / s_mid.minor_m.max(1.0)).clamp(1.0, 4.0);
    let w = searcher.sweep_width_m;
    let budget = searcher.budget_m();
    let factors = [0.5, 0.75, 1.0, 1.5, 2.0];
    let patterns = if patterns.is_empty() {
        &Pattern::ALL[..]
    } else {
        patterns
    };
    let plan = |pattern, spacing, area, waypoints| Plan {
        pattern,
        spacing_m: spacing,
        area,
        drift_velocity: velocity,
        start_hour: start,
        waypoints,
    };
    let mut plans = vec![];
    for &pattern in patterns {
        match pattern {
            Pattern::ParallelTrack | Pattern::CreepingLine => {
                let creeping = pattern == Pattern::CreepingLine;
                for c in factors {
                    let s = w / c;
                    let a = w * budget / c;
                    let (mut length, mut width) = ((a * ratio).sqrt(), (a / ratio).sqrt());
                    // Whole legs that use the budget exactly.
                    if creeping {
                        let legs = (length / s).round().max(1.0);
                        length = legs * s;
                        width = (budget - (legs - 1.0) * s) / legs;
                    } else {
                        let legs = (width / s).round().max(1.0);
                        width = legs * s;
                        length = (budget - (legs - 1.0) * s) / legs;
                    }
                    if length <= s * 0.5 || width <= s * 0.5 {
                        continue;
                    }
                    let area = Area {
                        centre,
                        axis_deg: axis,
                        length_m: length,
                        width_m: width,
                        round: false,
                    };
                    plans.push(plan(pattern, s, area, parallel(&area, s, budget, creeping)));
                }
            }
            Pattern::ExpandingSquare => {
                for c in factors {
                    let s = w / c;
                    let points = expanding_square(centre, axis, s, budget);
                    // Bounding square on the pattern's axes plus half a spacing.
                    let area_axis = Area {
                        centre,
                        axis_deg: axis,
                        ..Default::default()
                    };
                    let (a, r) = area_axis.axes();
                    let (mut u, mut q) = (0.0f64, 0.0f64);
                    for p in &points {
                        let d = [p[0] - centre[0], p[1] - centre[1]];
                        u = u.max((d[0] * a[0] + d[1] * a[1]).abs());
                        q = q.max((d[0] * r[0] + d[1] * r[1]).abs());
                    }
                    let side = 2.0 * u.max(q) + s;
                    let area = Area {
                        length_m: side,
                        width_m: side,
                        ..area_axis
                    };
                    plans.push(plan(pattern, s, area, points));
                }
            }
            Pattern::Sector => {
                for passes in [1.0, 2.0, 3.0, 4.0] {
                    let radius = budget / (9.0 * passes);
                    let area = Area {
                        centre,
                        axis_deg: axis,
                        length_m: 2.0 * radius,
                        width_m: 2.0 * radius,
                        round: true,
                    };
                    plans.push(plan(
                        pattern,
                        radius,
                        area,
                        sector(centre, axis, radius, budget),
                    ));
                }
            }
        }
    }
    plans
}

/// Everything for one SAR case.
#[derive(Clone, Debug)]
pub struct Report {
    pub setup: DriftSetup,
    pub searcher: Searcher,
    pub drift: Drift,
    /// Evaluated plans, best (highest POS) first.
    pub options: Vec<Outcome>,
    pub warnings: Vec<String>,
}

/// Drift, then plan and evaluate the search; `progress(done, total)`
/// reports evaluated plans.
pub fn run(
    drivers: &Drivers,
    setup: &DriftSetup,
    searcher: &Searcher,
    patterns: &[Pattern],
    threads: usize,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<Report, String> {
    searcher.validate()?;
    let end = searcher.start_hour + searcher.endurance_h;
    let mut warnings = vec![];
    if searcher.start_hour < setup.incident_hour {
        return Err("The search must start after the incident.".into());
    }
    let drift = drift(drivers, setup, end, cancel)?;
    let plans = candidates(&drift, searcher, patterns);
    if plans.is_empty() {
        return Err("No search plan fits this sweep width and time on scene.".into());
    }
    let total = plans.len();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let mut options: Vec<Outcome> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.clamp(1, 32).min(total))
            .map(|_| {
                scope.spawn(|| {
                    let mut out = vec![];
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(plan) = plans.get(i) else { break };
                        out.push((i, evaluate(&drift, searcher, plan)));
                        progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                    }
                    out
                })
            })
            .collect();
        let mut all: Vec<(usize, Outcome)> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_default())
            .collect();
        all.sort_by_key(|(i, _)| *i);
        all.into_iter().map(|(_, o)| o).collect()
    });
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled.".into());
    }
    options.sort_by(|a, b| b.pos.total_cmp(&a.pos));
    let s = drift.spread(searcher.start_hour);
    if s.released < drift.particles() {
        warnings.push(format!(
            "{} of {} hypotheses enter the water after the search starts (incident window).",
            drift.particles() - s.released,
            drift.particles()
        ));
    }
    if let Some(best) = options.first()
        && best.poc < 0.5
    {
        warnings.push(format!(
            "The best plan covers only {:.0} % of where the object may be: more search units, a wider sweep width or more time are needed.",
            best.poc * 100.0
        ));
    }
    Ok(Report {
        setup: setup.clone(),
        searcher: searcher.clone(),
        drift,
        options,
        warnings,
    })
}

#[derive(Serialize)]
struct SpreadRow {
    time_utc: String,
    hour: f64,
    datum_latitude: f64,
    datum_longitude: f64,
    drift_from_lkp_m: f64,
    drift_bearing_deg: f64,
    axis_deg: f64,
    major_m: f64,
    minor_m: f64,
    in_water: usize,
}

#[derive(Serialize)]
struct OptionRow<'a> {
    pattern: &'a str,
    spacing_m: f64,
    area_km2: f64,
    coverage: f64,
    poc: f64,
    pod: f64,
    pos: f64,
}

impl Report {
    /// Datum track and spread every hour from the incident to the search end.
    pub fn spreads(&self) -> Vec<Spread> {
        let mut out = vec![];
        let mut h = self.drift.first_hour;
        while h <= self.drift.last_hour() + 1e-9 {
            out.push(self.drift.spread(h));
            h += 1.0;
        }
        if out
            .last()
            .is_some_and(|s| s.hour < self.drift.last_hour() - 1e-6)
        {
            out.push(self.drift.spread(self.drift.last_hour()));
        }
        out
    }

    pub fn summary(&self, chosen: usize) -> String {
        let d = &self.drift;
        let mut s = format!(
            "Pelagic search and rescue plan\n\nObject: {} ({})\nLast known position: {:.4}°, {:.4}° (90 % within {:.0} m)\nIn the water: {} to {} UTC\nWind and current: {}\n",
            d.object.name,
            d.object.id,
            self.setup.latitude,
            self.setup.longitude,
            self.setup.radius_m,
            d.time_of(self.setup.incident_hour).replace('T', " "),
            d.time_of(self.setup.incident_hour + self.setup.window_h)
                .replace('T', " "),
            d.source
        );
        for (label, hour) in [
            ("Search starts", self.searcher.start_hour),
            (
                "Search ends",
                self.searcher.start_hour + self.searcher.endurance_h,
            ),
        ] {
            let sp = d.spread(hour.min(d.last_hour()));
            let (lat, lon) = d.plane.geo(sp.datum);
            s += &format!(
                "{label} {} UTC: datum {lat:.4}°, {lon:.4}°, {:.2} km from the LKP towards {:.0}°, spread {:.2} × {:.2} km (1σ) on {:.0}°\n",
                d.time_of(hour).replace('T', " "),
                sp.datum[0].hypot(sp.datum[1]) / 1000.0,
                bearing(sp.datum),
                sp.major_m / 1000.0,
                sp.minor_m / 1000.0,
                sp.axis_deg
            );
        }
        s += &format!(
            "\nSearcher: {}, {:.1} kn, sweep width {:.0} m ({}), {:.1} h on scene{}\n\nPlans (best first)\n  pattern            spacing    area km²   C      POC    POD    POS\n",
            self.searcher.name,
            self.searcher.speed_mps / 0.514_444,
            self.searcher.sweep_width_m,
            if self.searcher.definite_range {
                "definite range"
            } else {
                "inverse cube law"
            },
            self.searcher.endurance_h,
            if self.searcher.follow_drift {
                ", pattern follows the drift"
            } else {
                ""
            }
        );
        for (i, o) in self.options.iter().enumerate() {
            s += &format!(
                "{} {:<18} {:>7.0} m {:>9.2} {:>6.2} {:>6.2} {:>6.2} {:>6.2}\n",
                if i == chosen { "*" } else { " " },
                o.plan.pattern.name(),
                o.plan.spacing_m,
                o.plan.area.size_m2() / 1e6,
                o.coverage,
                o.poc,
                o.pod,
                o.pos
            );
        }
        for w in self.warnings.iter().chain(&d.notes) {
            s += &format!("\nNote: {w}");
        }
        s += "\n\nPlanning aid only: confirm with your rescue coordination centre's tools and procedures.\n";
        s
    }

    /// GPX route of the chosen plan (ground positions at the planned times).
    pub fn gpx(&self, chosen: usize) -> String {
        let Some(o) = self.options.get(chosen) else {
            return String::new();
        };
        let d = &self.drift;
        let (lat, lon) = (self.setup.latitude, self.setup.longitude);
        let mut s = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"Pelagic\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n  <wpt lat=\"{lat:.6}\" lon=\"{lon:.6}\"><name>LKP</name></wpt>\n"
        );
        let sp = d.spread(o.plan.start_hour);
        let (dl, dn) = d.plane.geo(sp.datum);
        s += &format!(
            "  <wpt lat=\"{dl:.6}\" lon=\"{dn:.6}\"><time>{}:00Z</time><name>Datum at search start</name></wpt>\n  <rte><name>{} {:.0} m</name>\n",
            d.time_of(o.plan.start_hour),
            o.plan.pattern.name(),
            o.plan.spacing_m
        );
        for (i, (hour, p)) in o.plan.schedule(self.searcher.speed_mps).iter().enumerate() {
            let (la, lo) = d.plane.geo(*p);
            s += &format!(
                "    <rtept lat=\"{la:.6}\" lon=\"{lo:.6}\"><time>{}:00Z</time><name>WP{}</name></rtept>\n",
                d.time_of(*hour),
                i + 1
            );
        }
        s += "  </rte>\n</gpx>\n";
        s
    }

    /// QGroundControl / Mission Planner waypoint list (QGC WPL 110) for an
    /// uncrewed surface vehicle: home at the first waypoint, search speed,
    /// then the waypoints.
    pub fn waypoints(&self, chosen: usize) -> String {
        let Some(o) = self.options.get(chosen) else {
            return String::new();
        };
        let schedule = o.plan.schedule(self.searcher.speed_mps);
        let mut s = String::from("QGC WPL 110\n");
        let geo = |p: [f64; 2]| self.drift.plane.geo(p);
        let (hl, ho) = geo(schedule[0].1);
        s += &format!("0\t1\t0\t16\t0\t0\t0\t0\t{hl:.7}\t{ho:.7}\t0\t1\n");
        s += &format!(
            "1\t0\t2\t178\t1\t{:.2}\t-1\t0\t0\t0\t0\t1\n",
            self.searcher.speed_mps
        );
        for (i, (_, p)) in schedule.iter().enumerate() {
            let (la, lo) = geo(*p);
            s += &format!("{}\t0\t3\t16\t0\t0\t0\t0\t{la:.7}\t{lo:.7}\t0\t1\n", i + 2);
        }
        s
    }

    /// Writes summary.txt, drift.csv, particles.csv (search end, with the
    /// chosen plan's detection probability), plans.json, plan.gpx and
    /// plan.waypoints.
    pub fn write(&self, folder: &std::path::Path, chosen: usize) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let put = |name: &str, text: String| {
            std::fs::write(folder.join(name), text).map_err(|e| format!("{name}: {e}"))
        };
        put("summary.txt", self.summary(chosen))?;
        let d = &self.drift;
        let rows: Vec<SpreadRow> = self
            .spreads()
            .iter()
            .map(|s| {
                let (lat, lon) = d.plane.geo(s.datum);
                SpreadRow {
                    time_utc: d.time_of(s.hour),
                    hour: s.hour,
                    datum_latitude: lat,
                    datum_longitude: lon,
                    drift_from_lkp_m: s.datum[0].hypot(s.datum[1]),
                    drift_bearing_deg: bearing(s.datum),
                    axis_deg: s.axis_deg,
                    major_m: s.major_m,
                    minor_m: s.minor_m,
                    in_water: s.released,
                }
            })
            .collect();
        let mut csv = String::from(
            "time_utc,hour,datum_latitude,datum_longitude,drift_from_lkp_m,drift_bearing_deg,axis_deg,major_m,minor_m,in_water\n",
        );
        for r in &rows {
            csv += &format!(
                "{},{:.3},{:.6},{:.6},{:.1},{:.1},{:.1},{:.1},{:.1},{}\n",
                r.time_utc,
                r.hour,
                r.datum_latitude,
                r.datum_longitude,
                r.drift_from_lkp_m,
                r.drift_bearing_deg,
                r.axis_deg,
                r.major_m,
                r.minor_m,
                r.in_water
            );
        }
        put("drift.csv", csv)?;
        let end = d.last_hour();
        let detection = self
            .options
            .get(chosen)
            .and_then(|o| o.detection.last())
            .cloned()
            .unwrap_or_default();
        let mut csv = String::from("particle,latitude,longitude,in_water,detection_probability\n");
        for (i, (p, wet)) in d.positions(end).iter().enumerate() {
            let (lat, lon) = d.plane.geo(*p);
            csv += &format!(
                "{i},{lat:.6},{lon:.6},{wet},{:.4}\n",
                detection.get(i).copied().unwrap_or(0.0)
            );
        }
        put("particles.csv", csv)?;
        let options: Vec<OptionRow> = self
            .options
            .iter()
            .map(|o| OptionRow {
                pattern: o.plan.pattern.name(),
                spacing_m: o.plan.spacing_m,
                area_km2: o.plan.area.size_m2() / 1e6,
                coverage: o.coverage,
                poc: o.poc,
                pod: o.pod,
                pos: o.pos,
            })
            .collect();
        let json = serde_json::json!({
            "format": FORMAT,
            "version": 1,
            "object": { "id": d.object.id, "name": d.object.name },
            "setup": self.setup,
            "searcher": self.searcher,
            "drivers_source": d.source,
            "hour_zero_utc": d.start,
            "chosen": self.options.get(chosen),
            "options": options,
            "drift": rows,
            "warnings": self.warnings,
            "notes": d.notes,
        });
        put(
            "plans.json",
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
        )?;
        put("plan.gpx", self.gpx(chosen))?;
        put("plan.waypoints", self.waypoints(chosen))?;
        Ok(())
    }
}
