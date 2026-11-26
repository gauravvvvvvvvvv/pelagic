//! Seabed survey planning and coverage: survey lines for a multibeam
//! echosounder or a side-scan sonar over the scene's seabed, and the
//! coverage, gaps and depth uncertainty they achieve — on the ideal plan and
//! when the selected vehicle runs the lines with its own physics in the
//! scene's sea and current.
//!
//! Standards: IHO S-44 Edition 6.2.0 (2024), Table 1 — maximum total
//! vertical uncertainty TVU(d) = √(a² + (b·d)²) at 95 %, feature detection
//! sizes and the required bathymetric coverage per survey order.
//!
//! Method:
//! - **Swath.** A multibeam's usable swath is the smallest of its swath
//!   angle, the angle its maximum slant range reaches, and the angle beyond
//!   which the sounding uncertainty exceeds the order's TVU. Sounding
//!   uncertainty (1σ) at beam angle θ and height h above the seabed:
//!   √(σ₀² + (k·h)² + (h·tanθ·σ_θ)²) — fixed (heave, draft, tide), range
//!   (sound speed) and pointing (attitude, refraction) parts; ×1.96 for 95 %.
//!   A side-scan covers from a blind zone under the vehicle out to its range
//!   on each side and measures no depth.
//! - **Lines.** Parallel lines on the area's axis; each spacing adapts to
//!   the narrowest swath along the neighbouring lines (shallow water narrows
//!   a multibeam swath), with the chosen overlap, and halves for 200 %.
//! - **Coverage.** Edge beams are ray-cast onto the seabed from the
//!   transducer's actual pose; the swath between successive samples is
//!   rasterised on a grid aligned with the area, counting passes per cell
//!   and keeping each cell's best TVU.
use crate::{
    entities::Entity,
    helm::Helm,
    math::{Det, Quat, Vec3},
    ocean::Bathymetry,
    sar::{Area, towards},
    scene::Scene,
    terrain::HeightGrid,
    trials::{self, Bench},
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub const FORMAT: &str = "pelagic-survey";
/// Simulation step of scene worlds (s).
const DT: f64 = 1.0 / 60.0;
/// Least water under a surface transducer, or least altitude, to survey (m).
const MIN_CLEARANCE: f64 = 0.3;

/// A survey order from IHO S-44 Ed. 6.2.0 Table 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Order {
    pub name: &'static str,
    /// Total horizontal uncertainty: fixed (m) + % of depth.
    pub thu_m: f64,
    pub thu_percent: f64,
    /// TVU parameters a (m) and b.
    pub tvu_a: f64,
    pub tvu_b: f64,
    /// Cubic feature size to detect (m), and % of depth beyond 40 m.
    pub feature_m: Option<f64>,
    pub feature_percent_beyond_40: Option<f64>,
    /// Required bathymetric coverage (%).
    pub coverage_percent: f64,
}
impl Order {
    /// Maximum allowable TVU (m, 95 %) at depth `d`.
    pub fn tvu(&self, d: f64) -> f64 {
        (self.tvu_a * self.tvu_a + (self.tvu_b * d).powi(2)).sqrt()
    }
    /// Maximum allowable THU (m, 95 %) at depth `d`.
    pub fn thu(&self, d: f64) -> f64 {
        self.thu_m + self.thu_percent * 0.01 * d
    }
    /// Feature size to detect at depth `d` (m).
    pub fn feature(&self, d: f64) -> Option<f64> {
        let size = self.feature_m?;
        Some(match self.feature_percent_beyond_40 {
            Some(p) if d > 40.0 => p * 0.01 * d,
            _ => size,
        })
    }
    /// Passes needed over every point (1 for 100 %, 2 for 200 %).
    pub fn passes(&self) -> u8 {
        (self.coverage_percent / 100.0).round().max(1.0) as u8
    }
}

/// IHO S-44 Ed. 6.2.0 (October 2024), Table 1.
pub const ORDERS: [Order; 5] = [
    Order {
        name: "Order 2",
        thu_m: 20.0,
        thu_percent: 10.0,
        tvu_a: 1.0,
        tvu_b: 0.023,
        feature_m: None,
        feature_percent_beyond_40: None,
        coverage_percent: 5.0,
    },
    Order {
        name: "Order 1b",
        thu_m: 5.0,
        thu_percent: 5.0,
        tvu_a: 0.5,
        tvu_b: 0.013,
        feature_m: None,
        feature_percent_beyond_40: None,
        coverage_percent: 5.0,
    },
    Order {
        name: "Order 1a",
        thu_m: 5.0,
        thu_percent: 5.0,
        tvu_a: 0.5,
        tvu_b: 0.013,
        feature_m: Some(2.0),
        feature_percent_beyond_40: Some(10.0),
        coverage_percent: 100.0,
    },
    Order {
        name: "Special Order",
        thu_m: 2.0,
        thu_percent: 0.0,
        tvu_a: 0.25,
        tvu_b: 0.0075,
        feature_m: Some(1.0),
        feature_percent_beyond_40: None,
        coverage_percent: 100.0,
    },
    Order {
        name: "Exclusive Order",
        thu_m: 1.0,
        thu_percent: 0.0,
        tvu_a: 0.15,
        tvu_b: 0.0075,
        feature_m: Some(0.5),
        feature_percent_beyond_40: None,
        coverage_percent: 200.0,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SonarKind {
    Multibeam,
    SideScan,
}

/// The survey sensor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sonar {
    pub kind: SonarKind,
    /// Multibeam: total swath angle (degrees).
    pub swath_deg: f64,
    /// Multibeam: maximum slant range; side-scan: range per side (m).
    pub range_m: f64,
    pub ping_hz: f64,
    /// Multibeam soundings per ping (equiangular).
    pub beams: usize,
    /// Uncertainty (1σ): fixed (m), range as a fraction of slant range,
    /// beam pointing (degrees).
    pub sigma_fixed_m: f64,
    pub sigma_range: f64,
    pub sigma_angle_deg: f64,
    /// Side-scan blind zone each side, as a multiple of the altitude.
    pub blind_zone: f64,
}
impl Default for Sonar {
    fn default() -> Self {
        Self::multibeam()
    }
}
impl Sonar {
    /// A shallow-water multibeam: 120° swath, 256 beams, 10 Hz.
    pub fn multibeam() -> Self {
        Self {
            kind: SonarKind::Multibeam,
            swath_deg: 120.0,
            range_m: 200.0,
            ping_hz: 10.0,
            beams: 256,
            sigma_fixed_m: 0.05,
            sigma_range: 0.0015,
            sigma_angle_deg: 0.1,
            blind_zone: 1.0,
        }
    }
    /// A side-scan sonar with 75 m range per side.
    pub fn side_scan() -> Self {
        Self {
            kind: SonarKind::SideScan,
            range_m: 75.0,
            ping_hz: 10.0,
            ..Self::multibeam()
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let checks = [
            (self.swath_deg, 1.0, 175.0, "Swath angle (°)"),
            (self.range_m, 1.0, 12_000.0, "Range (m)"),
            (self.ping_hz, 0.05, 100.0, "Ping rate (Hz)"),
            (self.sigma_fixed_m, 0.0, 10.0, "Fixed uncertainty (m)"),
            (self.sigma_range, 0.0, 0.1, "Range uncertainty"),
            (self.sigma_angle_deg, 0.0, 10.0, "Pointing uncertainty (°)"),
            (self.blind_zone, 0.0, 10.0, "Blind zone"),
        ];
        for (v, lo, hi, name) in checks {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{name} must be between {lo} and {hi}."));
            }
        }
        if !(2..=4096).contains(&self.beams) {
            return Err("Use 2 to 4096 beams.".into());
        }
        Ok(())
    }
    /// Sounding uncertainty at 95 % for height `h` above the seabed and beam
    /// angle `theta` (radians from vertical).
    pub fn tvu(&self, h: f64, theta: f64) -> f64 {
        let s = self.sigma_angle_deg.to_radians();
        1.96 * (self.sigma_fixed_m.powi(2)
            + (self.sigma_range * h).powi(2)
            + (h * theta.dtan() * s).powi(2))
        .sqrt()
    }
    /// Largest beam angle (radians) whose TVU stays within `allowed` at
    /// height `h`; None when even the nadir beam exceeds it.
    pub fn tvu_angle(&self, h: f64, allowed: f64) -> Option<f64> {
        let a = allowed / 1.96;
        let room = a * a - self.sigma_fixed_m.powi(2) - (self.sigma_range * h).powi(2);
        if room < 0.0 {
            return None;
        }
        let s = self.sigma_angle_deg.to_radians();
        Some(if s * h <= 1e-12 {
            std::f64::consts::FRAC_PI_2
        } else {
            (room.sqrt() / (h * s)).datan2(1.0)
        })
    }
}

/// Where the sensor is carried.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Platform {
    /// An underwater vehicle following the seabed at `altitude_m`; else a
    /// surface vessel with the transducer `draft_m` below the surface.
    pub underwater: bool,
    pub altitude_m: f64,
    pub draft_m: f64,
}
impl Default for Platform {
    fn default() -> Self {
        Self {
            underwater: false,
            altitude_m: 10.0,
            draft_m: 0.5,
        }
    }
}

