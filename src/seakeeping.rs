//! Seakeeping and operability: how a vehicle moves in irregular seas, the
//! largest sea it can work in against motion criteria, and when a forecast
//! sea lets it operate.
//!
//! Method (the standard linear approach, computed in the time domain):
//! 1. For each peak period Tp and relative heading, the vehicle runs in a
//!    long-crested JONSWAP sea of a reference significant height (1 m by
//!    default) at its transit speed with its heading held, and the RMS
//!    motions are measured: heave, roll, pitch, vertical acceleration at the
//!    bow (forward perpendicular) and at a working point ("bridge"), felt
//!    lateral acceleration at the working point, and the bow's motion and
//!    velocity relative to the water surface.
//! 2. Responses scale with Hs (linear seakeeping), so each cell gives a
//!    response per metre of Hs and a limiting Hs for every criterion. Bow
//!    emergence, slamming (Ochi: emergence with relative velocity above
//!    0.093·√(g·L)) and deck wetness follow from Rayleigh statistics of the
//!    relative motion.
//! 3. An outlook combines the table with forecast wind sea and swell (their
//!    variances add) hour by hour for a planned course.
use crate::{
    definition::VehicleDefinition,
    entities::Entity,
    math::{Det, Quat, Vec3},
    metocean::{Conditions, Forecast},
    trials::{self, Bench},
    waves::SeaState,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

pub const FORMAT: &str = "pelagic-seakeeping";
const G: f64 = 9.80665;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    /// Significant wave height of the computed seas, m.
    pub reference_height: f64,
    /// Peak periods, s.
    pub periods: Vec<f64>,
    /// Relative headings, degrees: 0 following, 90 beam (from starboard),
    /// 180 head seas.
    pub headings: Vec<f64>,
    /// Transit thrust, fraction of forward capability (0 = no way on).
    pub speed_fraction: f64,
    /// Measured duration per cell after the waves are established, s.
    pub seconds: f64,
    /// Depth for submerged vehicles, m.
    pub operating_depth: f64,
    /// Working point (bridge, crane tip, sensor), body frame, m. `None`
    /// uses a point on deck 20 % of the length forward of amidships.
    pub working_point: Option<Vec3>,
}
impl Default for Setup {
    fn default() -> Self {
        Self {
            reference_height: 1.0,
            periods: vec![4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 12.0, 14.0],
            headings: vec![0.0, 30.0, 60.0, 90.0, 120.0, 150.0, 180.0],
            speed_fraction: 0.6,
            seconds: 300.0,
            operating_depth: 20.0,
            working_point: None,
        }
    }
}
impl Setup {
    pub fn quick() -> Self {
        Self {
            periods: vec![5.0, 8.0, 12.0],
            headings: vec![0.0, 90.0, 180.0],
            seconds: 150.0,
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !(0.1..=5.0).contains(&self.reference_height) {
            return Err("Reference wave height must be 0.1–5 m.".into());
        }
        if self.periods.is_empty()
            || self.periods.len() > 20
            || !self.periods.iter().all(|t| (2.0..=25.0).contains(t))
        {
            return Err("Use 1–20 peak periods of 2–25 s.".into());
        }
        if self.headings.is_empty()
            || self.headings.len() > 13
            || !self.headings.iter().all(|h| (0.0..=180.0).contains(h))
        {
            return Err("Use 1–13 relative headings of 0–180°.".into());
        }
        if !(0.0..=1.0).contains(&self.speed_fraction) {
            return Err("Transit thrust must be 0–100 %.".into());
        }
        if !(30.0..=3600.0).contains(&self.seconds) {
            return Err("Measure for 30–3600 s per sea state.".into());
        }
        if self
            .working_point
            .is_some_and(|p| !p.is_finite() || p.length() > 50.0)
        {
            return Err("The working point must be within 50 m of the centre of mass.".into());
        }
        Ok(())
    }
}

/// Responses in one sea state, per metre of significant wave height.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub heading_deg: f64,
    pub period_s: f64,
    pub speed_mps: f64,
    /// RMS per metre Hs.
    pub heave_m: f64,
    pub roll_deg: f64,
    pub pitch_deg: f64,
    pub bow_vertical_g: f64,
    pub point_vertical_g: f64,
    pub point_lateral_g: f64,
    /// Bow motion and velocity relative to the surface (surface vessels).
    pub bow_relative_m: f64,
    pub bow_relative_mps: f64,
    /// Largest tilt seen during the reference run, degrees.
    pub max_tilt_deg: f64,
}

