//! Hull and harbour inspection: camera passes over the underwater surfaces
//! of harbour pilings or a ship's hull, and how much of each surface is
//! imaged sharply enough — on the plan and when a hovering vehicle flies it
//! with its own physics in the scene's current.
//!
//! - **Stand-off** from the required ground sample distance (GSD, mm per
//!   pixel: range × pixel angle ÷ cos incidence) and water clarity (image
//!   contrast falls as exp(−c·range) with beam attenuation c; a minimum
//!   contrast sets the longest range), kept outside a clearance.
//! - **Passes** along the longer side of each face at that stand-off, the
//!   spacing set by the part of the frame that still meets the GSD,
//!   contrast and incidence limits, with the chosen overlap. Transits route
//!   around the structure (inflated boxes).
//! - **Coverage**: each face is divided into patches; a camera frame images
//!   a patch when it is inside the frame, faces the camera within the
//!   incidence limit, is not hidden by another part of the structure, and
//!   meets the GSD and contrast limits.
//! - **Physics run**: the vehicle tracks a target point moving along the
//!   path (PID on position, heading held towards the surface) in the
//!   scene's sea and current; the camera is gimbal-stabilised on the
//!   planned view direction and frames are taken from the real pose.
use crate::{
    entities::Entity,
    math::{Det, Quat, Vec3},
    ocean::Bathymetry,
    scene::Scene,
    trials::{self, Bench},
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

pub const FORMAT: &str = "pelagic-inspection";
const DT: f64 = 1.0 / 60.0;

/// A box about a vertical axis: centre, half sizes, yaw (radians from +X).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Obb {
    pub centre: Vec3,
    pub half: Vec3,
    pub yaw: f64,
}
impl Obb {
    fn into_local(self, p: Vec3) -> Vec3 {
        let d = p - self.centre;
        let (s, c) = (-self.yaw).dsin_cos();
        Vec3::new(d.x * c - d.y * s, d.x * s + d.y * c, d.z)
    }
    fn into_world(self, l: Vec3) -> Vec3 {
        let (s, c) = self.yaw.dsin_cos();
        self.centre + Vec3::new(l.x * c - l.y * s, l.x * s + l.y * c, l.z)
    }
    fn axis(&self, k: usize) -> Vec3 {
        let (s, c) = self.yaw.dsin_cos();
        match k {
            0 => Vec3::new(c, s, 0.0),
            1 => Vec3::new(-s, c, 0.0),
            _ => Vec3::new(0.0, 0.0, 1.0),
        }
    }
    /// Distance from a point to the box surface (negative inside).
    pub fn distance(&self, p: Vec3) -> f64 {
        let l = self.into_local(p);
        let q = Vec3::new(
            l.x.abs() - self.half.x,
            l.y.abs() - self.half.y,
            l.z.abs() - self.half.z,
        );
        let outside = Vec3::new(q.x.max(0.0), q.y.max(0.0), q.z.max(0.0)).length();
        outside + q.x.max(q.y).max(q.z).min(0.0)
    }
    /// Whether segment a–b passes within `margin` of the box (slab test on
    /// the inflated box).
    fn hits(&self, a: Vec3, b: Vec3, margin: f64) -> bool {
        let (la, lb) = (self.into_local(a), self.into_local(b));
        let d = lb - la;
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let h = [
            self.half.x + margin,
            self.half.y + margin,
            self.half.z + margin,
        ];
        let (o, dv) = ([la.x, la.y, la.z], [d.x, d.y, d.z]);
        for k in 0..3 {
            if dv[k].abs() < 1e-12 {
                if o[k] < -h[k] || o[k] > h[k] {
                    return false;
                }
                continue;
            }
            let (mut e0, mut e1) = ((-h[k] - o[k]) / dv[k], (h[k] - o[k]) / dv[k]);
            if e0 > e1 {
                std::mem::swap(&mut e0, &mut e1);
            }
            t0 = t0.max(e0);
            t1 = t1.min(e1);
            if t0 > t1 {
                return false;
            }
        }
        true
    }
}

/// A rectangular surface to inspect: centre, outward normal, in-plane axes
/// u and v (unit) and half sizes along them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub name: String,
    pub centre: Vec3,
    pub normal: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub half: [f64; 2],
    /// Index of the box this face belongs to.
    pub body: usize,
}
impl Face {
    pub fn area(&self) -> f64 {
        4.0 * self.half[0] * self.half[1]
    }
    fn at(&self, a: f64, b: f64) -> Vec3 {
        self.centre + self.u * a + self.v * b
    }
}

/// What to inspect.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub name: String,
    pub boxes: Vec<Obb>,
    pub faces: Vec<Face>,
    /// The seabed: the path keeps a clearance above it.
    pub seabed: Option<crate::terrain::HeightGrid>,
}
impl Target {
    /// `p` raised, if needed, to `clearance` above the seabed.
    fn above_seabed(&self, p: Vec3, clearance: f64) -> Vec3 {
        match &self.seabed {
            Some(bed) => Vec3::new(p.x, p.y, p.z.max(bed.elevation(p.x, p.y) + clearance)),
            None => p,
        }
    }
}