/// A survey to plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    /// Survey area; lines run along its axis (compass bearing).
    pub area: Area,
    pub sonar: Sonar,
    pub platform: Platform,
    /// Index into [`ORDERS`].
    pub order: usize,
    /// Overlap between neighbouring swaths (fraction).
    pub overlap: f64,
    /// Survey speed through the water (m/s).
    pub speed_mps: f64,
    /// Coverage grid cell (m); 0 chooses one.
    pub cell_m: f64,
    /// Straight run before each line (m).
    pub run_in_m: f64,
}
impl Default for Setup {
    fn default() -> Self {
        Self {
            area: Area {
                centre: [0.0, 0.0],
                axis_deg: 0.0,
                length_m: 300.0,
                width_m: 200.0,
                round: false,
            },
            sonar: Sonar::multibeam(),
            platform: Platform::default(),
            order: 2,
            overlap: 0.1,
            speed_mps: 2.5,
            cell_m: 0.0,
            run_in_m: 50.0,
        }
    }
}
impl Setup {
    pub fn validate(&self) -> Result<(), String> {
        self.sonar.validate()?;
        let a = &self.area;
        let checks = [
            (a.length_m, 5.0, 50_000.0, "Area length (m)"),
            (a.width_m, 5.0, 50_000.0, "Area width (m)"),
            (self.overlap, 0.0, 0.9, "Overlap"),
            (self.speed_mps, 0.1, 20.0, "Survey speed (m/s)"),
            (self.cell_m, 0.0, 100.0, "Grid cell (m)"),
            (self.run_in_m, 0.0, 2000.0, "Run-in (m)"),
            (self.platform.altitude_m, 0.5, 500.0, "Altitude (m)"),
            (self.platform.draft_m, 0.0, 30.0, "Transducer draft (m)"),
        ];
        for (v, lo, hi, name) in checks {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{name} must be between {lo} and {hi}."));
            }
        }
        if !a.centre.iter().all(|v| v.is_finite()) || !a.axis_deg.is_finite() {
            return Err("The survey area needs a finite centre and heading.".into());
        }
        if self.order >= ORDERS.len() {
            return Err("Unknown survey order.".into());
        }
        Ok(())
    }
    pub fn order(&self) -> &'static Order {
        &ORDERS[self.order.min(ORDERS.len() - 1)]
    }
    /// Shallowest water the platform can survey (m).
    pub fn min_depth(&self) -> f64 {
        MIN_CLEARANCE
            + if self.platform.underwater {
                self.platform.altitude_m
            } else {
                self.platform.draft_m
            }
    }
    /// Coverage grid cell size for this area and sensor.
    pub fn cell(&self, seabed: &HeightGrid) -> f64 {
        if self.cell_m > 0.0 {
            return self.cell_m;
        }
        let a = &self.area;
        let by_count = (a.length_m * a.width_m / 1.5e6).sqrt();
        let depth = -seabed.elevation(a.centre[0], a.centre[1]);
        let h = if self.platform.underwater {
            self.platform.altitude_m
        } else {
            (depth - self.platform.draft_m).max(1.0)
        };
        (h / 15.0).max(by_count).clamp(0.25, 20.0)
    }
}

/// The scene's seabed as a height grid (elevation, negative below the sea).
pub fn seabed(scene: &Scene) -> HeightGrid {
    scene
        .grid
        .as_ref()
        .filter(|_| scene.terrain == 3)
        .map(|g| (**g).clone())
        .unwrap_or_else(|| HeightGrid::procedural(scene.terrain, scene.bottom_depth))
}

/// Line heading (compass) along the depth contours of the area: lines run
/// across the mean slope, so the swath stays about the same width.
pub fn contour_heading(seabed: &HeightGrid, area: &Area) -> f64 {
    let (mut gx, mut gy) = (0.0, 0.0);
    let n = 12;
    for i in 0..n {
        for j in 0..n {
            let u = (i as f64 + 0.5) / n as f64 - 0.5;
            let v = (j as f64 + 0.5) / n as f64 - 0.5;
            let (a, r) = area.axes();
            let p = [
                area.centre[0] + a[0] * u * area.length_m + r[0] * v * area.width_m,
                area.centre[1] + a[1] * u * area.length_m + r[1] * v * area.width_m,
            ];
            let nrm = seabed.normal(p[0], p[1]);
            gx += nrm.x;
            gy += nrm.y;
        }
    }
    if gx.hypot(gy) < 1e-9 {
        return area.axis_deg;
    }
    // The normal leans down-slope; contours run perpendicular to it.
    (crate::sar::bearing([gx, gy]) + 90.0).rem_euclid(180.0)
}

/// One survey line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub start: [f64; 2],
    pub end: [f64; 2],
    /// Offset across the area from its centre line (m).
    pub offset_m: f64,
    /// Narrowest reach of the swath to each side of the area axis
    /// (towards −across, +across) along the line (m).
    pub reach_m: [f64; 2],
}

/// A swath sample: nadir and edge points on the seabed.
#[derive(Clone, Copy, Debug)]
struct Swath {
    transducer: Vec3,
    nadir: [f64; 2],
    /// Port and starboard outer edges, and (side-scan) inner edges.
    port: [f64; 2],
    starboard: [f64; 2],
    port_inner: [f64; 2],
    starboard_inner: [f64; 2],
}

