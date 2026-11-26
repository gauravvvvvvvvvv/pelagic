//! Test campaigns: run one scene many times with varied seeds (and optionally
//! perturbed sea state and current), measure each run and judge it against
//! acceptance criteria. Every run is a deterministic headless batch, so any
//! failing run can be reproduced exactly from its index.
//!
//! Metrics describe the tracked vehicle (the sensor rig's vehicle, else the
//! first vehicle): navigation error of the noisy position fix against truth,
//! depth excursion from the start, seabed clearance, attitude, speed, sonar
//! coverage and the closest approach between any two vehicles.
use crate::{
    experiment::{Recording, batch},
    math::Vec3,
    ocean::Bathymetry,
    scene::{Scene, world},
};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub const MAX_RUNS: usize = 500;

/// Pass/fail thresholds. `None` disables a check.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Criteria {
    /// Largest allowed |position fix - truth| (m).
    pub max_nav_error_m: Option<f64>,
    /// Largest allowed depth change from the start (m).
    pub max_depth_excursion_m: Option<f64>,
    /// Smallest allowed gap between the vehicle's bounding sphere and the seabed (m).
    pub min_seabed_clearance_m: Option<f64>,
    /// Smallest allowed gap between any two vehicles' bounding spheres (m).
    pub min_vehicle_separation_m: Option<f64>,
    /// Largest allowed roll/pitch tilt from upright (degrees).
    pub max_tilt_deg: Option<f64>,
    /// Smallest allowed mean fraction of sonar beams with a return (0..1).
    pub min_sonar_coverage: Option<f64>,
}