/// Underwater faces of a vertical-sided box between `low` and `high`
/// (z), sides only or with the bottom.
fn box_faces(b: &Obb, body: usize, prefix: &str, low: f64, high: f64, bottom: bool) -> Vec<Face> {
    let z0 = (b.centre.z - b.half.z).max(low);
    let z1 = (b.centre.z + b.half.z).min(high);
    let mut out = vec![];
    if z1 - z0 > 0.1 {
        let zc = 0.5 * (z0 + z1);
        let hz = 0.5 * (z1 - z0);
        let up = Vec3::new(0.0, 0.0, 1.0);
        for (k, sign, name) in [
            (0, 1.0, "bow / +x"),
            (0, -1.0, "stern / −x"),
            (1, 1.0, "port / +y"),
            (1, -1.0, "starboard / −y"),
        ] {
            let n = b.axis(k) * sign;
            let along = b.axis(1 - k) * sign;
            let half_w = if k == 0 { b.half.y } else { b.half.x };
            let offset = if k == 0 { b.half.x } else { b.half.y };
            let c = b.centre + n * offset;
            out.push(Face {
                name: format!("{prefix} {name}"),
                centre: Vec3::new(c.x, c.y, zc),
                normal: n,
                u: along,
                v: up,
                half: [half_w, hz],
                body,
            });
        }
    }
    let bottom_z = b.centre.z - b.half.z;
    if bottom && bottom_z < high && bottom_z > low {
        out.push(Face {
            name: format!("{prefix} bottom"),
            centre: Vec3::new(b.centre.x, b.centre.y, bottom_z),
            normal: Vec3::new(0.0, 0.0, -1.0),
            u: b.axis(0),
            v: b.axis(1) * -1.0,
            half: [b.half.x, b.half.y],
            body,
        });
    }
    out
}

/// The harbour structure's pilings (deck kept as an obstacle).
pub fn pilings(scene: &Scene) -> Result<Target, String> {
    if !scene.structure {
        return Err("This scene has no harbour structure.".into());
    }
    let bed = crate::survey::seabed(scene);
    let world = crate::scene::world(&Scene {
        entities: vec![],
        vehicle: false,
        ..scene.clone()
    })?;
    let mut boxes = vec![];
    let mut faces = vec![];
    let mut count = 0;
    for s in &world.ocean.structures {
        let b = Obb {
            centre: s.center,
            half: s.half,
            yaw: 0.0,
        };
        let index = boxes.len();
        boxes.push(b);
        if s.half.z > 3.0 * s.half.x.max(s.half.y) {
            count += 1;
            let floor = bed.elevation(s.center.x, s.center.y) + 0.3;
            faces.extend(box_faces(
                &b,
                index,
                &format!("Piling {count}"),
                floor,
                -0.3,
                false,
            ));
        }
    }
    if faces.is_empty() {
        return Err("The structure has no pilings below the water.".into());
    }
    Ok(Target {
        name: "Harbour pilings".into(),
        boxes,
        faces,
        seabed: Some(bed),
    })
}

/// A vessel's hull below the waterline (bottom and sides), from its box;
/// a floating hull sits at its hydrostatic draft.
pub fn hull(entity: &Entity, density: f64) -> Result<Target, String> {
    let h = entity.model.half;
    let floats = entity.mass < 0.98 * density * entity.volume;
    let zc = if floats {
        h.z - 2.0 * h.z * entity.mass / (density * entity.volume)
    } else {
        entity.position.z
    };
    let b = Obb {
        centre: Vec3::new(entity.position.x, entity.position.y, zc),
        half: h,
        yaw: entity.heading.to_radians(),
    };
    let faces = box_faces(&b, 0, &entity.name, -11_000.0, -0.1, true);
    if faces.is_empty() {
        return Err(format!(
            "{} has nothing below the waterline to inspect.",
            entity.name
        ));
    }
    Ok(Target {
        name: format!("{} hull", entity.name),
        boxes: vec![b],
        faces,
        seabed: None,
    })
}