/// First seabed hit along a ray within `range` (m).
fn cast(seabed: &HeightGrid, from: Vec3, dir: Vec3, range: f64, step: f64) -> Option<Vec3> {
    let mut previous = 0.0;
    let mut s = step;
    while s <= range + step {
        let s_now = s.min(range);
        let p = from + dir * s_now;
        if p.z <= seabed.elevation(p.x, p.y) {
            let (mut lo, mut hi) = (previous, s_now);
            for _ in 0..12 {
                let mid = 0.5 * (lo + hi);
                let q = from + dir * mid;
                if q.z <= seabed.elevation(q.x, q.y) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(from + dir * hi);
        }
        if s_now >= range {
            break;
        }
        previous = s_now;
        s += step;
    }
    None
}

/// The swath from a transducer at `at` with `orientation` (body X forward,
/// Y to port, Z up).
fn swath(seabed: &HeightGrid, setup: &Setup, at: Vec3, orientation: Quat) -> Option<Swath> {
    let sonar = &setup.sonar;
    let ground = seabed.elevation(at.x, at.y);
    let h = at.z - ground;
    if h <= 0.05 {
        return None;
    }
    let step = (h / 20.0).clamp(0.1, 5.0);
    let beam = |theta: f64| {
        // theta > 0 to port.
        let (s, c) = theta.dsin_cos();
        orientation.rotate(Vec3::new(0.0, s, -c))
    };
    let nadir = cast(seabed, at, beam(0.0), sonar.range_m.max(h * 1.01), step)?;
    let flat = |p: Vec3| [p.x, p.y];
    let edge = |limit: f64, sign: f64| -> [f64; 2] {
        // The widest angle that reaches the seabed within range.
        let mut theta = limit;
        for _ in 0..8 {
            if let Some(p) = cast(seabed, at, beam(sign * theta), sonar.range_m, step) {
                return flat(p);
            }
            theta *= 0.8;
        }
        flat(nadir)
    };
    match sonar.kind {
        SonarKind::Multibeam => {
            let half = (sonar.swath_deg * 0.5).to_radians();
            let range = if h < sonar.range_m {
                (h / sonar.range_m).dacos()
            } else {
                0.0
            };
            // Beams beyond the order's TVU are still recorded (raw data);
            // each cell is judged by its own beam angle.
            let limit = half.min(range.max(0.0)).max(0.0);
            let port = edge(limit, 1.0);
            let starboard = edge(limit, -1.0);
            Some(Swath {
                transducer: at,
                nadir: flat(nadir),
                port,
                starboard,
                port_inner: flat(nadir),
                starboard_inner: flat(nadir),
            })
        }
        SonarKind::SideScan => {
            let reach = (sonar.range_m * sonar.range_m - h * h).max(0.0).sqrt();
            if reach <= 0.0 {
                return None;
            }
            let gap = (sonar.blind_zone * h).min(reach);
            let across = orientation.rotate(Vec3::new(0.0, 1.0, 0.0));
            let len = across.x.hypot(across.y).max(1e-9);
            let side = [across.x / len, across.y / len];
            let n = flat(nadir);
            let at_d = |d: f64| [n[0] + side[0] * d, n[1] + side[1] * d];
            Some(Swath {
                transducer: at,
                nadir: n,
                port: at_d(reach),
                starboard: at_d(-reach),
                port_inner: at_d(gap),
                starboard_inner: at_d(-gap),
            })
        }
    }
}

/// Coverage on a grid aligned with the survey area.
#[derive(Clone, Debug, PartialEq)]
pub struct Coverage {
    pub area: Area,
    /// Nominal cell size, and the exact cell length along and across the
    /// axis (the area divides into whole cells).
    pub cell: f64,
    pub cell_u: f64,
    pub cell_v: f64,
    /// Cells along the axis (columns) and across it (rows).
    pub cols: usize,
    pub rows: usize,
    /// Passes per cell: any data, and data within the order's TVU.
    pub raw: Vec<u8>,
    pub passes: Vec<u8>,
    /// Best TVU (m, 95 %) per cell; NaN where none.
    pub tvu: Vec<f32>,
    /// Water depth at each cell centre (m).
    pub depth: Vec<f32>,
    /// Land, or water too shallow for the platform: left out of the
    /// coverage figures.
    pub dry: Vec<bool>,
    last_raw: Vec<u32>,
    last_ok: Vec<u32>,
}

impl Coverage {
    fn new(seabed: &HeightGrid, area: Area, cell: f64, min_depth: f64) -> Self {
        let cols = ((area.length_m / cell).round() as usize).max(1);
        let rows = ((area.width_m / cell).round() as usize).max(1);
        let n = cols * rows;
        let mut c = Self {
            area,
            cell,
            cell_u: area.length_m / cols as f64,
            cell_v: area.width_m / rows as f64,
            cols,
            rows,
            raw: vec![0; n],
            passes: vec![0; n],
            tvu: vec![f32::NAN; n],
            depth: vec![0.0; n],
            dry: vec![false; n],
            last_raw: vec![u32::MAX; n],
            last_ok: vec![u32::MAX; n],
        };
        for j in 0..rows {
            for i in 0..cols {
                let p = c.centre(i, j);
                let depth = -seabed.elevation(p[0], p[1]);
                c.depth[j * cols + i] = depth as f32;
                c.dry[j * cols + i] = depth < min_depth;
            }
        }
        c
    }
    /// World position of cell (i, j)'s centre.
    pub fn centre(&self, i: usize, j: usize) -> [f64; 2] {
        let (a, r) = self.area.axes();
        let u = -self.area.length_m * 0.5 + (i as f64 + 0.5) * self.cell_u;
        let v = -self.area.width_m * 0.5 + (j as f64 + 0.5) * self.cell_v;
        [
            self.area.centre[0] + a[0] * u + r[0] * v,
            self.area.centre[1] + a[1] * u + r[1] * v,
        ]
    }
    /// Grid coordinates (in cells) of a world point.
    fn local(&self, p: [f64; 2]) -> [f64; 2] {
        let (a, r) = self.area.axes();
        let d = [p[0] - self.area.centre[0], p[1] - self.area.centre[1]];
        [
            ((d[0] * a[0] + d[1] * a[1]) + self.area.length_m * 0.5) / self.cell_u,
            ((d[0] * r[0] + d[1] * r[1]) + self.area.width_m * 0.5) / self.cell_v,
        ]
    }

    /// Marks the swath between two samples of line `line`.
    fn mark(&mut self, setup: &Setup, line: u32, a: &Swath, b: &Swath) {
        let quads: Vec<[[f64; 2]; 4]> = match setup.sonar.kind {
            SonarKind::Multibeam => vec![[a.port, a.starboard, b.starboard, b.port]],
            SonarKind::SideScan => vec![
                [a.port_inner, a.port, b.port, b.port_inner],
                [
                    a.starboard,
                    a.starboard_inner,
                    b.starboard_inner,
                    b.starboard,
                ],
            ],
        };
        let order = setup.order();
        let multibeam = setup.sonar.kind == SonarKind::Multibeam;
        for quad in quads {
            let q: Vec<[f64; 2]> = quad.iter().map(|p| self.local(*p)).collect();
            let (mut i0, mut i1, mut j0, mut j1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for p in &q {
                i0 = i0.min(p[0]);
                i1 = i1.max(p[0]);
                j0 = j0.min(p[1]);
                j1 = j1.max(p[1]);
            }
            if i1 < 0.0 || j1 < 0.0 || i0 > self.cols as f64 || j0 > self.rows as f64 {
                continue;
            }
            let i0 = (i0.floor().max(0.0)) as usize;
            let j0 = (j0.floor().max(0.0)) as usize;
            let i1 = (i1.ceil() as usize).min(self.cols);
            let j1 = (j1.ceil() as usize).min(self.rows);
            let inside = |x: f64, y: f64| {
                let tri = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
                    let d1 = (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]);
                    let d2 = (r[0] - q[0]) * (y - q[1]) - (r[1] - q[1]) * (x - q[0]);
                    let d3 = (p[0] - r[0]) * (y - r[1]) - (p[1] - r[1]) * (x - r[0]);
                    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
                    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
                    !(neg && pos)
                };
                tri(q[0], q[1], q[2]) || tri(q[0], q[2], q[3])
            };
            // Nadir segment for beam angles.
            let (na, nb) = (a.nadir, b.nadir);
            let seg = [nb[0] - na[0], nb[1] - na[1]];
            let seg2 = (seg[0] * seg[0] + seg[1] * seg[1]).max(1e-12);
            for j in j0..j1 {
                for i in i0..i1 {
                    if !inside(i as f64 + 0.5, j as f64 + 0.5) {
                        continue;
                    }
                    let k = j * self.cols + i;
                    if self.last_raw[k] != line {
                        self.last_raw[k] = line;
                        self.raw[k] = self.raw[k].saturating_add(1);
                    }
                    if !multibeam {
                        if self.last_ok[k] != line {
                            self.last_ok[k] = line;
                            self.passes[k] = self.passes[k].saturating_add(1);
                        }
                        continue;
                    }
                    let c = self.centre(i, j);
                    let t = (((c[0] - na[0]) * seg[0] + (c[1] - na[1]) * seg[1]) / seg2)
                        .clamp(0.0, 1.0);
                    let lateral = (c[0] - na[0] - seg[0] * t).hypot(c[1] - na[1] - seg[1] * t);
                    let depth = f64::from(self.depth[k]);
                    let z = a.transducer.z + (b.transducer.z - a.transducer.z) * t;
                    let h = (z + depth).max(0.05);
                    let tvu = setup.sonar.tvu(h, lateral.datan2(h));
                    if self.tvu[k].is_nan() || tvu < f64::from(self.tvu[k]) {
                        self.tvu[k] = tvu as f32;
                    }
                    if tvu <= order.tvu(depth) && self.last_ok[k] != line {
                        self.last_ok[k] = line;
                        self.passes[k] = self.passes[k].saturating_add(1);
                    }
                }
            }
        }
    }

    /// Coverage figures for an order.
    pub fn stats(&self, order: &Order) -> CoverageStats {
        let wet = self.dry.iter().filter(|d| !**d).count();
        let n = wet.max(1) as f64;
        let water: Vec<f64> = self
            .depth
            .iter()
            .zip(&self.dry)
            .filter(|(_, dry)| !**dry || wet == 0)
            .map(|(d, _)| f64::from(*d))
            .collect();
        let need = order.passes();
        let share = |f: &dyn Fn(usize) -> bool| {
            (0..self.passes.len())
                .filter(|k| !self.dry[*k] && f(*k))
                .count() as f64
                / n
                * 100.0
        };
        let covered = share(&|k| self.passes[k] >= 1);
        let multiple = share(&|k| self.passes[k] >= need);
        let raw = share(&|k| self.raw[k] >= 1);
        let tvus: Vec<f64> = self
            .tvu
            .iter()
            .filter(|v| v.is_finite())
            .map(|v| f64::from(*v))
            .collect();
        let tvu_mean = if tvus.is_empty() {
            None
        } else {
            Some(tvus.iter().sum::<f64>() / tvus.len() as f64)
        };
        // Gaps: 4-connected groups of cells without a compliant pass.
        let mut seen = vec![false; self.passes.len()];
        let (mut gaps, mut largest) = (0usize, 0usize);
        let mut stack = vec![];
        for start in 0..self.passes.len() {
            if seen[start] || self.dry[start] || self.passes[start] >= need {
                continue;
            }
            let mut size = 0;
            stack.push(start);
            seen[start] = true;
            while let Some(k) = stack.pop() {
                size += 1;
                let (i, j) = (k % self.cols, k / self.cols);
                let mut push = |kk: usize| {
                    if !seen[kk] && !self.dry[kk] && self.passes[kk] < need {
                        seen[kk] = true;
                        stack.push(kk);
                    }
                };
                if i > 0 {
                    push(k - 1);
                }
                if i + 1 < self.cols {
                    push(k + 1);
                }
                if j > 0 {
                    push(k - self.cols);
                }
                if j + 1 < self.rows {
                    push(k + self.cols);
                }
            }
            gaps += 1;
            largest = largest.max(size);
        }
        CoverageStats {
            covered_percent: covered,
            required_passes: need,
            required_percent: order.coverage_percent,
            multiple_percent: multiple,
            raw_percent: raw,
            meets_order: if order.coverage_percent < 100.0 {
                covered >= order.coverage_percent
            } else {
                multiple >= 99.5
            },
            gaps,
            largest_gap_m2: largest as f64 * self.cell_u * self.cell_v,
            unworkable_percent: (self.dry.len() - wet) as f64 / self.dry.len().max(1) as f64
                * 100.0,
            tvu_mean_m: tvu_mean,
            // Water depths over the workable area (all cells if none).
            min_depth_m: water.iter().fold(f64::MAX, |m, d| m.min(*d)),
            max_depth_m: water.iter().fold(f64::MIN, |m, d| m.max(*d)),
        }
    }
}