/// Motion criteria (RMS values and probabilities per wave encounter).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Criteria {
    pub name: String,
    pub max_roll_deg: Option<f64>,
    pub max_pitch_deg: Option<f64>,
    pub max_bow_vertical_g: Option<f64>,
    pub max_point_vertical_g: Option<f64>,
    pub max_point_lateral_g: Option<f64>,
    pub max_slam_probability: Option<f64>,
    pub max_wetness_probability: Option<f64>,
}
impl Criteria {
    /// NORDFORSK (1987) general operability limiting criteria for ships
    /// (RMS values; the working point is the bridge), plus an example for
    /// survey work. Confirm against the standard your programme uses.
    pub fn presets() -> Vec<Criteria> {
        let nordforsk = |name: &str, bow: f64, bridge: f64, lateral: f64, roll: f64| Criteria {
            name: name.into(),
            max_roll_deg: Some(roll),
            max_pitch_deg: None,
            max_bow_vertical_g: Some(bow),
            max_point_vertical_g: Some(bridge),
            max_point_lateral_g: Some(lateral),
            max_slam_probability: Some(0.03),
            max_wetness_probability: Some(0.05),
        };
        vec![
            nordforsk("NORDFORSK 1987 · naval vessel", 0.275, 0.2, 0.1, 4.0),
            nordforsk(
                "NORDFORSK 1987 · merchant ship (L ≤ 100 m)",
                0.275,
                0.15,
                0.12,
                6.0,
            ),
            nordforsk("NORDFORSK 1987 · fast small craft", 0.65, 0.275, 0.1, 4.0),
            Criteria {
                name: "Survey / launch and recovery (example)".into(),
                max_roll_deg: Some(3.0),
                max_pitch_deg: Some(2.0),
                max_bow_vertical_g: None,
                max_point_vertical_g: Some(0.1),
                max_point_lateral_g: Some(0.07),
                max_slam_probability: Some(0.01),
                max_wetness_probability: Some(0.02),
            },
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub format: String,
    pub engine: String,
    pub build_id: String,
    pub vehicle: VehicleDefinition,
    /// "surface" or "submerged".
    pub mode: String,
    pub length_m: f64,
    pub setup: Setup,
    pub environment: trials::Environment,
    pub bow_point: Vec3,
    pub working_point: Vec3,
    /// Bow draft and freeboard at rest (surface vessels), m.
    pub bow_draft_m: f64,
    pub bow_freeboard_m: f64,
    pub cells: Vec<Cell>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

/// Motions in a sea state of known per-metre responses (and Hs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Motions {
    pub roll_deg: f64,
    pub pitch_deg: f64,
    pub heave_m: f64,
    pub bow_vertical_g: f64,
    pub point_vertical_g: f64,
    pub point_lateral_g: f64,
    pub slam_probability: f64,
    pub wetness_probability: f64,
    relative_m: f64,
    relative_mps: f64,
}

impl Report {
    /// Ochi's threshold relative velocity for slamming, m/s.
    fn slam_velocity(&self) -> f64 {
        0.093 * (G * self.length_m).sqrt()
    }
    /// Motions in a sea of significant height `hs` with the responses of `c`.
    pub fn motions(&self, c: &Cell, hs: f64) -> Motions {
        let m = Motions {
            roll_deg: c.roll_deg * hs,
            pitch_deg: c.pitch_deg * hs,
            heave_m: c.heave_m * hs,
            bow_vertical_g: c.bow_vertical_g * hs,
            point_vertical_g: c.point_vertical_g * hs,
            point_lateral_g: c.point_lateral_g * hs,
            relative_m: c.bow_relative_m * hs,
            relative_mps: c.bow_relative_mps * hs,
            ..Motions::default()
        };
        self.with_probabilities(m)
    }
    fn with_probabilities(&self, mut m: Motions) -> Motions {
        if self.mode == "surface" && m.relative_m > 1e-9 {
            let s2 = 2.0 * m.relative_m * m.relative_m;
            m.wetness_probability = (-self.bow_freeboard_m.powi(2) / s2).dexp();
            let v2 = 2.0 * m.relative_mps.max(1e-9).powi(2);
            m.slam_probability =
                (-self.bow_draft_m.powi(2) / s2 - self.slam_velocity().powi(2) / v2).dexp();
        }
        m
    }
    /// Criteria the motions exceed (empty = operable).
    pub fn exceeded(m: &Motions, c: &Criteria) -> Vec<&'static str> {
        let mut out = vec![];
        let checks = [
            ("roll", m.roll_deg, c.max_roll_deg),
            ("pitch", m.pitch_deg, c.max_pitch_deg),
            ("bow acceleration", m.bow_vertical_g, c.max_bow_vertical_g),
            (
                "vertical acceleration",
                m.point_vertical_g,
                c.max_point_vertical_g,
            ),
            (
                "lateral acceleration",
                m.point_lateral_g,
                c.max_point_lateral_g,
            ),
            ("slamming", m.slam_probability, c.max_slam_probability),
            (
                "deck wetness",
                m.wetness_probability,
                c.max_wetness_probability,
            ),
        ];
        for (name, value, limit) in checks {
            if limit.is_some_and(|l| value > l) {
                out.push(name);
            }
        }
        out
    }
    /// Largest Hs that meets every criterion in this cell, and the
    /// criterion that limits it.
    pub fn limiting_height(&self, cell: &Cell, criteria: &Criteria) -> (f64, &'static str) {
        let mut best = (f64::INFINITY, "none");
        let mut take = |hs: f64, name: &'static str| {
            if hs.is_finite() && hs < best.0 {
                best = (hs.max(0.0), name);
            }
        };
        let linear = [
            ("roll", cell.roll_deg, criteria.max_roll_deg),
            ("pitch", cell.pitch_deg, criteria.max_pitch_deg),
            (
                "bow acceleration",
                cell.bow_vertical_g,
                criteria.max_bow_vertical_g,
            ),
            (
                "vertical acceleration",
                cell.point_vertical_g,
                criteria.max_point_vertical_g,
            ),
            (
                "lateral acceleration",
                cell.point_lateral_g,
                criteria.max_point_lateral_g,
            ),
        ];
        for (name, per_metre, limit) in linear {
            if let Some(l) = limit
                && per_metre > 1e-12
            {
                take(l / per_metre, name);
            }
        }
        if self.mode == "surface" && cell.bow_relative_m > 1e-12 {
            let a = cell.bow_relative_m;
            let b = cell.bow_relative_mps.max(1e-12);
            if let Some(p) = criteria
                .max_wetness_probability
                .filter(|p| *p > 0.0 && *p < 1.0)
            {
                take(
                    self.bow_freeboard_m / a / (-2.0 * p.dln()).sqrt(),
                    "deck wetness",
                );
            }
            if let Some(p) = criteria
                .max_slam_probability
                .filter(|p| *p > 0.0 && *p < 1.0)
            {
                let k = (self.bow_draft_m / a).powi(2) + (self.slam_velocity() / b).powi(2);
                take((k / (-2.0 * p.dln())).sqrt(), "slamming");
            }
        }
        best
    }
    /// Per-metre responses at (period, relative heading), interpolated
    /// bilinearly in the table (clamped to its range).
    pub fn response(&self, period: f64, heading: f64) -> Cell {
        let mut ts: Vec<f64> = self.cells.iter().map(|c| c.period_s).collect();
        let mut hs: Vec<f64> = self.cells.iter().map(|c| c.heading_deg).collect();
        for v in [&mut ts, &mut hs] {
            v.sort_by(f64::total_cmp);
            v.dedup();
        }
        let bracket = |list: &[f64], x: f64| {
            let x = x.clamp(list[0], list[list.len() - 1]);
            let i = list.iter().rposition(|v| *v <= x).unwrap_or(0);
            let j = (i + 1).min(list.len() - 1);
            let w = if j == i {
                0.0
            } else {
                (x - list[i]) / (list[j] - list[i])
            };
            (list[i], list[j], w)
        };
        let (t0, t1, wt) = bracket(&ts, period);
        let (h0, h1, wh) = bracket(&hs, heading);
        let find = |t: f64, h: f64| {
            self.cells
                .iter()
                .find(|c| c.period_s == t && c.heading_deg == h)
                .cloned()
                .unwrap_or_default()
        };
        let corners = [
            (find(t0, h0), (1.0 - wt) * (1.0 - wh)),
            (find(t1, h0), wt * (1.0 - wh)),
            (find(t0, h1), (1.0 - wt) * wh),
            (find(t1, h1), wt * wh),
        ];
        let mut out = Cell {
            heading_deg: heading,
            period_s: period,
            ..Cell::default()
        };
        for (c, w) in corners {
            out.speed_mps += c.speed_mps * w;
            out.heave_m += c.heave_m * w;
            out.roll_deg += c.roll_deg * w;
            out.pitch_deg += c.pitch_deg * w;
            out.bow_vertical_g += c.bow_vertical_g * w;
            out.point_vertical_g += c.point_vertical_g * w;
            out.point_lateral_g += c.point_lateral_g * w;
            out.bow_relative_m += c.bow_relative_m * w;
            out.bow_relative_mps += c.bow_relative_mps * w;
            out.max_tilt_deg = out.max_tilt_deg.max(c.max_tilt_deg);
        }
        out
    }