/// The camera.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    /// Horizontal field of view (°) and image size (pixels).
    pub fov_deg: f64,
    pub pixels: [f64; 2],
    pub frame_hz: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            fov_deg: 70.0,
            pixels: [1920.0, 1080.0],
            frame_hz: 2.0,
        }
    }
}
impl Camera {
    /// Angle per pixel (radians).
    pub fn ifov(&self) -> f64 {
        self.fov_deg.to_radians() / self.pixels[0]
    }
    /// Half fields of view (radians), horizontal and vertical.
    pub fn half_fov(&self) -> [f64; 2] {
        let h = (self.fov_deg.to_radians() * 0.5).dtan();
        [
            h.datan2(1.0),
            (h * self.pixels[1] / self.pixels[0]).datan2(1.0),
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    pub camera: Camera,
    /// Required ground sample distance (mm per pixel).
    pub gsd_mm: f64,
    /// Frame overlap between passes (fraction).
    pub overlap: f64,
    /// Lowest acceptable image contrast, exp(−c·range).
    pub min_contrast: f64,
    /// Beam attenuation (1/m).
    pub attenuation: f64,
    pub max_incidence_deg: f64,
    /// Survey speed along passes (m/s).
    pub speed_mps: f64,
    /// Stand-off (m); 0 chooses it.
    pub standoff_m: f64,
    /// Least distance from the vehicle centre to the structure (m).
    pub clearance_m: f64,
}
impl Default for Setup {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            gsd_mm: 1.0,
            overlap: 0.3,
            min_contrast: 0.25,
            attenuation: 0.5,
            max_incidence_deg: 60.0,
            speed_mps: 0.25,
            standoff_m: 0.0,
            clearance_m: 0.8,
        }
    }
}
impl Setup {
    pub fn validate(&self) -> Result<(), String> {
        let checks = [
            (self.camera.fov_deg, 5.0, 170.0, "Field of view (°)"),
            (self.camera.pixels[0], 64.0, 20_000.0, "Image width (px)"),
            (self.camera.pixels[1], 64.0, 20_000.0, "Image height (px)"),
            (self.camera.frame_hz, 0.1, 60.0, "Frame rate (Hz)"),
            (self.gsd_mm, 0.05, 100.0, "Required GSD (mm/px)"),
            (self.overlap, 0.0, 0.9, "Overlap"),
            (self.min_contrast, 0.01, 0.95, "Minimum contrast"),
            (self.attenuation, 0.001, 20.0, "Attenuation (1/m)"),
            (self.max_incidence_deg, 10.0, 85.0, "Incidence limit (°)"),
            (self.speed_mps, 0.02, 3.0, "Speed (m/s)"),
            (self.standoff_m, 0.0, 50.0, "Stand-off (m)"),
            (self.clearance_m, 0.1, 20.0, "Clearance (m)"),
        ];
        for (v, lo, hi, name) in checks {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{name} must be between {lo} and {hi}."));
            }
        }
        Ok(())
    }
    /// Longest range meeting the contrast limit (m).
    pub fn visibility_m(&self) -> f64 {
        (1.0 / self.min_contrast).dln() / self.attenuation
    }
    /// Range at which the on-axis GSD equals the requirement (m).
    pub fn gsd_range_m(&self) -> f64 {
        self.gsd_mm * 1e-3 / self.camera.ifov()
    }
    /// Stand-off used and notes on what limits it.
    pub fn standoff(&self) -> (f64, Vec<String>) {
        let mut notes = vec![];
        if self.standoff_m > 0.0 {
            return (self.standoff_m.max(self.clearance_m), notes);
        }
        let ideal = 0.85 * self.gsd_range_m().min(self.visibility_m());
        if ideal < self.clearance_m {
            notes.push(format!(
                "The image limits need a stand-off of {ideal:.2} m, inside the {:.2} m clearance: images will not meet them everywhere (sharper camera, clearer water or a lower requirement).",
                self.clearance_m
            ));
        }
        (ideal.max(self.clearance_m), notes)
    }
    /// Usable half-angles (radians) of the frame at stand-off `d`: within
    /// the field of view, GSD, contrast and incidence limits.
    fn usable(&self, d: f64) -> [f64; 2] {
        let ratio = (d / self.gsd_range_m()).clamp(0.0, 1.0);
        let gsd = ratio.sqrt().dacos();
        let vis = (d / self.visibility_m()).clamp(0.0, 1.0).dacos();
        let inc = self.max_incidence_deg.to_radians();
        let fov = self.camera.half_fov();
        let lim = gsd.min(vis).min(inc).max(0.05);
        [fov[0].min(lim), fov[1].min(lim)]
    }
}

/// A point on the inspection path.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Waypoint {
    pub position: Vec3,
    /// Vehicle heading (radians from +X), camera look and up directions.
    pub yaw: f64,
    pub look: Vec3,
    pub up: Vec3,
    /// Frames are taken on the leg ending here.
    pub capture: bool,
}