/// Coverage figures for one survey order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoverageStats {
    /// Share of the area with at least one pass within the order's TVU (%).
    pub covered_percent: f64,
    /// Passes the order needs (1, or 2 for 200 %) and the share reaching it.
    pub required_passes: u8,
    pub required_percent: f64,
    pub multiple_percent: f64,
    /// Share with any data, regardless of uncertainty (%).
    pub raw_percent: f64,
    pub meets_order: bool,
    /// Gaps (holidays) short of the required passes: count and largest (m²).
    pub gaps: usize,
    pub largest_gap_m2: f64,
    /// Share of the area that is land or too shallow for the platform (%).
    pub unworkable_percent: f64,
    pub tvu_mean_m: Option<f64>,
    pub min_depth_m: f64,
    pub max_depth_m: f64,
}

/// Plans survey lines across the area.
pub fn plan(seabed: &HeightGrid, setup: &Setup) -> Result<Vec<Line>, String> {
    setup.validate()?;
    let area = &setup.area;
    let (a, r) = area.axes();
    let passes = f64::from(setup.order().passes());
    let samples = 40usize;
    let at = |u: f64, s: f64| {
        [
            area.centre[0] + a[0] * s + r[0] * u,
            area.centre[1] + a[1] * s + r[1] * u,
        ]
    };
    let heading = (90.0 - area.axis_deg).to_radians();
    let level = Quat::from_rotation_vector(Vec3::new(0.0, 0.0, heading));
    // Narrowest reach towards −across and +across (and the blind zone)
    // along a line at offset u.
    // Narrowest reach towards −across and +across (and the blind zone)
    // along a line at offset u, over the stretches the platform can work
    // (land, water too shallow and, for a multibeam, depths where even
    // its nadir beam misses the order's TVU are left out). None when no
    // stretch is workable.
    let reach = |u: f64| -> Option<([f64; 2], f64)> {
        let mut out = [f64::MAX, f64::MAX];
        let mut gap: f64 = 0.0;
        let mut workable = 0;
        for k in 0..samples {
            let s = (k as f64 / (samples - 1) as f64 - 0.5) * area.length_m;
            let p = at(u, s);
            let ground = seabed.elevation(p[0], p[1]);
            let z = if setup.platform.underwater {
                ground + setup.platform.altitude_m
            } else {
                -setup.platform.draft_m
            };
            let h = z - ground;
            if h < MIN_CLEARANCE {
                continue;
            }
            let Some(sw) = swath(seabed, setup, Vec3::new(p[0], p[1], z), level) else {
                continue;
            };
            let d = |q: [f64; 2]| (q[0] - p[0]) * r[0] + (q[1] - p[1]) * r[1];
            // Body +Y (port) faces −across when the line runs along +axis.
            let (port, starboard) = (d(sw.port), d(sw.starboard));
            let mut minus = -port.min(starboard);
            let mut plus = port.max(starboard);
            if setup.sonar.kind == SonarKind::Multibeam {
                // Only the part within the order's TVU counts for spacing.
                let Some(t) = setup.sonar.tvu_angle(h, setup.order().tvu(-ground)) else {
                    continue;
                };
                let width = h * t.dtan();
                minus = minus.min(width);
                plus = plus.min(width);
            }
            out[0] = out[0].min(minus);
            out[1] = out[1].min(plus);
            gap = gap.max(d(sw.port_inner).abs().max(d(sw.starboard_inner).abs()));
            workable += 1;
        }
        (workable > 0).then_some((out, gap))
    };
    let half = area.width_m * 0.5;
    // Where nothing is workable, step across in small increments.
    let probe = (area.width_m / 200.0).max(0.5);
    let mut lines = vec![];
    let mut u = -half;
    let first = loop {
        if let Some((r, _)) = reach(u) {
            break r;
        }
        u += probe;
        if u > half {
            return Err(format!(
                "Nothing in the area can be surveyed to {}: it is land, too shallow for the platform, or too deep for the sonar's range or uncertainty.",
                setup.order().name
            ));
        }
    };
    let edge = u;
    // First line: its swath (and, for 200 %, the next one's too) must reach
    // the near edge.
    let first_offset = |r: f64| {
        let reach = r * (1.0 - setup.overlap);
        edge + reach - (passes - 1.0) * 2.0 * reach / passes
    };
    if setup.sonar.kind == SonarKind::Multibeam {
        u = first_offset(first[0]);
        for _ in 0..3 {
            if let Some((r0, _)) = reach(u) {
                u = first_offset(r0[0]);
            }
        }
    }
    let need = passes as usize;
    // Workable stretches of a line at offset u (along-axis intervals):
    // land and water too shallow for the platform are cut out.
    let workable = |u: f64| -> Vec<(f64, f64)> {
        let step = (area.length_m / 400.0).max(1.0);
        let n = (area.length_m / step).ceil() as usize;
        let min_depth = setup.min_depth() + 0.5;
        let mut out: Vec<(f64, f64)> = vec![];
        let mut open: Option<f64> = None;
        for k in 0..=n {
            let s = (-0.5 * area.length_m + k as f64 * step).min(0.5 * area.length_m);
            let p = at(u, s);
            let ok = -seabed.elevation(p[0], p[1]) >= min_depth;
            match (ok, open) {
                (true, None) => open = Some(s),
                (false, Some(s0)) => {
                    out.push((s0, s - step));
                    open = None;
                }
                _ => {}
            }
        }
        if let Some(s0) = open {
            out.push((s0, 0.5 * area.length_m));
        }
        // Bridge short gaps; drop slivers.
        let mut merged: Vec<(f64, f64)> = vec![];
        for (a, b) in out {
            match merged.last_mut() {
                Some(last) if a - last.1 < 3.0 * step => last.1 = b,
                _ => merged.push((a, b)),
            }
        }
        merged.retain(|(a, b)| b - a >= (4.0 * step).max(10.0));
        merged
    };
    // Each row (offset) and its reach, for the far-edge test.
    let mut rows: Vec<(f64, [f64; 2])> = vec![];
    let mut guard = 0;
    let mut last_step = probe;
    loop {
        guard += 1;
        if guard > 5000 {
            return Err("Over 5000 lines: enlarge the swath or shrink the area.".into());
        }
        let Some((r_here, gap)) = reach(u) else {
            // An unworkable stretch (land): move on without a line.
            if u >= half - 1e-6 {
                break;
            }
            u = (u + last_step.min(probe * 10.0)).min(half);
            continue;
        };
        let dir = if rows.len().is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let mut stretches = workable(u);
        if dir < 0.0 {
            stretches.reverse();
        }
        for (s0, s1) in stretches {
            let (a, b) = if dir > 0.0 { (s0, s1) } else { (s1, s0) };
            lines.push(Line {
                start: at(u, a),
                end: at(u, b),
                offset_m: u,
                reach_m: r_here,
            });
        }
        rows.push((u, r_here));
        let done = match setup.sonar.kind {
            // The far edge is reached by the last `need` rows.
            SonarKind::Multibeam => {
                rows.len() >= need && {
                    let (o, r) = rows[rows.len() - need];
                    o + r[1] >= half
                }
            }
            SonarKind::SideScan => u >= half - 1e-6,
        } || u >= half - 1e-6;
        if done {
            break;
        }
        // Next spacing: adapt to the neighbour's reach (iterate).
        let mut next = u + 1.0;
        let mut step = 1.0;
        for _ in 0..4 {
            let (r_next, _) = reach(next).unwrap_or((r_here, gap));
            step = match setup.sonar.kind {
                SonarKind::Multibeam => (r_here[1] + r_next[0]) * (1.0 - setup.overlap) / passes,
                // Each blind zone is covered by the neighbours' outer swath.
                SonarKind::SideScan => {
                    (r_here[1].min(r_next[0]) - gap) * (1.0 - setup.overlap) / passes
                }
            }
            .max(0.5);
            next = u + step;
        }
        last_step = step;
        if next > half {
            // The last line sits on the boundary.
            next = half.max(u + step * 0.25);
        }
        u = next;
    }
    if lines.is_empty() {
        return Err("No workable survey line in the area.".into());
    }
    Ok(lines)
}