    /// Motions in forecast conditions on a course (compass degrees): wind
    /// sea and swell are combined by adding their variances.
    pub fn forecast_motions(&self, c: &Conditions, course: f64) -> Option<Motions> {
        let mut parts = vec![];
        let peak = |p: Option<f64>, m: Option<f64>| p.or(m.map(|t| t / 0.9));
        match c.wind_wave_height {
            Some(h) => {
                parts.push((
                    h,
                    peak(c.wind_wave_peak_period, c.wind_wave_period),
                    c.wind_wave_direction,
                ));
                if let Some(s) = c.swell_height {
                    parts.push((
                        s,
                        peak(c.swell_peak_period, c.swell_period),
                        c.swell_direction,
                    ));
                }
            }
            None => parts.push((c.wave_height?, peak(None, c.wave_period), c.wave_direction)),
        }
        let mut total = Motions::default();
        let mut any = false;
        for (hs, period, from) in parts {
            let (Some(period), Some(from)) = (period, from) else {
                continue;
            };
            if hs <= 0.0 {
                continue;
            }
            any = true;
            // Relative heading: 180 when the waves travel against the course.
            let travel = from + 180.0;
            let relative = ((travel - course + 540.0).rem_euclid(360.0) - 180.0).abs();
            let m = self.motions(&self.response(period, relative), hs);
            let add = |a: f64, b: f64| (a * a + b * b).sqrt();
            total.roll_deg = add(total.roll_deg, m.roll_deg);
            total.pitch_deg = add(total.pitch_deg, m.pitch_deg);
            total.heave_m = add(total.heave_m, m.heave_m);
            total.bow_vertical_g = add(total.bow_vertical_g, m.bow_vertical_g);
            total.point_vertical_g = add(total.point_vertical_g, m.point_vertical_g);
            total.point_lateral_g = add(total.point_lateral_g, m.point_lateral_g);
            total.relative_m = add(total.relative_m, m.relative_m);
            total.relative_mps = add(total.relative_mps, m.relative_mps);
        }
        any.then(|| self.with_probabilities(total))
    }