/// Plans passes over every face and routes between them.
pub fn plan(target: &Target, setup: &Setup) -> Result<(Vec<Waypoint>, f64, Vec<String>), String> {
    setup.validate()?;
    let (d, notes) = setup.standoff();
    let usable = setup.usable(d);
    let foot = [
        2.0 * d * usable[0].dtan() * (1.0 - setup.overlap),
        2.0 * d * usable[1].dtan() * (1.0 - setup.overlap),
    ];
    let mut path: Vec<Waypoint> = vec![];
    let mut remaining: Vec<usize> = (0..target.faces.len()).collect();
    let mut at: Option<Vec3> = None;
    while !remaining.is_empty() {
        // Nearest face next.
        let pick = remaining
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let da = at.map_or(0.0, |p| (target.faces[*a.1].centre - p).length());
                let db = at.map_or(0.0, |p| (target.faces[*b.1].centre - p).length());
                da.total_cmp(&db)
            })
            .map(|x| x.0)
            .expect("non-empty");
        let f = &target.faces[remaining.remove(pick)];
        // Passes along the longer side; the frame's wide axis across them.
        let along_u = f.half[0] >= f.half[1];
        let (len_a, len_b) = if along_u {
            (f.half[0], f.half[1])
        } else {
            (f.half[1], f.half[0])
        };
        let step = foot[0].max(0.05);
        let rows = ((2.0 * len_b / step).ceil() as usize).max(1);
        let first = -len_b + len_b * 2.0 / rows as f64 * 0.5;
        let spacing = 2.0 * len_b / rows as f64;
        let ends = (len_a - foot[1] * 0.25).max(0.0);
        let vertical = f.normal.z.abs() < 0.5;
        let yaw = if vertical {
            (-f.normal.y).datan2(-f.normal.x)
        } else {
            f.u.y.datan2(f.u.x)
        };
        let look = f.normal * -1.0;
        // Camera up along the passes, so the frame's wide axis spans them.
        let up = if along_u { f.u } else { f.v };
        for r in 0..rows {
            let b = first + r as f64 * spacing;
            let sign = if r % 2 == 0 { -1.0 } else { 1.0 };
            let (s0, s1) = (sign * ends, -sign * ends);
            let point = |a: f64| {
                let (x, y) = if along_u { (a, b) } else { (b, a) };
                target.above_seabed(f.at(x, y) + f.normal * d, setup.clearance_m)
            };
            let start = Waypoint {
                position: point(s0),
                yaw,
                look,
                up,
                capture: false,
            };
            if let Some(last) = path.last().copied() {
                for p in route(target, last.position, start.position, setup.clearance_m) {
                    path.push(Waypoint {
                        position: p,
                        capture: false,
                        ..start
                    });
                }
            }
            path.push(start);
            path.push(Waypoint {
                position: point(s1),
                capture: true,
                ..start
            });
        }
        at = path.last().map(|w| w.position);
    }
    Ok((path, d, notes))
}

/// Intermediate points from a to b around the structure.
fn route(target: &Target, a: Vec3, b: Vec3, clearance: f64) -> Vec<Vec3> {
    let margin = clearance * 0.9;
    let clear = |p: Vec3, q: Vec3| target.boxes.iter().all(|o| !o.hits(p, q, margin));
    if clear(a, b) {
        return vec![];
    }
    // Corners of the inflated boxes, kept under the surface.
    let mut corners = vec![];
    for o in &target.boxes {
        let m = clearance * 1.6;
        for sx in [-1.0, 1.0] {
            for sy in [-1.0, 1.0] {
                for z in [a.z, b.z, 0.5 * (a.z + b.z)] {
                    let l = o.into_local(Vec3::new(0.0, 0.0, z));
                    let c = o.into_world(Vec3::new(sx * (o.half.x + m), sy * (o.half.y + m), l.z));
                    let c = target.above_seabed(c, clearance);
                    if c.z < -0.3 && target.boxes.iter().all(|x| x.distance(c) > margin) {
                        corners.push(c);
                    }
                }
            }
        }
    }
    let mut best: Option<(f64, Vec<Vec3>)> = None;
    for &c in &corners {
        if clear(a, c) && clear(c, b) {
            let cost = (c - a).length() + (b - c).length();
            if best.as_ref().is_none_or(|x| cost < x.0) {
                best = Some((cost, vec![c]));
            }
        }
    }
    if best.is_none() {
        for &c in &corners {
            if !clear(a, c) {
                continue;
            }
            for &e in &corners {
                if clear(c, e) && clear(e, b) {
                    let cost = (c - a).length() + (e - c).length() + (b - e).length();
                    if best.as_ref().is_none_or(|x| cost < x.0) {
                        best = Some((cost, vec![c, e]));
                    }
                }
            }
        }
    }
    best.map(|x| x.1).unwrap_or_default()
}

/// Coverage of each face's patches.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceCoverage {
    pub cols: usize,
    pub rows: usize,
    pub patch_m: [f64; 2],
    /// 0 unseen, 1 seen below the requirements, 2 meets them.
    pub state: Vec<u8>,
    /// Best GSD reached per patch (mm/px), NaN when unseen.
    pub gsd: Vec<f32>,
}

struct Patches {
    faces: Vec<FaceCoverage>,
    centres: Vec<Vec<Vec3>>,
}

