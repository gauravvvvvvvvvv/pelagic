//! Oil spill drift and weathering: where a surface spill goes under the wind
//! and current, how much evaporates, and where and when it reaches the
//! shore. A planning and exercise aid.
//!
//! - **Drift.** Each particle is a parcel of oil: it moves with the surface
//!   current plus a wind drift (3 % of the 10 m wind by default, optionally
//!   deflected to the right of the wind in the northern hemisphere) and a
//!   random walk for horizontal eddy diffusion (Δx ~ N(0, 2KΔt)), midpoint
//!   (RK2) in 10-minute steps. Wind and current come from the same
//!   forecast blend as search-and-rescue drift.
//! - **Spreading.** Parcels start inside the Fay (1971) gravity–viscous
//!   radius R = 1.45 (Δ g V² t^1.5 / ν^0.5)^(1/6) ten minutes after release;
//!   diffusion spreads them after that.
//! - **Evaporation.** Fingas's empirical equations, %Ev = (a + b·T)·ln t
//!   (t in minutes, T in °C): published coefficients for five crude oils,
//!   or the generic distillation form [0.165 %D + 0.045 (T − 15)] ln t
//!   from the percentage distilled at 180 °C.
//! - **Shoreline.** With the scene's seabed, a parcel that reaches land
//!   (elevation ≥ 0) strands there.
use crate::{
    life::random,
    math::Det,
    metocean::{format_utc, parse_utc},
    ocean::Bathymetry,
    sar::{self, Drivers, Plane},
    terrain::HeightGrid,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub const FORMAT: &str = "pelagic-spill";
/// Integration step and frame interval (hours).
pub const STEP_H: f64 = 1.0 / 6.0;
const G: f64 = 9.80665;

/// An oil with an empirical evaporation equation %Ev = (a + b·T)·ln t.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Product {
    pub id: &'static str,
    pub name: &'static str,
    /// Typical density (kg/m³), approximate.
    pub density: f64,
    pub a: f64,
    pub b: f64,
}

/// Crude oils with Fingas's published empirical equations (Environment
/// Canada, 2004); densities are typical values.
pub const PRODUCTS: [Product; 5] = [
    Product {
        id: "ANS",
        name: "Alaska North Slope crude (2002)",
        density: 875.0,
        a: 2.86,
        b: 0.045,
    },
    Product {
        id: "ASMB",
        name: "Alberta Sweet Mixed Blend crude",
        density: 840.0,
        a: 3.24,
        b: 0.054,
    },
    Product {
        id: "ARAB-LIGHT",
        name: "Arabian Light crude (2001)",
        density: 860.0,
        a: 2.4,
        b: 0.045,
    },
    Product {
        id: "ARAB-MEDIUM",
        name: "Arabian Medium crude",
        density: 880.0,
        a: 1.89,
        b: 0.045,
    },
    Product {
        id: "ARAB-HEAVY",
        name: "Arabian Heavy crude",
        density: 890.0,
        a: 1.31,
        b: 0.045,
    },
];

/// Evaporated fraction after `minutes` at `temp_c` for coefficients
/// (a, b) of %Ev = (a + b·T)·ln t, capped at 80 %.
pub fn evaporated(a: f64, b: f64, temp_c: f64, minutes: f64) -> f64 {
    if minutes <= 1.0 {
        return 0.0;
    }
    ((a + b * temp_c).max(0.0) * minutes.dln() / 100.0).clamp(0.0, 0.8)
}

/// Generic coefficients from the percentage distilled at 180 °C:
/// %Ev = [0.165 %D + 0.045 (T − 15)] ln t, written as (a, b).
pub fn distillation_coefficients(percent_distilled: f64) -> (f64, f64) {
    (0.165 * percent_distilled - 0.045 * 15.0, 0.045)
}

/// Fay gravity–viscous radius (m) of `volume` m³ after `seconds`.
pub fn fay_radius(volume: f64, density: f64, seconds: f64) -> f64 {
    let delta = ((1025.0 - density) / 1025.0).max(0.01);
    let nu = 1.0e-6_f64;
    1.45 * (delta * G * volume * volume * seconds.powf(1.5) / nu.sqrt()).powf(1.0 / 6.0)
}