/// One run over the lines: the ideal plan, or a simulated vehicle.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub coverage: Coverage,
    pub stats: CoverageStats,
    /// Track (t s, x, y, z) every second (simulated runs).
    pub track: Vec<[f64; 4]>,
    /// Cross-track error on survey lines (t s, m, + to the left) every
    /// second (simulated runs).
    pub cross_track: Vec<[f64; 2]>,
    /// Line km surveyed, hours taken (including turns).
    pub line_km: f64,
    pub hours: f64,
    /// Simulated runs: cross-track error RMS and maximum, roll and pitch
    /// RMS on line, and lines completed.
    pub cross_track_rms_m: f64,
    pub cross_track_max_m: f64,
    pub roll_rms_deg: f64,
    pub pitch_rms_deg: f64,
    pub lines_done: usize,
    pub notes: Vec<String>,
}

/// Coverage of the lines flown exactly (level platform, on the line).
pub fn ideal(seabed: &HeightGrid, setup: &Setup, lines: &[Line]) -> Run {
    let cell = setup.cell(seabed);
    let mut coverage = Coverage::new(seabed, setup.area, cell, setup.min_depth());
    let step = (cell * 0.7).max(0.2);
    let mut line_m = 0.0;
    for (index, line) in lines.iter().enumerate() {
        let d = [line.end[0] - line.start[0], line.end[1] - line.start[1]];
        let len = d[0].hypot(d[1]);
        line_m += len;
        let yaw = d[1].datan2(d[0]);
        let q = Quat::from_rotation_vector(Vec3::new(0.0, 0.0, yaw));
        let n = (len / step).ceil().max(1.0) as usize;
        let mut previous: Option<Swath> = None;
        for k in 0..=n {
            let f = k as f64 / n as f64;
            let p = [line.start[0] + d[0] * f, line.start[1] + d[1] * f];
            let ground = seabed.elevation(p[0], p[1]);
            let z = if setup.platform.underwater {
                ground + setup.platform.altitude_m
            } else {
                -setup.platform.draft_m
            };
            let now = swath(seabed, setup, Vec3::new(p[0], p[1], z), q);
            if let (Some(a), Some(b)) = (previous, now) {
                coverage.mark(setup, index as u32, &a, &b);
            }
            previous = now;
        }
    }
    let turns = lines
        .windows(2)
        .map(|w| {
            (w[1].start[0] - w[0].end[0]).hypot(w[1].start[1] - w[0].end[1]) + 2.0 * setup.run_in_m
        })
        .sum::<f64>();
    let stats = coverage.stats(setup.order());
    Run {
        coverage,
        stats,
        track: vec![],
        cross_track: vec![],
        line_km: line_m / 1000.0,
        hours: (line_m + turns) / setup.speed_mps / 3600.0,
        cross_track_rms_m: 0.0,
        cross_track_max_m: 0.0,
        roll_rms_deg: 0.0,
        pitch_rms_deg: 0.0,
        lines_done: lines.len(),
        notes: vec![],
    }
}