impl Patches {
    fn new(target: &Target, patch: f64) -> Self {
        let mut faces = vec![];
        let mut centres = vec![];
        for f in &target.faces {
            let cols = ((2.0 * f.half[0] / patch).ceil() as usize).clamp(1, 400);
            let rows = ((2.0 * f.half[1] / patch).ceil() as usize).clamp(1, 400);
            let pu = 2.0 * f.half[0] / cols as f64;
            let pv = 2.0 * f.half[1] / rows as f64;
            let mut c = Vec::with_capacity(cols * rows);
            for j in 0..rows {
                for i in 0..cols {
                    c.push(f.at(
                        -f.half[0] + (i as f64 + 0.5) * pu,
                        -f.half[1] + (j as f64 + 0.5) * pv,
                    ));
                }
            }
            faces.push(FaceCoverage {
                cols,
                rows,
                patch_m: [pu, pv],
                state: vec![0; cols * rows],
                gsd: vec![f32::NAN; cols * rows],
            });
            centres.push(c);
        }
        Self { faces, centres }
    }

    /// Records one frame from `eye` looking along `look` with `up`.
    fn frame(&mut self, target: &Target, setup: &Setup, eye: Vec3, look: Vec3, up: Vec3) {
        let look = look.normalized();
        let right = look.cross(up).normalized();
        let up = right.cross(look);
        let half = setup.camera.half_fov();
        let (tx, ty) = (half[0].dtan(), half[1].dtan());
        let ifov = setup.camera.ifov();
        let vis = setup.visibility_m();
        let cos_inc = setup.max_incidence_deg.to_radians().dcos();
        for (k, f) in target.faces.iter().enumerate() {
            if f.normal.dot(eye - f.centre) <= 0.0 {
                continue;
            }
            for (p, q) in self.centres[k].iter().enumerate() {
                let dvec = *q - eye;
                let z = dvec.dot(look);
                if z <= 0.0 {
                    continue;
                }
                if (dvec.dot(right) / z).abs() > tx || (dvec.dot(up) / z).abs() > ty {
                    continue;
                }
                let range = dvec.length();
                if range > vis * 1.5 {
                    continue;
                }
                let c = -dvec.dot(f.normal) / range;
                if c <= 0.05 {
                    continue;
                }
                // Hidden by another part of the structure?
                let hidden = target
                    .boxes
                    .iter()
                    .enumerate()
                    .any(|(b, o)| b != f.body && o.hits(eye, *q - dvec * (0.02 / range), 0.0));
                if hidden {
                    continue;
                }
                let gsd = (range * ifov / c * 1000.0) as f32;
                let fc = &mut self.faces[k];
                let ok = gsd as f64 <= setup.gsd_mm * 1.0001 && range <= vis && c >= cos_inc - 1e-9;
                if fc.gsd[p].is_nan() || gsd < fc.gsd[p] {
                    fc.gsd[p] = gsd;
                }
                fc.state[p] = fc.state[p].max(if ok { 2 } else { 1 });
            }
        }
    }
}

/// Coverage of one face.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FaceStats {
    pub name: String,
    pub area_m2: f64,
    /// Share imaged to the requirements, and imaged at all (%).
    pub covered_percent: f64,
    pub seen_percent: f64,
    pub best_gsd_mm: Option<f64>,
}

/// An inspection run (ideal or with vehicle physics).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub coverage: Vec<FaceCoverage>,
    pub faces: Vec<FaceStats>,
    /// Area-weighted share of the surfaces imaged to the requirements (%).
    pub covered_percent: f64,
    pub seen_percent: f64,
    pub minutes: f64,
    pub path_m: f64,
    pub frames: usize,
    /// Physics runs: position error RMS on passes, least clearance to the
    /// structure, and the track (t, x, y, z) every second.
    pub position_rms_m: f64,
    pub min_clearance_m: f64,
    pub track: Vec<[f64; 4]>,
    pub notes: Vec<String>,
}

fn finish(target: &Target, patches: Patches) -> (Vec<FaceStats>, f64, f64, Vec<FaceCoverage>) {
    let mut stats = vec![];
    let (mut area, mut ok, mut seen) = (0.0, 0.0, 0.0);
    for (f, c) in target.faces.iter().zip(&patches.faces) {
        let n = c.state.len().max(1) as f64;
        let good = c.state.iter().filter(|s| **s == 2).count() as f64 / n;
        let any = c.state.iter().filter(|s| **s >= 1).count() as f64 / n;
        let best = c
            .gsd
            .iter()
            .filter(|g| g.is_finite())
            .fold(f64::MAX, |m, g| m.min(f64::from(*g)));
        area += f.area();
        ok += f.area() * good;
        seen += f.area() * any;
        stats.push(FaceStats {
            name: f.name.clone(),
            area_m2: f.area(),
            covered_percent: good * 100.0,
            seen_percent: any * 100.0,
            best_gsd_mm: (best < f64::MAX).then_some(best),
        });
    }
    let area = area.max(1e-9);
    (stats, ok / area * 100.0, seen / area * 100.0, patches.faces)
}