/// The spill and the model's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpillSetup {
    /// Product id ([`PRODUCTS`]) or "CUSTOM" with `percent_distilled`.
    pub product: String,
    pub percent_distilled: f64,
    pub latitude: f64,
    pub longitude: f64,
    pub volume_m3: f64,
    /// Release start (hours after the drivers' hour 0) and length (h; 0 is
    /// instantaneous).
    pub release_hour: f64,
    pub release_h: f64,
    /// Hours to follow the oil from the release start.
    pub duration_h: f64,
    pub particles: usize,
    /// Wind drift factor (fraction of the 10 m wind) and its deflection to
    /// the right of the wind (°, negative to the left).
    pub wind_factor: f64,
    pub deflection_deg: f64,
    /// Horizontal eddy diffusivity (m²/s).
    pub diffusivity: f64,
    /// Sea temperature for evaporation (°C).
    pub water_temp_c: f64,
    pub seed: u64,
}
impl Default for SpillSetup {
    fn default() -> Self {
        Self {
            product: "ANS".into(),
            percent_distilled: 20.0,
            latitude: 0.0,
            longitude: 0.0,
            volume_m3: 100.0,
            release_hour: 0.0,
            release_h: 0.0,
            duration_h: 48.0,
            particles: 3000,
            wind_factor: 0.03,
            deflection_deg: 0.0,
            diffusivity: 10.0,
            water_temp_c: 20.0,
            seed: 1,
        }
    }
}
impl SpillSetup {
    pub fn validate(&self) -> Result<(), String> {
        if self.product != "CUSTOM" && PRODUCTS.iter().all(|p| p.id != self.product) {
            return Err(format!("Unknown product '{}'.", self.product));
        }
        if !(-85.0..=85.0).contains(&self.latitude) || !self.longitude.is_finite() {
            return Err("The release needs a valid latitude and longitude.".into());
        }
        let checks = [
            (
                self.percent_distilled,
                0.0,
                100.0,
                "Percentage distilled at 180 °C",
            ),
            (self.volume_m3, 0.001, 1.0e6, "Volume (m³)"),
            (self.release_h, 0.0, 240.0, "Release duration (h)"),
            (self.duration_h, 0.5, 240.0, "Duration (h)"),
            (self.wind_factor, 0.0, 0.1, "Wind drift factor"),
            (self.deflection_deg, -45.0, 45.0, "Deflection (°)"),
            (self.diffusivity, 0.0, 1000.0, "Diffusivity (m²/s)"),
            (self.water_temp_c, -2.0, 40.0, "Water temperature (°C)"),
            (self.release_hour, -1e6, 1e6, "Release time"),
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
    /// (name, density, a, b) of the evaporation equation.
    pub fn oil(&self) -> (String, f64, f64, f64) {
        match PRODUCTS.iter().find(|p| p.id == self.product) {
            Some(p) => (p.name.into(), p.density, p.a, p.b),
            None => {
                let (a, b) = distillation_coefficients(self.percent_distilled);
                (
                    format!(
                        "Crude oil, {:.0} % distilled at 180 °C",
                        self.percent_distilled
                    ),
                    870.0,
                    a,
                    b,
                )
            }
        }
    }
}

/// Land from a height grid, `offset` metres from the release to the grid's
/// frame (the release point's scene coordinates).
#[derive(Clone, Debug)]
pub struct Land {
    pub grid: HeightGrid,
    pub offset: [f64; 2],
}
impl Land {
    pub fn is_land(&self, p: [f64; 2]) -> bool {
        let q = [p[0] + self.offset[0], p[1] + self.offset[1]];
        let b = self.grid.bounds();
        q[0] >= b[0]
            && q[0] <= b[2]
            && q[1] >= b[1]
            && q[1] <= b[3]
            && self.grid.elevation(q[0], q[1]) >= 0.0
    }
}

/// The simulated spill.
#[derive(Clone, Debug)]
pub struct Spill {
    pub setup: SpillSetup,
    pub oil: (String, f64, f64, f64),
    pub plane: Plane,
    pub start: String,
    pub source: String,
    pub first_hour: f64,
    pub step_h: f64,
    /// Positions per frame per parcel (m east/north of the release).
    pub frames: Vec<Vec<[f32; 2]>>,
    /// Release and stranding times (hours; NaN when afloat).
    pub release: Vec<f32>,
    pub stranded: Vec<f32>,
    pub notes: Vec<String>,
}

/// Mass balance and slick at one time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    pub hour: f64,
    pub released_m3: f64,
    pub surface_m3: f64,
    pub evaporated_m3: f64,
    pub stranded_m3: f64,
    /// Area of 100 m cells holding floating oil (km²) and the mean
    /// thickness there (µm).
    pub area_km2: f64,
    pub thickness_um: f64,
    /// Centre of the floating oil (m east/north of the release).
    pub centre: [f64; 2],
}

/// Runs the spill.
pub fn run(
    drivers: &Drivers,
    setup: &SpillSetup,
    land: Option<&Land>,
    cancel: &AtomicBool,
) -> Result<Spill, String> {
    drivers.validate()?;
    setup.validate()?;
    let last = (drivers.hours - 1) as f64;
    let end = setup.release_hour + setup.duration_h;
    if setup.release_hour < 0.0 || end > last + 1e-9 {
        return Err(format!(
            "The spill must lie within the wind and current data ({} to {} UTC).",
            drivers.time_of(0.0).replace('T', " "),
            drivers.time_of(last).replace('T', " ")
        ));
    }
    let plane = Plane {
        latitude: setup.latitude,
        longitude: setup.longitude,
    };
    if let Some(l) = land
        && l.is_land([0.0, 0.0])
    {
        return Err("The release point is on land.".into());
    }
    let oil = setup.oil();
    let n = setup.particles;
    let mut state = setup.seed ^ 0x0011_5911;
    // Instantaneous parcels start within the Fay radius ten minutes on; a
    // continuous release spreads the volume over its duration.
    let rate_volume = if setup.release_h > 0.0 {
        setup.volume_m3 / setup.release_h.max(1.0 / 6.0)
    } else {
        setup.volume_m3
    };
    let r0 = fay_radius(rate_volume.min(setup.volume_m3), oil.1, 600.0).min(5_000.0);
    let mut pos: Vec<[f64; 2]> = (0..n)
        .map(|_| {
            let r = r0 * random(&mut state).sqrt();
            let a = std::f64::consts::TAU * random(&mut state);
            let (s, c) = a.dsin_cos();
            [r * c, r * s]
        })
        .collect();
    let release: Vec<f64> = (0..n)
        .map(|i| setup.release_hour + setup.release_h * (i as f64 + random(&mut state)) / n as f64)
        .collect();
    let mut stranded = vec![f64::NAN; n];
    let mut field = sar::Field::new(drivers, &plane);
    let dt = STEP_H * 3600.0;
    let walk = (2.0 * setup.diffusivity * dt).sqrt();
    let (ds, dc) = (-setup.deflection_deg).to_radians().dsin_cos();
    let velocity = |field: &sar::Field, p: [f64; 2]| {
        let (wind, current) = field.sample(p);
        // Wind drift, rotated by the deflection (clockwise = to the right).
        let w = [wind[0] * dc - wind[1] * ds, wind[0] * ds + wind[1] * dc];
        [
            current[0] + setup.wind_factor * w[0],
            current[1] + setup.wind_factor * w[1],
        ]
    };
    let snapshot = |p: &[[f64; 2]]| -> Vec<[f32; 2]> {
        p.iter().map(|q| [q[0] as f32, q[1] as f32]).collect()
    };
    let steps = (setup.duration_h / STEP_H).ceil() as usize;
    let mut frames = Vec::with_capacity(steps + 1);
    frames.push(snapshot(&pos));
    for step in 0..steps {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".into());
        }
        let t0 = setup.release_hour + step as f64 * STEP_H;
        let t1 = t0 + STEP_H;
        field.at_time(drivers, t0 + 0.5 * STEP_H);
        for i in 0..n {
            if release[i] >= t1 || !stranded[i].is_nan() {
                continue;
            }
            let span = (t1 - release[i].max(t0)) / STEP_H * dt;
            let p = pos[i];
            let v1 = velocity(&field, p);
            let mid = [p[0] + v1[0] * span * 0.5, p[1] + v1[1] * span * 0.5];
            let v2 = velocity(&field, mid);
            let scale = walk * (span / dt).sqrt();
            let next = [
                p[0] + v2[0] * span + sar::gaussian(&mut state) * scale,
                p[1] + v2[1] * span + sar::gaussian(&mut state) * scale,
            ];
            if let Some(l) = land
                && l.is_land(next)
            {
                // Strand at the shore, between the last wet point and land.
                let mut a = p;
                let mut b = next;
                for _ in 0..12 {
                    let m = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                    if l.is_land(m) {
                        b = m;
                    } else {
                        a = m;
                    }
                }
                pos[i] = a;
                stranded[i] = t1;
                continue;
            }
            pos[i] = next;
        }
        frames.push(snapshot(&pos));
    }
    let mut notes = drivers.notes.clone();
    if land.is_none() {
        notes.push(
            "No coastline: oil cannot strand (use a real-world scene to include the shore).".into(),
        );
    }
    if drivers.stations.len() == 1 {
        notes.push("Wind and current from one location, taken as uniform.".into());
    }
    Ok(Spill {
        setup: setup.clone(),
        oil,
        plane,
        start: drivers.start.clone(),
        source: drivers.source.clone(),
        first_hour: setup.release_hour,
        step_h: STEP_H,
        frames,
        release: release.iter().map(|v| *v as f32).collect(),
        stranded: stranded.iter().map(|v| *v as f32).collect(),
        notes,
    })
}