fn wrap(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// Runs the lines with `vehicle`'s physics in the scene's sea, current and
/// seabed: speed control, integral line-of-sight steering, terrain
/// following for underwater vehicles, and the swath from the vehicle's
/// actual pose. `progress(fraction)` reports the share of lines done.
pub fn simulate(
    scene: &Scene,
    vehicle: &Entity,
    setup: &Setup,
    lines: &[Line],
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<Run, String> {
    setup.validate()?;
    if lines.is_empty() {
        return Err("Plan the survey lines first.".into());
    }
    let seabed = seabed(scene);
    let env = trials::Environment::from_scene(scene);
    let first = &lines[0];
    let first_dir = {
        let d = [first.end[0] - first.start[0], first.end[1] - first.start[1]];
        let l = d[0].hypot(d[1]).max(1e-9);
        [d[0] / l, d[1] / l]
    };
    let start = [
        first.start[0] - first_dir[0] * setup.run_in_m,
        first.start[1] - first_dir[1] * setup.run_in_m,
    ];
    let start_depth = -seabed.elevation(start[0], start[1]);
    let operating = (start_depth - setup.platform.altitude_m).max(1.0);
    let bench = Bench::new(
        vehicle,
        &env,
        &trials::Setup {
            operating_depth: operating.min(env.water_depth - vehicle.model.half.length() - 1.5),
            ..trials::Setup::quick()
        },
    )?;
    let mut notes = vec![];
    if bench.surface == setup.platform.underwater {
        notes.push(if bench.surface {
            format!(
                "{} floats, so it was run as a surface vessel with the transducer at its keel.",
                vehicle.name
            )
        } else {
            format!(
                "{} does not float, so it was run underwater at {:.1} m altitude.",
                vehicle.name, setup.platform.altitude_m
            )
        });
    }
    if bench.forward <= 0.0 {
        return Err(format!("{} has no forward thrust.", vehicle.name));
    }
    // The scene with only the survey vehicle.
    let mut s = scene.clone();
    s.vehicle = false;
    s.entities.clear();
    s.school.count = 0;
    s.particles = 0;
    let mut world = crate::scene::world(&s)?;
    let mut v = bench.entity.spawn();
    let yaw0 = first_dir[1].datan2(first_dir[0]);
    let z0 = if bench.surface {
        bench.start_z
    } else {
        seabed.elevation(start[0], start[1]) + setup.platform.altitude_m
    };
    v.position = Vec3::new(start[0], start[1], z0);
    v.orientation = Quat::from_rotation_vector(Vec3::new(0.0, 0.0, yaw0));
    let current = Vec3::new(scene.current[0], scene.current[1], scene.current[2]);
    v.velocity = current;
    world.add_vehicle(v);
    let mut rig = bench.rig_in(world, current);
    let mut helm = Helm::new(&bench, scene.density, false);
    let half = bench.entity.model.half;
    let keel = Vec3::new(0.0, 0.0, -half.z);
    // Waypoint legs: run-in → line end (logged), then a racetrack turn.
    struct Leg {
        from: [f64; 2],
        to: [f64; 2],
        line: Option<usize>,
    }
    let mut legs = vec![];
    for (i, line) in lines.iter().enumerate() {
        let d = [line.end[0] - line.start[0], line.end[1] - line.start[1]];
        let l = d[0].hypot(d[1]).max(1e-9);
        let u = [d[0] / l, d[1] / l];
        let run_in = [
            line.start[0] - u[0] * setup.run_in_m,
            line.start[1] - u[1] * setup.run_in_m,
        ];
        if let Some(previous) = legs.last().map(|l: &Leg| l.to) {
            legs.push(Leg {
                from: previous,
                to: run_in,
                line: None,
            });
        }
        legs.push(Leg {
            from: run_in,
            to: line.start,
            line: None,
        });
        legs.push(Leg {
            from: line.start,
            to: line.end,
            line: Some(i),
        });
        let run_out = [
            line.end[0] + u[0] * setup.run_in_m,
            line.end[1] + u[1] * setup.run_in_m,
        ];
        legs.push(Leg {
            from: line.end,
            to: run_out,
            line: None,
        });
    }
    let length = 2.0 * half.x;
    let lookahead = (3.0 * length).max(15.0);
    let mut speed_sum = 0.0;
    let mut cross_integral = 0.0f64;
    let cell = setup.cell(&seabed);
    let mut coverage = Coverage::new(&seabed, setup.area, cell, setup.min_depth());
    let planned: f64 = lines
        .iter()
        .map(|l| (l.end[0] - l.start[0]).hypot(l.end[1] - l.start[1]))
        .sum::<f64>()
        + legs
            .iter()
            .filter(|l| l.line.is_none())
            .map(|l| (l.to[0] - l.from[0]).hypot(l.to[1] - l.from[1]))
            .sum::<f64>();
    let limit_s = (planned / setup.speed_mps * 2.5).max(600.0);
    let mut previous: Option<(usize, Swath)> = None;
    let (mut xte2, mut xte_max, mut roll2, mut pitch2, mut on_line_n) =
        (0.0, 0.0f64, 0.0, 0.0, 0usize);
    let mut track = vec![];
    let mut cross_track = vec![];
    let mut leg = 0usize;
    let mut tick = 0u64;
    let mut line_m = 0.0;
    let mut lines_done = 0usize;
    while leg < legs.len() {
        if rig.time() > limit_s {
            notes.push(format!(
                "Stopped after {:.1} h with {} of {} lines done: the vehicle could not keep to the plan.",
                rig.time() / 3600.0,
                lines_done,
                lines.len()
            ));
            break;
        }
        if tick.is_multiple_of(600) {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled.".into());
            }
            progress(leg as f64 / legs.len() as f64);
        }
        let l = &legs[leg];
        let v = rig.vehicle();
        let p = [v.position.x, v.position.y];
        let d = [l.to[0] - l.from[0], l.to[1] - l.from[1]];
        let len = d[0].hypot(d[1]).max(1e-6);
        let u = [d[0] / len, d[1] / len];
        let along = (p[0] - l.from[0]) * u[0] + (p[1] - l.from[1]) * u[1];
        // Positive to the left of the leg.
        let cross = -(p[0] - l.from[0]) * u[1] + (p[1] - l.from[1]) * u[0];
        // Survey lines are run to their end; turns start a turning radius
        // early, so the vehicle rounds the corner instead of overshooting.
        let early = if l.line.is_some() {
            0.0
        } else {
            lookahead.min(0.5 * len)
        };
        if along >= len - early {
            if let Some(i) = l.line {
                lines_done = lines_done.max(i + 1);
                line_m += len;
            }
            leg += 1;
            previous = None;
            cross_integral = 0.0;
            continue;
        }
        let path = u[1].datan2(u[0]);
        // Integral action (current set) only once close to the line.
        if cross.abs() < 0.5 * lookahead {
            cross_integral =
                (cross_integral + cross * DT).clamp(-lookahead * 20.0, lookahead * 20.0);
        }
        let desired = path - ((cross + 0.012 * cross_integral) / lookahead).datan2(1.0);
        // Slow down in turns (the helm keeps thrust in reserve for depth
        // keeping and steering).
        let target = if l.line.is_some() {
            setup.speed_mps
        } else {
            0.75 * setup.speed_mps
        };
        let (thrust, steer) = helm.command(&rig, desired, target);
        if !bench.surface
            && let Some(vm) = rig.world.vehicle_mut(0)
        {
            // Terrain following: hold altitude over the seabed a little ahead.
            let ahead = [p[0] + u[0] * length, p[1] + u[1] * length];
            let ground = seabed
                .elevation(p[0], p[1])
                .max(seabed.elevation(ahead[0], ahead[1]));
            vm.model.hold_depth = Some((-ground - setup.platform.altitude_m).max(0.5));
        }
        rig.step(thrust, steer);
        tick += 1;
        if tick.is_multiple_of(60) {
            let v = rig.vehicle();
            track.push([rig.time(), v.position.x, v.position.y, v.position.z]);
            if l.line.is_some() {
                cross_track.push([rig.time(), cross]);
            }
        }
        let logging = l.line.is_some();
        if logging && tick.is_multiple_of(6) {
            let v = rig.vehicle();
            let (roll, pitch, yaw) = trials::attitude(v.orientation);
            let heading_error = wrap(yaw - path).abs();
            xte2 += cross * cross;
            xte_max = xte_max.max(cross.abs());
            roll2 += roll * roll;
            pitch2 += pitch * pitch;
            speed_sum += rig.speed();
            on_line_n += 1;
            let line = l.line.expect("logging");
            let at = v.position + v.orientation.rotate(keel);
            // A surface vessel's transducer sits at its keel, kept wet.
            let at = if bench.surface {
                Vec3::new(at.x, at.y, at.z.min(-0.05))
            } else {
                at
            };
            if heading_error < 35f64.to_radians() {
                let now = swath(&seabed, setup, at, v.orientation);
                if let (Some((pl, a)), Some(b)) = (previous, now)
                    && pl == line
                {
                    coverage.mark(setup, line as u32, &a, &b);
                }
                previous = now.map(|s| (line, s));
            } else {
                previous = None;
            }
        }
    }
    progress(1.0);
    let n = on_line_n.max(1) as f64;
    let mean_speed = speed_sum / n;
    if on_line_n > 0 && mean_speed < 0.9 * setup.speed_mps {
        notes.push(format!(
            "{} reached only {:.1} kn on the lines (planned {:.1} kn); the survey took longer and the soundings are denser.",
            vehicle.name,
            mean_speed / 0.514_444,
            setup.speed_mps / 0.514_444
        ));
    }
    let stats = coverage.stats(setup.order());
    Ok(Run {
        coverage,
        stats,
        track,
        cross_track,
        line_km: line_m / 1000.0,
        hours: rig.time() / 3600.0,
        cross_track_rms_m: (xte2 / n).sqrt(),
        cross_track_max_m: xte_max,
        roll_rms_deg: (roll2 / n).sqrt(),
        pitch_rms_deg: (pitch2 / n).sqrt(),
        lines_done,
        notes,
    })
}

/// Sounding spacing at the swath edge and whether it can resolve the
/// order's feature size (at least about three soundings across it).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Density {
    pub along_m: f64,
    pub across_edge_m: f64,
    pub feature_m: Option<f64>,
    pub resolves_feature: Option<bool>,
}