    /// Hour-by-hour operability for a forecast on a course.
    pub fn outlook(
        &self,
        forecast: &Forecast,
        course: f64,
        criteria: &Criteria,
    ) -> Vec<OutlookHour> {
        forecast
            .hours
            .iter()
            .map(|c| {
                let motions = self.forecast_motions(c, course);
                let exceeded = motions
                    .as_ref()
                    .map(|m| Self::exceeded(m, criteria))
                    .unwrap_or_default();
                OutlookHour {
                    time: c.time.clone(),
                    wave_height: c.wave_height,
                    operable: motions.is_some().then_some(exceeded.is_empty()),
                    limited_by: exceeded.iter().map(|s| s.to_string()).collect(),
                    motions,
                }
            })
            .collect()
    }

    pub fn summary(&self, criteria: &Criteria) -> String {
        let mut s = format!(
            "Seakeeping: {} ({}, L {:.2} m) · reference Hs {:.2} m · criteria: {}\n",
            self.vehicle.name, self.mode, self.length_m, self.setup.reference_height, criteria.name
        );
        let mut headings: Vec<f64> = self.cells.iter().map(|c| c.heading_deg).collect();
        headings.sort_by(f64::total_cmp);
        headings.dedup();
        s += "Limiting Hs (m) by peak period (rows) and relative heading (columns, 180 = head seas)\n";
        s += "  Tp  ";
        for h in &headings {
            s += &format!("{h:>7.0}");
        }
        s += "\n";
        let mut periods: Vec<f64> = self.cells.iter().map(|c| c.period_s).collect();
        periods.sort_by(f64::total_cmp);
        periods.dedup();
        for t in &periods {
            s += &format!("{t:5.1} ");
            for h in &headings {
                let cell = self
                    .cells
                    .iter()
                    .find(|c| c.period_s == *t && c.heading_deg == *h);
                // Beyond 20 m no real sea limits the vehicle.
                match cell.map(|c| self.limiting_height(c, criteria).0) {
                    Some(v) if v <= 20.0 => s += &format!("{v:>7.2}"),
                    Some(_) => s += "    >20",
                    None => s += "      —",
                }
            }
            s += "\n";
        }
        for w in &self.warnings {
            s += &format!("Warning: {w}\n");
        }
        s
    }