fn patch_size(target: &Target, setup: &Setup, d: f64) -> f64 {
    let total: f64 = target.faces.iter().map(|f| f.area()).sum();
    let usable = setup.usable(d);
    let foot = 2.0 * d * usable[0].dtan().min(usable[1].dtan());
    (foot / 5.0).max((total / 60_000.0).sqrt()).clamp(0.03, 1.0)
}

/// Frames taken along the planned path at survey speed (no vehicle).
pub fn ideal(target: &Target, setup: &Setup, path: &[Waypoint], standoff: f64) -> Run {
    let mut patches = Patches::new(target, patch_size(target, setup, standoff));
    let spacing = setup.speed_mps / setup.camera.frame_hz;
    let (mut length, mut frames) = (0.0, 0usize);
    for w in path.windows(2) {
        let seg = w[1].position - w[0].position;
        let len = seg.length();
        length += len;
        if !w[1].capture {
            continue;
        }
        let n = (len / spacing).ceil().max(1.0) as usize;
        for k in 0..=n {
            let p = w[0].position + seg * (k as f64 / n as f64);
            patches.frame(target, setup, p, w[1].look, w[1].up);
            frames += 1;
        }
    }
    let (faces, covered, seen, coverage) = finish(target, patches);
    Run {
        coverage,
        faces,
        covered_percent: covered,
        seen_percent: seen,
        minutes: length / setup.speed_mps / 60.0,
        path_m: length,
        frames,
        position_rms_m: 0.0,
        min_clearance_m: path
            .iter()
            .map(|w| {
                target
                    .boxes
                    .iter()
                    .map(|b| b.distance(w.position))
                    .fold(f64::MAX, f64::min)
            })
            .fold(f64::MAX, f64::min),
        track: vec![],
        notes: vec![],
    }
}