impl Spill {
    pub fn parcels(&self) -> usize {
        self.release.len()
    }
    pub fn last_hour(&self) -> f64 {
        self.first_hour + (self.frames.len() - 1) as f64 * self.step_h
    }
    pub fn time_of(&self, hour: f64) -> String {
        parse_utc(&self.start).map_or_else(
            || format!("hour {hour:.2}"),
            |t| format_utc(t + (hour * 3600.0).round() as i64),
        )
    }
    /// Parcel `i` at `hour`.
    pub fn position(&self, i: usize, hour: f64) -> [f64; 2] {
        let x = ((hour - self.first_hour) / self.step_h).clamp(0.0, (self.frames.len() - 1) as f64);
        let k = (x.floor() as usize).min(self.frames.len().saturating_sub(2));
        let f = x - k as f64;
        let a = self.frames[k][i];
        let b = self.frames[(k + 1).min(self.frames.len() - 1)][i];
        [
            f64::from(a[0]) + f64::from(b[0] - a[0]) * f,
            f64::from(a[1]) + f64::from(b[1] - a[1]) * f,
        ]
    }
    /// State of parcel `i` at `hour`: 0 not released, 1 afloat, 2 stranded.
    pub fn state(&self, i: usize, hour: f64) -> u8 {
        if f64::from(self.release[i]) > hour {
            0
        } else if self.stranded[i].is_nan() || f64::from(self.stranded[i]) > hour {
            1
        } else {
            2
        }
    }
    /// Evaporated fraction of parcel `i` at `hour` (it stops evaporating
    /// once stranded).
    pub fn evaporated(&self, i: usize, hour: f64) -> f64 {
        let until = if self.stranded[i].is_nan() {
            hour
        } else {
            hour.min(f64::from(self.stranded[i]))
        };
        let minutes = (until - f64::from(self.release[i])) * 60.0;
        evaporated(self.oil.2, self.oil.3, self.setup.water_temp_c, minutes)
    }
    pub fn balance(&self, hour: f64) -> Balance {
        let n = self.parcels();
        let v = self.setup.volume_m3 / n as f64;
        let mut b = Balance {
            hour,
            ..Default::default()
        };
        let mut cells = std::collections::HashSet::new();
        let (mut cx, mut cy, mut count) = (0.0, 0.0, 0usize);
        for i in 0..n {
            match self.state(i, hour) {
                0 => continue,
                s => {
                    b.released_m3 += v;
                    let e = self.evaporated(i, hour);
                    b.evaporated_m3 += v * e;
                    if s == 1 {
                        b.surface_m3 += v * (1.0 - e);
                        let p = self.position(i, hour);
                        cells
                            .insert(((p[0] / 100.0).floor() as i64, (p[1] / 100.0).floor() as i64));
                        cx += p[0];
                        cy += p[1];
                        count += 1;
                    } else {
                        b.stranded_m3 += v * (1.0 - e);
                    }
                }
            }
        }
        b.area_km2 = cells.len() as f64 * 0.01;
        if b.area_km2 > 0.0 {
            b.thickness_um = b.surface_m3 / (b.area_km2 * 1e6) * 1e6;
        }
        if count > 0 {
            b.centre = [cx / count as f64, cy / count as f64];
        }
        b
    }
    /// Hourly mass balance from the release.
    pub fn balances(&self) -> Vec<Balance> {
        let mut out = vec![];
        let mut h = self.first_hour;
        while h <= self.last_hour() + 1e-9 {
            out.push(self.balance(h));
            h += 1.0;
        }
        out
    }
    /// First stranding: time (h) and place (m east/north of the release).
    pub fn first_landfall(&self) -> Option<(f64, [f64; 2])> {
        let i = (0..self.parcels())
            .filter(|i| !self.stranded[*i].is_nan())
            .min_by(|a, b| self.stranded[*a].total_cmp(&self.stranded[*b]))?;
        let h = f64::from(self.stranded[i]);
        Some((h, self.position(i, h)))
    }