pub fn density(seabed: &HeightGrid, setup: &Setup) -> Density {
    let along = setup.speed_mps / setup.sonar.ping_hz;
    let depth = -seabed.elevation(setup.area.centre[0], setup.area.centre[1]);
    let h = if setup.platform.underwater {
        setup.platform.altitude_m
    } else {
        (depth - setup.platform.draft_m).max(0.5)
    };
    let across = match setup.sonar.kind {
        SonarKind::Multibeam => {
            let half = (setup.sonar.swath_deg * 0.5).to_radians();
            let limit = setup
                .sonar
                .tvu_angle(h, setup.order().tvu(depth))
                .unwrap_or(0.0)
                .min(half);
            let dtheta = setup.sonar.swath_deg.to_radians() / (setup.sonar.beams - 1) as f64;
            h * dtheta / limit.dcos().powi(2)
        }
        // Side-scan resolution is set by its pulse; report the ping spacing.
        SonarKind::SideScan => along,
    };
    let feature = setup.order().feature(depth);
    Density {
        along_m: along,
        across_edge_m: across,
        feature_m: feature,
        resolves_feature: feature.map(|f| along.max(across) <= f / 2.0),
    }
}

/// A line's start and end as (latitude, longitude).
type GeoLine = ((f64, f64), (f64, f64));

/// A survey plan with its ideal coverage and, optionally, a simulated run.
#[derive(Clone, Debug)]
pub struct Report {
    pub setup: Setup,
    pub lines: Vec<Line>,
    pub ideal: Run,
    pub simulated: Option<Run>,
    pub vehicle: Option<String>,
    pub density: Density,
    /// Scene origin (latitude, longitude) for geographic exports.
    pub origin: Option<(f64, f64)>,
    pub warnings: Vec<String>,
}