    /// Writes seakeeping.json, responses.csv, limits.csv and summary.txt.
    pub fn write(&self, folder: &std::path::Path, criteria: &Criteria) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let put = |name: &str, bytes: &[u8]| {
            let path = folder.join(name);
            let temp = path.with_extension("partial");
            std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&temp, &path).map_err(|e| e.to_string())
        };
        put(
            "seakeeping.json",
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )?;
        let mut csv = String::from(
            "period_s,heading_deg,speed_mps,heave_m_per_m,roll_deg_per_m,pitch_deg_per_m,bow_vertical_g_per_m,point_vertical_g_per_m,point_lateral_g_per_m,bow_relative_m_per_m,bow_relative_mps_per_m,limiting_hs_m,limited_by\n",
        );
        for c in &self.cells {
            let (hs, by) = self.limiting_height(c, criteria);
            csv += &format!(
                "{},{},{:.4},{:.5},{:.5},{:.5},{:.6},{:.6},{:.6},{:.5},{:.5},{},{}\n",
                c.period_s,
                c.heading_deg,
                c.speed_mps,
                c.heave_m,
                c.roll_deg,
                c.pitch_deg,
                c.bow_vertical_g,
                c.point_vertical_g,
                c.point_lateral_g,
                c.bow_relative_m,
                c.bow_relative_mps,
                if hs.is_finite() {
                    format!("{hs:.3}")
                } else {
                    String::new()
                },
                by
            );
        }
        put("responses.csv", csv.as_bytes())?;
        put("summary.txt", self.summary(criteria).as_bytes())
    }
}

/// One forecast hour judged against the criteria.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutlookHour {
    pub time: String,
    pub wave_height: Option<f64>,
    /// `None` when the forecast hour has no usable sea state.
    pub operable: Option<bool>,
    pub limited_by: Vec<String>,
    pub motions: Option<Motions>,
}