/// Flies the path with `vehicle`'s physics in the scene's sea and current.
#[allow(clippy::too_many_arguments)]
pub fn simulate(
    scene: &Scene,
    vehicle: &Entity,
    target: &Target,
    setup: &Setup,
    path: &[Waypoint],
    standoff: f64,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<Run, String> {
    if path.len() < 2 {
        return Err("Plan the inspection first.".into());
    }
    let env = trials::Environment::from_scene(scene);
    let start = path[0].position;
    let bench = Bench::new(
        vehicle,
        &env,
        &trials::Setup {
            operating_depth: (-start.z).clamp(1.0, env.water_depth - 2.0),
            ..trials::Setup::quick()
        },
    )?;
    let mut notes = vec![];
    if bench.surface {
        return Err(format!(
            "{} floats: inspection needs a vehicle that can hover underwater (an ROV).",
            vehicle.name
        ));
    }
    let mut entity = bench.entity.clone();
    // Position is controlled directly: no depth or heading holds.
    entity.model.hold_depth = None;
    entity.model.hold_heading = None;
    let mut s = scene.clone();
    s.vehicle = false;
    s.entities.clear();
    s.school.count = 0;
    s.particles = 0;
    let mut world = crate::scene::world(&s)?;
    let mut v = entity.spawn();
    v.position = start;
    v.orientation = Quat::from_rotation_vector(Vec3::new(0.0, 0.0, path[0].yaw));
    world.add_vehicle(v);
    let mass = entity.mass + (entity.model.added_mass.x + entity.model.added_mass.y) * 0.5;
    let inertia = entity.effective_inertia().z + entity.model.added_inertia.z.max(0.0);
    let (w, z) = (0.6, 1.0);
    let (kp, kd, ki) = (mass * w * w, 2.0 * z * w * mass, mass * 0.04);
    let limit = entity.model.thrust_limit.max(1.0);
    let radius = entity.model.half.x.max(entity.model.half.y);
    let half = entity.model.half;
    // Cumulative path length.
    let mut cum = vec![0.0];
    for w in path.windows(2) {
        cum.push(cum.last().expect("start") + (w[1].position - w[0].position).length());
    }
    let total = *cum.last().expect("path");
    let mut patches = Patches::new(target, patch_size(target, setup, standoff));
    let (mut s_along, mut integral) = (0.0f64, Vec3::ZERO);
    let (mut err2, mut err_n, mut frames, mut min_clear) = (0.0, 0usize, 0usize, f64::MAX);
    let mut track = vec![];
    let frame_every = (60.0 / setup.camera.frame_hz).round().max(1.0) as u64;
    let limit_s = total / setup.speed_mps * 3.0 + 600.0;
    let mut tick = 0u64;
    let mut seg = 0usize;
    let (mut stalled, mut skipped) = (0.0f64, false);
    while s_along < total || seg + 1 < path.len() {
        if world.time() > limit_s {
            notes.push(format!(
                "Stopped after {:.0} min at {:.0} % of the path: the vehicle could not keep up.",
                world.time() / 60.0,
                s_along / total * 100.0
            ));
            break;
        }
        if tick.is_multiple_of(600) {
            if cancel.load(Ordering::Relaxed) {
                return Err("Cancelled.".into());
            }
            progress(s_along / total.max(1e-9));
        }
        let vref = &world.vehicles()[0];
        let p = vref.position;
        while seg + 1 < path.len() - 1 && s_along >= cum[seg + 1] {
            seg += 1;
        }
        let (a, b) = (path[seg], path[(seg + 1).min(path.len() - 1)]);
        let seg_len = (cum[(seg + 1).min(cum.len() - 1)] - cum[seg]).max(1e-9);
        let f = ((s_along - cum[seg]) / seg_len).clamp(0.0, 1.0);
        let target_p = a.position + (b.position - a.position) * f;
        let dir = (b.position - a.position) * (1.0 / seg_len);
        let error = target_p - p;
        // The target point waits for the vehicle, but not for ever: after
        // 30 s without getting close it moves on (the section is skipped).
        let close = error.length() < 0.4 * standoff.max(0.5);
        stalled = if close { 0.0 } else { stalled + DT };
        if stalled > 30.0 && !skipped {
            skipped = true;
            notes.push(format!(
                "The vehicle could not reach part of the path near ({:.1}, {:.1}, {:.1}) m; it was skipped.",
                target_p.x, target_p.y, target_p.z
            ));
        }
        let pace = if close || stalled > 30.0 {
            setup.speed_mps
        } else {
            0.0
        };
        s_along = (s_along + pace * DT).min(total);
        if s_along >= total && error.length() < 0.2 {
            break;
        }
        let vel = vref.velocity;
        // Integral action (current, trim) only near the target point, so
        // long transits do not wind it up.
        if error.length() < 0.3 {
            integral = integral + error * DT;
        }
        let li = limit / ki.max(1e-9) * 0.25;
        integral = Vec3::new(
            integral.x.clamp(-li, li),
            integral.y.clamp(-li, li),
            integral.z.clamp(-li, li),
        );
        let mut force = error * kp + (dir * pace - vel) * kd + integral * ki;
        let len = force.length();
        if len > 0.9 * limit {
            force = force * (0.9 * limit / len);
        }
        let body = vref.orientation.conjugate().rotate(force);
        let (_, _, yaw) = trials::attitude(vref.orientation);
        let dyaw = (b.yaw - yaw + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        let torque = inertia * (0.8 * dyaw) - 2.0 * inertia * 0.9 * vref.angular_velocity.z;
        if let Some(v) = world.vehicle_mut(0) {
            v.force = body;
            v.torque = Vec3::new(0.0, 0.0, torque);
        }
        world.step();
        tick += 1;
        let v = &world.vehicles()[0];
        let clear = target
            .boxes
            .iter()
            .map(|o| o.distance(v.position))
            .fold(f64::MAX, f64::min)
            - radius;
        min_clear = min_clear.min(clear);
        if tick.is_multiple_of(60) {
            track.push([world.time(), v.position.x, v.position.y, v.position.z]);
        }
        if b.capture && tick.is_multiple_of(frame_every) {
            let eye = v.position + v.orientation.rotate(Vec3::new(half.x, 0.0, 0.0));
            patches.frame(target, setup, eye, b.look, b.up);
            frames += 1;
            err2 += error.dot(error);
            err_n += 1;
        }
    }
    progress(1.0);
    if min_clear < 0.0 {
        notes.push("The vehicle touched the structure.".into());
    }
    let (faces, covered, seen, coverage) = finish(target, patches);
    Ok(Run {
        coverage,
        faces,
        covered_percent: covered,
        seen_percent: seen,
        minutes: world.time() / 60.0,
        path_m: total,
        frames,
        position_rms_m: (err2 / err_n.max(1) as f64).sqrt(),
        min_clearance_m: min_clear,
        track,
        notes,
    })
}

/// A planned inspection, its ideal coverage and optionally a physics run.
#[derive(Clone, Debug)]
pub struct Report {
    pub target: Target,
    pub setup: Setup,
    pub path: Vec<Waypoint>,
    pub standoff_m: f64,
    pub ideal: Run,
    pub simulated: Option<Run>,
    pub vehicle: Option<String>,
    pub notes: Vec<String>,
}

pub fn run(
    scene: &Scene,
    target: Target,
    setup: &Setup,
    vehicle: Option<&Entity>,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<Report, String> {
    let mut target = target;
    if target.seabed.is_none() {
        target.seabed = Some(crate::survey::seabed(scene));
    }
    // Keep the vehicle's own size clear of the structure.
    let mut setup = setup.clone();
    if let Some(v) = vehicle {
        let radius = v.model.half.x.max(v.model.half.y);
        setup.clearance_m = setup.clearance_m.max(radius + 0.4);
    }
    let setup = &setup;
    let (path, standoff, notes) = plan(&target, setup)?;
    let ideal = ideal(&target, setup, &path, standoff);
    let simulated = match vehicle {
        Some(v) => Some(simulate(
            scene, v, &target, setup, &path, standoff, cancel, progress,
        )?),
        None => None,
    };
    Ok(Report {
        target,
        setup: setup.clone(),
        path,
        standoff_m: standoff,
        ideal,
        simulated,
        vehicle: vehicle.map(|v| v.name.clone()),
        notes,
    })
}

impl Report {
    pub fn summary(&self) -> String {
        let s = &self.setup;
        let mut t = format!(
            "Pelagic inspection plan\n\nTarget: {} ({} faces, {:.1} m² below the waterline)\nCamera: {:.0}° × {:.0}×{:.0} px, required GSD {:.2} mm/px; water attenuation {:.2} /m (contrast ≥ {:.2} to {:.1} m)\nStand-off {:.2} m (GSD range {:.2} m), overlap {:.0} %, {:.2} m/s\n",
            self.target.name,
            self.target.faces.len(),
            self.target.faces.iter().map(|f| f.area()).sum::<f64>(),
            s.camera.fov_deg,
            s.camera.pixels[0],
            s.camera.pixels[1],
            s.gsd_mm,
            s.attenuation,
            s.min_contrast,
            s.visibility_m(),
            self.standoff_m,
            s.gsd_range_m(),
            s.overlap * 100.0,
            s.speed_mps
        );
        let block = |name: &str, r: &Run| {
            let mut x = format!(
                "\n{name}: {:.1} % imaged to the requirements ({:.1} % seen at all), {:.0} m path, {:.1} min, {} frames{}\n",
                r.covered_percent,
                r.seen_percent,
                r.path_m,
                r.minutes,
                r.frames,
                if r.track.is_empty() {
                    String::new()
                } else {
                    format!(
                        ", position error {:.2} m RMS, least clearance {:.2} m",
                        r.position_rms_m, r.min_clearance_m
                    )
                }
            );
            for f in &r.faces {
                x += &format!(
                    "  {:<28} {:>7.1} m²  {:>5.1} %  best {}\n",
                    f.name,
                    f.area_m2,
                    f.covered_percent,
                    f.best_gsd_mm
                        .map_or("—".into(), |g| format!("{g:.2} mm/px"))
                );
            }
            x
        };
        t += &block("Planned", &self.ideal);
        if let Some(r) = &self.simulated {
            t += &block(
                &format!("Flown by {}", self.vehicle.clone().unwrap_or_default()),
                r,
            );
        }
        for n in self
            .notes
            .iter()
            .chain(self.simulated.iter().flat_map(|r| &r.notes))
        {
            t += &format!("Note: {n}\n");
        }
        t
    }

    /// Writes summary.txt, inspection.json, faces.csv, path.csv and (with
    /// a physics run) track.csv.
    pub fn write(&self, folder: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let put = |name: &str, text: String| {
            std::fs::write(folder.join(name), text).map_err(|e| format!("{name}: {e}"))
        };
        put("summary.txt", self.summary())?;
        let run = self.simulated.as_ref().unwrap_or(&self.ideal);
        let mut csv = String::from("face,area_m2,covered_percent,seen_percent,best_gsd_mm\n");
        for f in &run.faces {
            csv += &format!(
                "\"{}\",{:.3},{:.2},{:.2},{}\n",
                f.name,
                f.area_m2,
                f.covered_percent,
                f.seen_percent,
                f.best_gsd_mm.map_or(String::new(), |g| format!("{g:.3}"))
            );
        }
        put("faces.csv", csv)?;
        let mut csv = String::from("x,y,z,yaw_deg,look_x,look_y,look_z,capture\n");
        for w in &self.path {
            csv += &format!(
                "{:.3},{:.3},{:.3},{:.1},{:.3},{:.3},{:.3},{}\n",
                w.position.x,
                w.position.y,
                w.position.z,
                w.yaw.to_degrees(),
                w.look.x,
                w.look.y,
                w.look.z,
                w.capture
            );
        }
        put("path.csv", csv)?;
        if let Some(r) = &self.simulated {
            let mut csv = String::from("t_s,x,y,z\n");
            for p in &r.track {
                csv += &format!("{:.1},{:.3},{:.3},{:.3}\n", p[0], p[1], p[2], p[3]);
            }
            put("track.csv", csv)?;
        }
        let json = serde_json::json!({
            "format": FORMAT,
            "version": 1,
            "target": self.target.name,
            "setup": self.setup,
            "standoff_m": self.standoff_m,
            "planned": { "covered_percent": self.ideal.covered_percent, "seen_percent": self.ideal.seen_percent, "faces": self.ideal.faces, "minutes": self.ideal.minutes, "path_m": self.ideal.path_m },
            "flown": self.simulated.as_ref().map(|r| serde_json::json!({
                "vehicle": self.vehicle,
                "covered_percent": r.covered_percent,
                "seen_percent": r.seen_percent,
                "faces": r.faces,
                "minutes": r.minutes,
                "position_rms_m": r.position_rms_m,
                "min_clearance_m": r.min_clearance_m,
                "notes": r.notes,
            })),
            "notes": self.notes,
        });
        put(
            "inspection.json",
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
        )
    }
}