/// Plans the lines and their ideal coverage; with a vehicle, also runs them
/// with its physics.
pub fn run(
    scene: &Scene,
    setup: &Setup,
    vehicle: Option<&Entity>,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<Report, String> {
    let bed = seabed(scene);
    let lines = plan(&bed, setup)?;
    let ideal = ideal(&bed, setup, &lines);
    let mut warnings = vec![];
    let bounds = bed.bounds();
    for c in setup.area.corners() {
        if c[0] < bounds[0] || c[0] > bounds[2] || c[1] < bounds[1] || c[1] > bounds[3] {
            warnings.push(
                "Part of the area is outside the scene's seabed, where the depth is held at its edge."
                    .into(),
            );
            break;
        }
    }
    if setup.order().coverage_percent < 100.0 {
        warnings.push(format!(
            "{} needs only {:.0} % bathymetric coverage; the lines are planned for full coverage.",
            setup.order().name,
            setup.order().coverage_percent
        ));
    }
    let density = density(&bed, setup);
    if density.resolves_feature == Some(false) {
        warnings.push(format!(
            "Sounding spacing ({:.2} m along, {:.2} m across at the swath edge) is coarse for {:.1} m features: slow down, ping faster or use more beams.",
            density.along_m,
            density.across_edge_m,
            density.feature_m.unwrap_or(0.0)
        ));
    }
    let simulated = match vehicle {
        Some(v) => Some(simulate(scene, v, setup, &lines, cancel, progress)?),
        None => None,
    };
    let p = &scene.provenance;
    Ok(Report {
        setup: setup.clone(),
        lines,
        ideal,
        simulated,
        vehicle: vehicle.map(|v| v.name.clone()),
        density,
        origin: (p.origin_latitude != 0.0 || p.origin_longitude != 0.0)
            .then_some((p.origin_latitude, p.origin_longitude)),
        warnings,
    })
}

impl Report {
    pub fn summary(&self) -> String {
        let s = &self.setup;
        let o = s.order();
        let st = &self.ideal.stats;
        let mut t = format!(
            "Pelagic survey plan\n\nArea: {:.0} × {:.0} m centred at ({:.0}, {:.0}) m, lines on {:.0}°\nDepth {:.1}–{:.1} m\nSensor: {} · {}\nStandard: IHO S-44 Ed. 6.2.0 {} (TVU a {:.2} m, b {:.4}; coverage {:.0} %)\n\nLines: {} · {:.2} line km · about {:.1} h at {:.1} kn (with turns)\n",
            s.area.length_m,
            s.area.width_m,
            s.area.centre[0],
            s.area.centre[1],
            s.area.axis_deg,
            st.min_depth_m,
            st.max_depth_m,
            match s.sonar.kind {
                SonarKind::Multibeam => format!(
                    "multibeam {:.0}° swath, {} beams, {:.0} Hz, range {:.0} m",
                    s.sonar.swath_deg, s.sonar.beams, s.sonar.ping_hz, s.sonar.range_m
                ),
                SonarKind::SideScan => format!(
                    "side-scan {:.0} m per side, blind zone {:.1} × altitude",
                    s.sonar.range_m, s.sonar.blind_zone
                ),
            },
            if s.platform.underwater {
                format!("underwater at {:.1} m altitude", s.platform.altitude_m)
            } else {
                format!(
                    "surface vessel, transducer {:.1} m deep",
                    s.platform.draft_m
                )
            },
            o.name,
            o.tvu_a,
            o.tvu_b,
            o.coverage_percent,
            self.lines.len(),
            self.ideal.line_km,
            self.ideal.hours,
            s.speed_mps / 0.514_444
        );
        let block = |name: &str, r: &Run| {
            format!(
                "\n{name}: covered {:.1} % within TVU ({:.1} % with {} pass{} needed), any data {:.1} %, gaps {} (largest {:.0} m²), mean TVU {}{}{}\n",
                r.stats.covered_percent,
                r.stats.multiple_percent,
                r.stats.required_passes,
                if r.stats.required_passes == 1 {
                    ""
                } else {
                    "es"
                },
                r.stats.raw_percent,
                r.stats.gaps,
                r.stats.largest_gap_m2,
                r.stats
                    .tvu_mean_m
                    .map_or("—".into(), |v| format!("{v:.2} m")),
                if r.stats.meets_order {
                    " — meets the order's coverage"
                } else {
                    " — short of the order's coverage"
                },
                if r.stats.unworkable_percent > 0.05 {
                    format!(
                        " ({:.1} % of the area is land or too shallow and is left out)",
                        r.stats.unworkable_percent
                    )
                } else {
                    String::new()
                }
            )
        };
        t += &block("Planned (ideal)", &self.ideal);
        if let Some(r) = &self.simulated {
            t += &block(
                &format!("Simulated ({})", self.vehicle.clone().unwrap_or_default()),
                r,
            );
            t += &format!(
                "  {:.1} h, {} of {} lines, cross-track RMS {:.2} m (max {:.1} m), roll RMS {:.2}°, pitch RMS {:.2}°\n",
                r.hours,
                r.lines_done,
                self.lines.len(),
                r.cross_track_rms_m,
                r.cross_track_max_m,
                r.roll_rms_deg,
                r.pitch_rms_deg
            );
        }
        t += &format!(
            "\nSounding spacing: {:.2} m along track, {:.2} m across at the swath edge{}\n",
            self.density.along_m,
            self.density.across_edge_m,
            self.density
                .feature_m
                .map_or(String::new(), |f| format!(" (feature size {f:.1} m)"))
        );
        for w in self
            .warnings
            .iter()
            .chain(self.simulated.iter().flat_map(|r| &r.notes))
        {
            t += &format!("Note: {w}\n");
        }
        t
    }

    /// Lines as latitude/longitude pairs (needs a scene origin).
    fn geo_lines(&self) -> Option<Vec<GeoLine>> {
        let (lat, lon) = self.origin?;
        let plane = crate::sar::Plane {
            latitude: lat,
            longitude: lon,
        };
        Some(
            self.lines
                .iter()
                .map(|l| (plane.geo(l.start), plane.geo(l.end)))
                .collect(),
        )
    }

    /// Writes summary.txt, survey.json, lines.csv, coverage and TVU grids
    /// (ESRI ASCII, scene metres) and, with a geographic origin, lines.gpx
    /// and survey.waypoints (QGC WPL 110).
    pub fn write(&self, folder: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let put = |name: &str, text: String| {
            std::fs::write(folder.join(name), text).map_err(|e| format!("{name}: {e}"))
        };
        put("summary.txt", self.summary())?;
        let mut csv =
            String::from("line,start_x,start_y,end_x,end_y,offset_m,reach_left_m,reach_right_m\n");
        for (i, l) in self.lines.iter().enumerate() {
            csv += &format!(
                "{},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2}\n",
                i + 1,
                l.start[0],
                l.start[1],
                l.end[0],
                l.end[1],
                l.offset_m,
                l.reach_m[0],
                l.reach_m[1]
            );
        }
        put("lines.csv", csv)?;
        let run = self.simulated.as_ref().unwrap_or(&self.ideal);
        put(
            "coverage.asc",
            ascii_grid(&run.coverage, |k| f64::from(run.coverage.passes[k])),
        )?;
        if self.setup.sonar.kind == SonarKind::Multibeam {
            put(
                "tvu.asc",
                ascii_grid(&run.coverage, |k| f64::from(run.coverage.tvu[k])),
            )?;
        }
        put(
            "depth.asc",
            ascii_grid(&run.coverage, |k| f64::from(run.coverage.depth[k])),
        )?;
        let json = serde_json::json!({
            "format": FORMAT,
            "version": 1,
            "setup": self.setup,
            "order": self.setup.order().name,
            "lines": self.lines,
            "ideal": { "stats": self.ideal.stats, "line_km": self.ideal.line_km, "hours": self.ideal.hours },
            "simulated": self.simulated.as_ref().map(|r| serde_json::json!({
                "vehicle": self.vehicle,
                "stats": r.stats,
                "hours": r.hours,
                "lines_done": r.lines_done,
                "cross_track_rms_m": r.cross_track_rms_m,
                "cross_track_max_m": r.cross_track_max_m,
                "roll_rms_deg": r.roll_rms_deg,
                "pitch_rms_deg": r.pitch_rms_deg,
                "notes": r.notes,
            })),
            "density": self.density,
            "origin": self.origin,
            "warnings": self.warnings,
        });
        put(
            "survey.json",
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
        )?;
        if let Some(lines) = self.geo_lines() {
            let mut gpx = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gpx version=\"1.1\" creator=\"Pelagic\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n  <rte><name>Survey lines</name>\n",
            );
            let mut wpl = String::from("QGC WPL 110\n");
            let mut k = 0;
            for (i, (a, b)) in lines.iter().enumerate() {
                for (p, end) in [(a, "start"), (b, "end")] {
                    gpx += &format!(
                        "    <rtept lat=\"{:.7}\" lon=\"{:.7}\"><name>L{} {end}</name></rtept>\n",
                        p.0,
                        p.1,
                        i + 1
                    );
                    if k == 0 {
                        wpl += &format!("0\t1\t0\t16\t0\t0\t0\t0\t{:.7}\t{:.7}\t0\t1\n", p.0, p.1);
                        wpl += &format!(
                            "1\t0\t2\t178\t1\t{:.2}\t-1\t0\t0\t0\t0\t1\n",
                            self.setup.speed_mps
                        );
                        k = 2;
                    }
                    wpl += &format!("{k}\t0\t3\t16\t0\t0\t0\t0\t{:.7}\t{:.7}\t0\t1\n", p.0, p.1);
                    k += 1;
                }
            }
            gpx += "  </rte>\n</gpx>\n";
            put("lines.gpx", gpx)?;
            put("survey.waypoints", wpl)?;
        }
        Ok(())
    }
}

/// North-aligned ESRI ASCII grid (scene metres) resampled from a coverage
/// grid; cells outside the area are NODATA.
fn ascii_grid(c: &Coverage, value: impl Fn(usize) -> f64) -> String {
    let corners = c.area.corners();
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in corners {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let cols = ((x1 - x0) / c.cell).ceil().max(1.0) as usize;
    let rows = ((y1 - y0) / c.cell).ceil().max(1.0) as usize;
    let mut s = format!(
        "ncols {cols}\nnrows {rows}\nxllcorner {x0:.3}\nyllcorner {y0:.3}\ncellsize {:.4}\nNODATA_value -9999\n",
        c.cell
    );
    for row in (0..rows).rev() {
        let y = y0 + (row as f64 + 0.5) * c.cell;
        let mut line = Vec::with_capacity(cols);
        for col in 0..cols {
            let x = x0 + (col as f64 + 0.5) * c.cell;
            let g = c.local([x, y]);
            let v = if g[0] >= 0.0
                && g[1] >= 0.0
                && (g[0] as usize) < c.cols
                && (g[1] as usize) < c.rows
            {
                let v = value(g[1] as usize * c.cols + g[0] as usize);
                if v.is_finite() { v } else { -9999.0 }
            } else {
                -9999.0
            };
            line.push(if v == -9999.0 {
                "-9999".to_string()
            } else {
                format!("{v:.3}")
            });
        }
        s += &line.join(" ");
        s.push('\n');
    }
    s
}

/// Compass bearing helper re-exported for callers laying out areas.
pub fn bearing_vector(deg: f64) -> [f64; 2] {
    towards(1.0, deg)
}