    pub fn summary(&self) -> String {
        let end = self.balance(self.last_hour());
        let mut t = format!(
            "Pelagic oil spill drift\n\nOil: {}\nRelease: {:.0} m³ at {:.4}°, {:.4}° from {} UTC{}\nWind and current: {}\nModel: wind drift {:.1} %{}, diffusivity {:.0} m²/s, water {:.1} °C, {} parcels\n\nAfter {:.0} h ({} UTC): afloat {:.1} m³, evaporated {:.1} m³, ashore {:.1} m³; slick about {:.2} km², mean {:.1} µm; centre {:.2} km towards {:.0}°\n",
            self.oil.0,
            self.setup.volume_m3,
            self.setup.latitude,
            self.setup.longitude,
            self.time_of(self.setup.release_hour).replace('T', " "),
            if self.setup.release_h > 0.0 {
                format!(" over {:.1} h", self.setup.release_h)
            } else {
                String::new()
            },
            self.source,
            self.setup.wind_factor * 100.0,
            if self.setup.deflection_deg != 0.0 {
                format!(" deflected {:+.0}°", self.setup.deflection_deg)
            } else {
                String::new()
            },
            self.setup.diffusivity,
            self.setup.water_temp_c,
            self.parcels(),
            self.setup.duration_h,
            self.time_of(self.last_hour()).replace('T', " "),
            end.surface_m3,
            end.evaporated_m3,
            end.stranded_m3,
            end.area_km2,
            end.thickness_um,
            end.centre[0].hypot(end.centre[1]) / 1000.0,
            sar::bearing(end.centre)
        );
        match self.first_landfall() {
            Some((h, p)) => {
                let (lat, lon) = self.plane.geo(p);
                t += &format!(
                    "First oil ashore after {:.1} h ({} UTC) at {lat:.4}°, {lon:.4}°\n",
                    h - self.setup.release_hour,
                    self.time_of(h).replace('T', " ")
                );
            }
            None => t += "No oil ashore in this period.\n",
        }
        for n in &self.notes {
            t += &format!("Note: {n}\n");
        }
        t += "\nEvaporation: Fingas empirical equations (Environment Canada). No emulsification, natural dispersion, sinking or response operations are modelled. A planning and exercise aid.\n";
        t
    }