/// How runs differ from each other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Variation {
    pub runs: usize,
    /// Run `i` uses seed `seed_base + i` (waves, marine life, sensor noise).
    pub seed_base: u64,
    /// Significant wave height is scaled by 1 ± this fraction (uniform).
    pub wave_height_jitter: f64,
    /// Current speed is scaled by 1 ± this fraction (uniform).
    pub current_jitter: f64,
}
impl Default for Variation {
    fn default() -> Self {
        Self {
            runs: 20,
            seed_base: 1,
            wave_height_jitter: 0.0,
            current_jitter: 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub samples: usize,
    pub nav_error_max_m: f64,
    pub nav_error_rms_m: f64,
    pub depth_excursion_max_m: f64,
    pub seabed_clearance_min_m: f64,
    /// Infinite when the scene has a single vehicle.
    pub vehicle_separation_min_m: f64,
    pub tilt_max_deg: f64,
    pub speed_max_mps: f64,
    pub distance_m: f64,
    pub sonar_coverage_mean: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    pub index: usize,
    pub seed: u64,
    pub wave_height_m: f64,
    pub current_scale: f64,
    pub metrics: Metrics,
    pub passed: bool,
    pub failures: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub format: String,
    pub engine: String,
    pub build_id: String,
    pub scene: String,
    pub seconds: f64,
    pub criteria: Criteria,
    pub variation: Variation,
    pub runs: Vec<RunResult>,
}
impl Report {
    pub fn passed(&self) -> usize {
        self.runs.iter().filter(|r| r.passed).count()
    }
    pub fn errors(&self) -> usize {
        self.runs.iter().filter(|r| r.error.is_some()).count()
    }
    /// Worst value of each metric over the runs that completed.
    pub fn worst(&self) -> Metrics {
        let ok: Vec<&Metrics> = self
            .runs
            .iter()
            .filter(|r| r.error.is_none())
            .map(|r| &r.metrics)
            .collect();
        let max = |f: fn(&Metrics) -> f64| ok.iter().map(|m| f(m)).fold(f64::NAN, f64::max);
        let min = |f: fn(&Metrics) -> f64| ok.iter().map(|m| f(m)).fold(f64::NAN, f64::min);
        Metrics {
            samples: ok.iter().map(|m| m.samples).min().unwrap_or(0),
            nav_error_max_m: max(|m| m.nav_error_max_m),
            nav_error_rms_m: max(|m| m.nav_error_rms_m),
            depth_excursion_max_m: max(|m| m.depth_excursion_max_m),
            seabed_clearance_min_m: min(|m| m.seabed_clearance_min_m),
            vehicle_separation_min_m: min(|m| m.vehicle_separation_min_m),
            tilt_max_deg: max(|m| m.tilt_max_deg),
            speed_max_mps: max(|m| m.speed_max_mps),
            distance_m: max(|m| m.distance_m),
            sonar_coverage_mean: min(|m| m.sonar_coverage_mean),
        }
    }
    /// One row per run; columns are stable for spreadsheets and CI scripts.
    pub fn csv(&self) -> String {
        let mut out = String::from(
            "index,seed,wave_height_m,current_scale,passed,samples,nav_error_max_m,nav_error_rms_m,depth_excursion_max_m,seabed_clearance_min_m,vehicle_separation_min_m,tilt_max_deg,speed_max_mps,distance_m,sonar_coverage_mean,failures,error\n",
        );
        for r in &self.runs {
            let m = &r.metrics;
            let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
            out += &format!(
                "{},{},{:.4},{:.4},{},{},{:.5},{:.5},{:.5},{:.5},{:.5},{:.4},{:.5},{:.4},{:.4},{},{}\n",
                r.index,
                r.seed,
                r.wave_height_m,
                r.current_scale,
                r.passed,
                m.samples,
                m.nav_error_max_m,
                m.nav_error_rms_m,
                m.depth_excursion_max_m,
                m.seabed_clearance_min_m,
                m.vehicle_separation_min_m,
                m.tilt_max_deg,
                m.speed_max_mps,
                m.distance_m,
                m.sonar_coverage_mean,
                quote(&r.failures.join("; ")),
                quote(r.error.as_deref().unwrap_or("")),
            );
        }
        out
    }
    /// Writes `campaign.json` and `campaign.csv` into `folder` (created).
    pub fn write(&self, folder: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        write_atomic(&folder.join("campaign.json"), &json)?;
        write_atomic(&folder.join("campaign.csv"), self.csv().as_bytes())
    }
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension("partial");
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}

fn unit(seed: u64, salt: u64) -> f64 {
    // SplitMix64 to a uniform value in [-1, 1).
    let mut z = seed ^ salt.wrapping_mul(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

impl Variation {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_RUNS).contains(&self.runs) {
            return Err(format!("Use 1–{MAX_RUNS} runs."));
        }
        if !(0.0..=0.9).contains(&self.wave_height_jitter)
            || !(0.0..=0.9).contains(&self.current_jitter)
        {
            return Err("Jitter fractions must be 0–0.9.".into());
        }
        Ok(())
    }
    /// The exact scene used for run `index`.
    pub fn scene(&self, base: &Scene, index: usize) -> Scene {
        let mut scene = base.clone();
        scene.seed = self.seed_base.wrapping_add(index as u64);
        scene.wave_height =
            (base.wave_height * (1.0 + self.wave_height_jitter * unit(scene.seed, 1))).max(0.0);
        let scale = 1.0 + self.current_jitter * unit(scene.seed, 2);
        scene.current = base.current.map(|c| c * scale);
        scene
    }
    fn current_scale(&self, seed: u64) -> f64 {
        1.0 + self.current_jitter * unit(seed, 2)
    }
}

impl Criteria {
    pub fn judge(&self, m: &Metrics) -> Vec<String> {
        let mut failures = vec![];
        let mut above = |value: f64, limit: Option<f64>, name: &str, unit: &str| {
            // NaN (no data) fails a check that is enabled.
            if let Some(limit) = limit
                && (value.is_nan() || value > limit)
            {
                failures.push(format!("{name} {value:.3}{unit} > {limit}{unit}"));
            }
        };
        above(m.nav_error_max_m, self.max_nav_error_m, "nav error", " m");
        above(
            m.depth_excursion_max_m,
            self.max_depth_excursion_m,
            "depth excursion",
            " m",
        );
        above(m.tilt_max_deg, self.max_tilt_deg, "tilt", "°");
        let mut below = |value: f64, limit: Option<f64>, name: &str, unit: &str| {
            if let Some(limit) = limit
                && (value.is_nan() || value < limit)
            {
                failures.push(format!("{name} {value:.3}{unit} < {limit}{unit}"));
            }
        };
        below(
            m.seabed_clearance_min_m,
            self.min_seabed_clearance_m,
            "seabed clearance",
            " m",
        );
        below(
            m.vehicle_separation_min_m,
            self.min_vehicle_separation_m,
            "vehicle separation",
            " m",
        );
        below(
            m.sonar_coverage_mean,
            self.min_sonar_coverage,
            "sonar coverage",
            "",
        );
        failures
    }
}

/// Measures one recorded run. `bottom` is the scene's seabed.
pub fn measure(run: &Recording, scene: &Scene, bottom: &dyn Bathymetry) -> Metrics {
    let mut m = Metrics {
        seabed_clearance_min_m: f64::INFINITY,
        vehicle_separation_min_m: f64::INFINITY,
        ..Default::default()
    };
    let tracked = |frame: &crate::experiment::RunFrame| {
        frame
            .truth
            .vehicles
            .iter()
            .find(|v| v.id == scene.sensors.entity)
            .or_else(|| frame.truth.vehicles.first())
            .cloned()
    };
    let Some(start) = run.frames.first().and_then(tracked) else {
        return m;
    };
    let mut squared = 0.0;
    let mut fixes = 0usize;
    let mut coverage = 0.0;
    let mut sonar = 0usize;
    let mut last = start.position;
    // Truth position of the tracked vehicle by sample time. A frame carries
    // the latest delivered observation, which was sampled at an earlier
    // truth time (sensor latency), so fixes are compared at their own time.
    let truth: Vec<(f64, Vec3)> = run
        .frames
        .iter()
        .filter_map(|f| tracked(f).map(|v| (f.truth.time, v.position)))
        .collect();
    let mut seen = f64::NEG_INFINITY;
    for frame in &run.frames {
        let Some(v) = tracked(frame) else { continue };
        m.samples += 1;
        m.depth_excursion_max_m = m
            .depth_excursion_max_m
            .max((v.position.z - start.position.z).abs());
        let radius = v.half.length();
        let clearance = v.position.z - radius - bottom.elevation(v.position.x, v.position.y);
        m.seabed_clearance_min_m = m.seabed_clearance_min_m.min(clearance);
        let up = v.orientation.rotate(Vec3::new(0.0, 0.0, 1.0));
        m.tilt_max_deg = m
            .tilt_max_deg
            .max(up.z.clamp(-1.0, 1.0).acos().to_degrees());
        m.speed_max_mps = m.speed_max_mps.max(v.velocity.length());
        m.distance_m += (v.position - last).length();
        last = v.position;
        for (i, a) in frame.truth.vehicles.iter().enumerate() {
            for b in &frame.truth.vehicles[i + 1..] {
                let gap = (a.position - b.position).length() - a.half.length() - b.half.length();
                m.vehicle_separation_min_m = m.vehicle_separation_min_m.min(gap);
            }
        }
        for o in frame.observations.iter().filter(|o| o.vehicle == v.id) {
            if o.time <= seen {
                continue;
            }
            let Some(&(_, actual)) = truth.iter().find(|(t, _)| (t - o.time).abs() < 1e-9) else {
                continue;
            };
            seen = o.time;
            let error = (o.position - actual).length();
            m.nav_error_max_m = m.nav_error_max_m.max(error);
            squared += error * error;
            fixes += 1;
            if !o.beams.is_empty() {
                coverage += o.beams.iter().filter(|b| b.range.is_some()).count() as f64
                    / o.beams.len() as f64;
                sonar += 1;
            }
        }
    }
    // No data is reported as NaN so that an enabled check fails.
    if fixes > 0 {
        m.nav_error_rms_m = (squared / fixes as f64).sqrt();
    } else {
        m.nav_error_max_m = f64::NAN;
        m.nav_error_rms_m = f64::NAN;
    }
    m.sonar_coverage_mean = if sonar > 0 {
        coverage / sonar as f64
    } else {
        f64::NAN
    };
    m
}

/// Runs and measures one campaign member.
pub fn run_one(
    base: &Scene,
    seconds: f64,
    variation: &Variation,
    criteria: &Criteria,
    index: usize,
    cancel: &AtomicBool,
) -> RunResult {
    let scene = variation.scene(base, index);
    let mut result = RunResult {
        index,
        seed: scene.seed,
        wave_height_m: scene.wave_height,
        current_scale: variation.current_scale(scene.seed),
        metrics: Metrics::default(),
        passed: false,
        failures: vec![],
        error: None,
    };
    let measured = scene.validate().and_then(|()| {
        let run = batch(&scene, seconds, cancel, |_, _| {})?;
        let world = world(&scene)?;
        Ok(measure(&run, &scene, world.ocean.bottom.as_ref()))
    });
    match measured {
        Ok(metrics) => {
            result.failures = criteria.judge(&metrics);
            result.passed = result.failures.is_empty();
            result.metrics = metrics;
        }
        Err(e) => result.error = Some(e),
    }
    result
}

/// Runs a whole campaign on up to `threads` worker threads. `progress` is
/// called on the calling thread after each run with (completed, total).
pub fn run(
    base: &Scene,
    seconds: f64,
    variation: &Variation,
    criteria: &Criteria,
    threads: usize,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Report, String> {
    variation.validate()?;
    base.validate()?;
    if !seconds.is_finite() || !(0.1..=120.0).contains(&seconds) {
        return Err("Run duration must be 0.1–120 s.".into());
    }
    let total = variation.runs;
    let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (send, receive) = mpsc::channel();
    let mut runs: Vec<RunResult> = Vec::with_capacity(total);
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, 16).min(total) {
            let send = send.clone();
            let next = next.clone();
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= total || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = run_one(base, seconds, variation, criteria, index, cancel);
                    if send.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(send);
        for result in receive {
            runs.push(result);
            progress(runs.len(), total);
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err("Campaign cancelled.".into());
    }
    runs.sort_by_key(|r| r.index);
    Ok(Report {
        format: "pelagic-campaign".into(),
        engine: env!("CARGO_PKG_VERSION").into(),
        build_id: crate::BUILD_ID.into(),
        scene: base.name.clone(),
        seconds,
        criteria: criteria.clone(),
        variation: variation.clone(),
        runs,
    })
}

/// Worker threads to use by default: all cores but one, at most eight.
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |n| n.get().saturating_sub(1))
        .clamp(1, 8)
}