/// Running mean and variance (Welford).
#[derive(Default, Clone, Copy)]
struct Stat {
    n: f64,
    mean: f64,
    m2: f64,
}
impl Stat {
    fn push(&mut self, x: f64) {
        self.n += 1.0;
        let d = x - self.mean;
        self.mean += d / self.n;
        self.m2 += d * (x - self.mean);
    }
    fn rms(&self) -> f64 {
        if self.n > 1.0 {
            (self.m2 / self.n).sqrt()
        } else {
            0.0
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cell(
    bench: &Bench,
    setup: &Setup,
    seed: u64,
    depth: f64,
    period: f64,
    heading: f64,
    bow: Vec3,
    point: Vec3,
) -> Result<Cell, String> {
    let mut rig = bench.rig(
        &bench.entity,
        SeaState::default(),
        false,
        Vec3::new(0.0, 0.0, bench.start_z),
        Quat::IDENTITY,
    )?;
    let thrust = setup.speed_fraction * bench.forward;
    let speed = if thrust > 0.0 {
        bench.approach(&mut rig, thrust).0
    } else {
        0.0
    };
    // Waves arrive once the vehicle is on its steady transit.
    rig.world.ocean.waves = SeaState::spectrum(
        setup.reference_height,
        period,
        heading.to_radians(),
        0.0,
        depth,
        seed,
    )
    .map_err(str::to_string)?;
    let ramp = rig.time() + (5.0 * period).max(30.0);
    let end = ramp + setup.seconds;
    let dt = 1.0 / 60.0;
    let world_point = |rig: &trials::Rig, r: Vec3| {
        let v = rig.vehicle();
        let p = v.position + v.orientation.rotate(r);
        let u = v.velocity + v.orientation.rotate(v.angular_velocity.cross(r));
        (p, u)
    };
    let mut last: Option<(Vec3, Vec3, Vec3)> = None;
    let (mut heave, mut roll, mut pitch) = (Stat::default(), Stat::default(), Stat::default());
    let (mut bow_az, mut az, mut ay) = (Stat::default(), Stat::default(), Stat::default());
    let (mut rel, mut rel_v) = (Stat::default(), Stat::default());
    let mut previous_relative: Option<f64> = None;
    let mut tilt: f64 = 0.0;
    while rig.time() < end {
        let steer = rig.hold(0.0);
        rig.step(thrust, steer);
        let (bp, bu) = world_point(&rig, bow);
        let (_, pu) = world_point(&rig, point);
        let v = rig.vehicle();
        if !v.position.is_finite() {
            return Err("The vehicle became unstable in waves.".into());
        }
        let up = v.orientation.rotate(Vec3::new(0.0, 0.0, 1.0)).z;
        let accelerations =
            last.map(|(_, lb, lp)| ((bu - lb) * (1.0 / dt), (pu - lp) * (1.0 / dt)));
        last = Some((bp, bu, pu));
        let t = rig.time();
        if t <= ramp {
            continue;
        }
        tilt = tilt.max(up.clamp(-1.0, 1.0).dacos().to_degrees());
        let (r, p, _) = trials::attitude(v.orientation);
        heave.push(v.position.z);
        roll.push(r.to_degrees());
        pitch.push(p.to_degrees());
        if let Some((ab, ap)) = accelerations {
            bow_az.push(ab.z / G);
            az.push(ap.z / G);
            // Felt (specific) force sideways in the body frame.
            let felt = v
                .orientation
                .conjugate()
                .rotate(ap + Vec3::new(0.0, 0.0, G));
            ay.push(felt.y / G);
        }
        if bench.surface {
            let surface = rig.world.ocean.surface(bp.x, bp.y, t).height;
            let relative = bp.z - surface;
            rel.push(relative);
            if let Some(before) = previous_relative {
                rel_v.push((relative - before) / dt);
            }
            previous_relative = Some(relative);
        }
    }
    let h = setup.reference_height;
    Ok(Cell {
        heading_deg: heading,
        period_s: period,
        speed_mps: speed,
        heave_m: heave.rms() / h,
        roll_deg: roll.rms() / h,
        pitch_deg: pitch.rms() / h,
        bow_vertical_g: bow_az.rms() / h,
        point_vertical_g: az.rms() / h,
        point_lateral_g: ay.rms() / h,
        bow_relative_m: rel.rms() / h,
        bow_relative_mps: rel_v.rms() / h,
        max_tilt_deg: tilt,
    })
}

/// Computes the response table for `vehicle` on up to `threads` threads.
pub fn run(
    vehicle: &Entity,
    environment: &trials::Environment,
    setup: &Setup,
    threads: usize,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Report, String> {
    setup.validate()?;
    let trial_setup = trials::Setup {
        operating_depth: setup.operating_depth,
        ..trials::Setup::default()
    };
    let bench = Bench::new(vehicle, environment, &trial_setup)?;
    let h = bench.entity.model.half;
    let (bow, point, draft, freeboard) = if bench.surface {
        // Waterline in the body frame is −start_z above the centre.
        let waterline = -bench.start_z;
        (
            Vec3::new(h.x, 0.0, waterline),
            setup
                .working_point
                .unwrap_or(Vec3::new(0.2 * 2.0 * h.x, 0.0, h.z)),
            h.z + bench.start_z,
            h.z - bench.start_z,
        )
    } else {
        (
            Vec3::new(h.x, 0.0, 0.0),
            setup
                .working_point
                .unwrap_or(Vec3::new(0.4 * h.x, 0.0, 0.0)),
            0.0,
            0.0,
        )
    };
    let tasks: Vec<(f64, f64)> = setup
        .periods
        .iter()
        .flat_map(|t| setup.headings.iter().map(move |h| (*t, *h)))
        .collect();
    let total = tasks.len();
    let next = Arc::new(AtomicUsize::new(0));
    let (send, receive) = mpsc::channel();
    let mut cells: Vec<Option<Result<Cell, String>>> = (0..total).map(|_| None).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, 16).min(total) {
            let send = send.clone();
            let next = next.clone();
            let (bench, tasks) = (&bench, &tasks);
            scope.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= total || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let (t, hd) = tasks[i];
                    let result = cell(
                        bench,
                        setup,
                        environment.seed.wrapping_add(i as u64),
                        environment.water_depth,
                        t,
                        hd,
                        bow,
                        point,
                    );
                    if send.send((i, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(send);
        let mut done = 0;
        for (i, result) in receive {
            cells[i] = Some(result);
            done += 1;
            progress(done, total);
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err("Seakeeping run cancelled.".into());
    }
    let mut report = Report {
        format: FORMAT.into(),
        engine: format!("pelagic {}", env!("CARGO_PKG_VERSION")),
        build_id: crate::BUILD_ID.into(),
        vehicle: VehicleDefinition::from_entity(vehicle),
        mode: if bench.surface { "surface" } else { "submerged" }.into(),
        length_m: bench.length,
        setup: setup.clone(),
        environment: environment.clone(),
        bow_point: bow,
        working_point: point,
        bow_draft_m: draft,
        bow_freeboard_m: freeboard,
        cells: vec![],
        notes: vec![
            format!(
                "Long-crested JONSWAP seas (γ 3.3) of Hs {:.2} m; responses scale linearly with Hs.",
                setup.reference_height
            ),
            format!(
                "Transit at {:.0} % thrust with heading held; statistics over {:.0} s after the waves build up.",
                setup.speed_fraction * 100.0,
                setup.seconds
            ),
            "Accelerations are RMS at the bow (forward end, waterline) and at the working point; lateral is the felt (specific) force, including gravity from roll.".into(),
            "Slamming (Ochi: emergence with relative velocity above 0.093·√(g·L)) and deck wetness use Rayleigh statistics of the bow's motion relative to the surface.".into(),
        ],
        warnings: vec![],
    };
    for (task, result) in tasks.iter().zip(cells) {
        match result {
            Some(Ok(c)) => report.cells.push(c),
            Some(Err(e)) => report
                .warnings
                .push(format!("Tp {:.1} s, heading {:.0}°: {e}", task.0, task.1)),
            None => {}
        }
    }
    if report.cells.is_empty() {
        return Err("No sea state could be computed.".into());
    }
    if let Some(worst) = report
        .cells
        .iter()
        .max_by(|a, b| a.max_tilt_deg.total_cmp(&b.max_tilt_deg))
        .filter(|c| c.max_tilt_deg > 25.0)
    {
        report.warnings.push(format!(
            "Large motions in the reference sea (tilt {:.0}° at Tp {:.1} s, heading {:.0}°): linear scaling to other heights is approximate there.",
            worst.max_tilt_deg, worst.period_s, worst.heading_deg
        ));
    }
    let m = &vehicle.model;
    if m.drag.is_none() || m.angular_drag.is_none() || m.inertia.is_none() {
        report.warnings.push(
            "Inertia or drag uses the legacy mass-scaled defaults; use Estimate in Details or measured coefficients.".into(),
        );
    }
    Ok(report)
}