    /// Writes summary.txt, mass_balance.csv, particles.csv (final state)
    /// and slick.geojson (floating and stranded parcels at the end).
    pub fn write(&self, folder: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let put = |name: &str, text: String| {
            std::fs::write(folder.join(name), text).map_err(|e| format!("{name}: {e}"))
        };
        put("summary.txt", self.summary())?;
        let mut csv = String::from(
            "time_utc,hours_since_release,released_m3,afloat_m3,evaporated_m3,ashore_m3,area_km2,thickness_um,centre_latitude,centre_longitude\n",
        );
        for b in self.balances() {
            let (lat, lon) = self.plane.geo(b.centre);
            csv += &format!(
                "{},{:.2},{:.3},{:.3},{:.3},{:.3},{:.4},{:.2},{lat:.6},{lon:.6}\n",
                self.time_of(b.hour),
                b.hour - self.first_hour,
                b.released_m3,
                b.surface_m3,
                b.evaporated_m3,
                b.stranded_m3,
                b.area_km2,
                b.thickness_um
            );
        }
        put("mass_balance.csv", csv)?;
        let end = self.last_hour();
        let mut csv = String::from("parcel,latitude,longitude,state,evaporated_fraction\n");
        let mut afloat = vec![];
        let mut ashore = vec![];
        for i in 0..self.parcels() {
            let (lat, lon) = self.plane.geo(self.position(i, end));
            let state = ["not released", "afloat", "ashore"][self.state(i, end) as usize];
            csv += &format!(
                "{i},{lat:.6},{lon:.6},{state},{:.4}\n",
                self.evaporated(i, end)
            );
            match self.state(i, end) {
                1 => afloat.push(format!("[{lon:.6},{lat:.6}]")),
                2 => ashore.push(format!("[{lon:.6},{lat:.6}]")),
                _ => {}
            }
        }
        put("particles.csv", csv)?;
        put(
            "slick.geojson",
            format!(
                "{{\"type\":\"FeatureCollection\",\"features\":[{{\"type\":\"Feature\",\"properties\":{{\"state\":\"afloat\",\"time_utc\":\"{}\"}},\"geometry\":{{\"type\":\"MultiPoint\",\"coordinates\":[{}]}}}},{{\"type\":\"Feature\",\"properties\":{{\"state\":\"ashore\"}},\"geometry\":{{\"type\":\"MultiPoint\",\"coordinates\":[{}]}}}}]}}\n",
                self.time_of(end),
                afloat.join(","),
                ashore.join(",")
            ),
        )?;
        Ok(())
    }
}
