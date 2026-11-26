//! The world beyond the scene: real terrain and seabed streamed in rings
//! around the camera (about 41 km at 160 m, 330 km at 1.3 km and 1,300 km
//! at 10 km spacing), with satellite colour on land, so any coast can be
//! flown to. Also live ships (AIS) drawn in 3D and the geographic helpers
//! that tie scene metres to latitude and longitude.
use super::globe::{SENTINEL_CREDIT, merge_elevations, seafloor, sentinel_grid, terrarium_grid};
use pelagic::habitat::Imagery;
use pelagic::math::Vec3;
use pelagic::render_scene::DrawVertex;
use pelagic::terrain::HeightGrid;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

const M_PER_DEG_LAT: f64 = 111_132.0;

/// Scene metres (x east, y north) ↔ latitude and longitude around an
/// origin (equirectangular, as scenes are built).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Geo {
    pub lat: f64,
    pub lon: f64,
}
impl Geo {
    fn per_lon(&self) -> f64 {
        111_320.0 * self.lat.to_radians().cos().max(0.05)
    }
    pub fn to_latlon(self, x: f64, y: f64) -> (f64, f64) {
        let lat = (self.lat + y / M_PER_DEG_LAT).clamp(-89.9, 89.9);
        let lon = (self.lon + x / self.per_lon() + 540.0).rem_euclid(360.0) - 180.0;
        (lat, lon)
    }
    pub fn to_xy(self, lat: f64, lon: f64) -> (f64, f64) {
        let dlon = (lon - self.lon + 540.0).rem_euclid(360.0) - 180.0;
        (dlon * self.per_lon(), (lat - self.lat) * M_PER_DEG_LAT)
    }
}

/// Grid points, spacing (m), terrain-tile zoom, imagery zoom and how far
/// the camera may move before the ring is re-centred (m).
const LEVELS: [(usize, f64, u8, u8, f64); 3] = [
    (257, 160.0, 11, 10, 9_000.0),
    (257, 1_280.0, 8, 7, 70_000.0),
    (129, 10_000.0, 5, 4, 260_000.0),
];

/// A fetched ring: elevations and satellite colour.
type Ring = (HeightGrid, Option<Imagery>);

struct Level {
    centre: Option<[f64; 2]>,
    grid: Option<Arc<HeightGrid>>,
    imagery: Option<Arc<Imagery>>,
    loading: Option<Receiver<Result<Ring, String>>>,
    failed: Option<Instant>,
}

pub(super) struct World {
    pub geo: Geo,
    /// Areas drawn by the scene itself (its terrain and backdrop).
    scene_holes: Vec<[f64; 4]>,
    levels: Vec<Level>,
    mesh: Option<Receiver<Vec<DrawVertex>>>,
    remesh: bool,
    quality: u8,
    pub status: String,
}

fn bounds(g: &HeightGrid) -> [f64; 4] {
    let w = (g.columns - 1) as f64 * g.spacing;
    let h = (g.rows - 1) as f64 * g.spacing;
    [g.origin[0], g.origin[1], g.origin[0] + w, g.origin[1] + h]
}

fn fetch(geo: Geo, centre: [f64; 2], level: usize) -> Result<Ring, String> {
    let (size, spacing, zoom, image_zoom, _) = LEVELS[level];
    let (lat, lon) = geo.to_latlon(centre[0], centre[1]);
    let agent = super::globe::http_agent(60);
    let tiles = crate::project::data_dir().join("tiles");
    let cache = seafloor::cache_dir();
    let land = terrarium_grid(&agent, &tiles, zoom, lat, lon, size, spacing)?;
    let sea = seafloor::dem(&agent, &cache, lat, lon, size, spacing).ok();
    let (elevations, _) = merge_elevations(&land, sea.as_deref());
    let half = (size - 1) as f64 * spacing * 0.5;
    let origin = [centre[0] - half, centre[1] - half];
    let grid = HeightGrid::new(size, size, origin, spacing, elevations).map_err(str::to_string)?;
    let imagery = grid
        .elevations
        .iter()
        .any(|e| *e > 0.5)
        .then(|| sentinel_grid(&agent, &tiles, image_zoom, lat, lon, size, spacing).ok())
        .flatten()
        .and_then(|rgb| Imagery::new(size, size, origin, spacing, rgb, SENTINEL_CREDIT).ok());
    Ok((grid, imagery))
}

impl World {
    /// `scene` is the area the scene draws itself (backdrop, else terrain).
    pub fn new(geo: Geo, scene: Option<[f64; 4]>, quality: u8) -> Self {
        Self {
            geo,
            scene_holes: scene.into_iter().collect(),
            levels: (0..LEVELS.len())
                .map(|_| Level {
                    centre: None,
                    grid: None,
                    imagery: None,
                    loading: None,
                    failed: None,
                })
                .collect(),
            mesh: None,
            remesh: false,
            quality,
            status: String::new(),
        }
    }
    pub fn loading(&self) -> bool {
        self.mesh.is_some() || self.levels.iter().any(|l| l.loading.is_some())
    }
    /// Starts fetches as the camera moves; returns a new terrain mesh when
    /// one is ready.
    pub fn update(&mut self, camera: Vec3) -> Option<Vec<DrawVertex>> {
        let geo = self.geo;
        for (k, level) in self.levels.iter_mut().enumerate() {
            if let Some(rx) = &level.loading
                && let Ok(result) = rx.try_recv()
            {
                level.loading = None;
                match result {
                    Ok((grid, imagery)) => {
                        level.grid = Some(Arc::new(grid));
                        level.imagery = imagery.map(Arc::new);
                        level.failed = None;
                        self.remesh = true;
                    }
                    Err(e) => {
                        level.failed = Some(Instant::now());
                        self.status = format!("Distant terrain unavailable: {e}");
                    }
                }
            }
            let (_, spacing, _, _, recentre) = LEVELS[k];
            // Centres snap to a lattice so revisits hit the disk cache.
            let snap = recentre * 0.5;
            let want = [
                (camera.x / snap).round() * snap,
                (camera.y / snap).round() * snap,
            ];
            let far = level.centre.is_none_or(|c| {
                (c[0] - camera.x).abs() > recentre || (c[1] - camera.y).abs() > recentre
            });
            // High above, the finest ring is pointless.
            let useful = k > 0 || camera.z < spacing * 60.0;
            let retry = level.failed.is_none_or(|t| t.elapsed().as_secs() > 30);
            // Nothing to fetch while the scene itself covers the whole ring.
            let half = (LEVELS[k].0 - 1) as f64 * spacing * 0.5;
            let hidden = self.scene_holes.iter().any(|h| {
                want[0] - half >= h[0]
                    && want[0] + half <= h[2]
                    && want[1] - half >= h[1]
                    && want[1] + half <= h[3]
            });
            if far && hidden {
                level.centre = Some(want);
                level.grid = None;
                level.imagery = None;
                self.remesh = true;
            } else if far && useful && level.loading.is_none() && retry {
                level.centre = Some(want);
                let (send, receive) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = send.send(fetch(geo, want, k));
                });
                level.loading = Some(receive);
            }
        }
        if let Some(rx) = &self.mesh
            && let Ok(mesh) = rx.try_recv()
        {
            self.mesh = None;
            return Some(mesh);
        }
        if self.remesh && self.mesh.is_none() {
            self.remesh = false;
            let layers: Vec<(Arc<HeightGrid>, Option<Arc<Imagery>>)> = self
                .levels
                .iter()
                .filter_map(|l| Some((l.grid.clone()?, l.imagery.clone())))
                .collect();
            let scene = self.scene_holes.clone();
            let quality = self.quality;
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut out = vec![];
                for (k, (grid, imagery)) in layers.iter().enumerate() {
                    // Each ring leaves out what the scene and finer rings draw.
                    let mut holes = scene.clone();
                    holes.extend(layers[..k].iter().map(|(g, _)| bounds(g)));
                    let stride = if quality < 2 && grid.columns > 129 {
                        2
                    } else {
                        1
                    };
                    pelagic::landscape::terrain_patch(
                        grid,
                        imagery.as_deref(),
                        &holes,
                        stride,
                        &mut out,
                    );
                }
                let _ = send.send(out);
            });
            self.mesh = Some(receive);
        }
        None
    }
    /// Terrain or seabed elevation from the finest ring covering a point.
    pub fn elevation(&self, x: f64, y: f64) -> Option<f64> {
        self.levels.iter().find_map(|l| {
            let g = l.grid.as_ref()?;
            let b = bounds(g);
            (x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3])
                .then(|| pelagic::ocean::Bathymetry::elevation(&**g, x, y))
        })
    }
    /// Satellite colour (linear RGB) from the finest ring with imagery.
    pub fn imagery(&self, x: f64, y: f64) -> Option<[f32; 3]> {
        self.levels
            .iter()
            .find_map(|l| l.imagery.as_ref().and_then(|i| i.linear(x, y)))
    }
    /// The nearest land (above 1 m) to a point: (distance m, x, y).
    pub fn nearest_land(&self, x: f64, y: f64) -> Option<(f64, f64, f64)> {
        let mut best: Option<(f64, f64, f64)> = None;
        for l in &self.levels {
            let Some(g) = &l.grid else { continue };
            for j in 0..g.rows {
                for i in 0..g.columns {
                    if g.elevations[j * g.columns + i] > 1.0 {
                        let (px, py) = (
                            g.origin[0] + i as f64 * g.spacing,
                            g.origin[1] + j as f64 * g.spacing,
                        );
                        let d = (px - x).hypot(py - y);
                        if best.is_none_or(|b| d < b.0) {
                            best = Some((d, px, py));
                        }
                    }
                }
            }
            // A finer ring that found land is exact enough.
            if best.is_some_and(|b| {
                let bb = bounds(g);
                b.1 > bb[0] && b.1 < bb[2] && b.2 > bb[1] && b.2 < bb[3]
            }) {
                break;
            }
        }
        best
    }
}

// ------------------------------------------------------------ live ships

/// Ship hull models by AIS category (model space: bow +x 0.5, length 1).
pub(super) const SHIP_KINDS: usize = 7;

fn ship_box(out: &mut Vec<DrawVertex>, c: [f64; 3], h: [f64; 3], colour: [f32; 3]) {
    let p =
        |sx: f64, sy: f64, sz: f64| Vec3::new(c[0] + sx * h[0], c[1] + sy * h[1], c[2] + sz * h[2]);
    let faces: [([f64; 3], [[f64; 3]; 4]); 6] = [
        (
            [1.0, 0.0, 0.0],
            [[1., -1., -1.], [1., 1., -1.], [1., 1., 1.], [1., -1., 1.]],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [-1., 1., -1.],
                [-1., -1., -1.],
                [-1., -1., 1.],
                [-1., 1., 1.],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [[1., 1., -1.], [-1., 1., -1.], [-1., 1., 1.], [1., 1., 1.]],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [-1., -1., -1.],
                [1., -1., -1.],
                [1., -1., 1.],
                [-1., -1., 1.],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [[-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [-1., 1., -1.],
                [1., 1., -1.],
                [1., -1., -1.],
                [-1., -1., -1.],
            ],
        ),
    ];
    for (n, q) in faces {
        let v = q.map(|k| p(k[0], k[1], k[2]));
        for idx in [0, 1, 2, 0, 2, 3] {
            out.push(DrawVertex {
                position: [v[idx].x as f32, v[idx].y as f32, v[idx].z as f32],
                normal: [n[0] as f32, n[1] as f32, n[2] as f32],
                uv: [0.0, 0.0],
                albedo: colour,
                roughness: 0.6,
                metallic: 0.0,
                kind: 3.0,
            });
        }
    }
}

/// A ship of an AIS category: 0 cargo, 1 tanker, 2 passenger, 3 fishing,
/// 4 tug and service, 5 sailing and pleasure, 6 other. Beam about 0.15 of
/// the length; the hull reaches below the waterline.
pub(super) fn ship_mesh(kind: usize) -> Vec<DrawVertex> {
    let mut out = vec![];
    let hull = [0.08, 0.08, 0.09];
    let red = [0.35, 0.05, 0.04];
    let white = [0.85, 0.85, 0.83];
    let (beam, freeboard) = match kind {
        3 | 4 => (0.2, 0.09),
        5 => (0.17, 0.04),
        _ => (0.15, 0.05),
    };
    // Hull in sections narrowing to the bow; red below the waterline.
    let sections = 6;
    for k in 0..sections {
        let x0 = -0.5 + k as f64 / sections as f64;
        let x1 = x0 + 1.0 / sections as f64;
        let taper = |x: f64| {
            if x > 0.25 {
                1.0 - (x - 0.25) / 0.3
            } else {
                1.0
            }
        };
        let w = beam * 0.5 * taper(0.5 * (x0 + x1)).max(0.15);
        ship_box(
            &mut out,
            [0.5 * (x0 + x1), 0.0, freeboard * 0.5],
            [0.5 / sections as f64, w, freeboard * 0.5],
            if kind == 2 || kind == 5 { white } else { hull },
        );
        ship_box(
            &mut out,
            [0.5 * (x0 + x1), 0.0, -0.02],
            [0.5 / sections as f64, w * 0.97, 0.02],
            red,
        );
    }
    let deck = freeboard;
    match kind {
        0 => {
            // Container stacks and an aft bridge.
            let colours = [
                [0.45, 0.08, 0.05],
                [0.05, 0.2, 0.45],
                [0.6, 0.45, 0.05],
                [0.1, 0.35, 0.15],
                [0.5, 0.5, 0.5],
            ];
            for i in 0..7 {
                let x = -0.25 + i as f64 * 0.09;
                ship_box(
                    &mut out,
                    [x, 0.0, deck + 0.025],
                    [0.04, beam * 0.45, 0.025],
                    colours[i % colours.len()],
                );
            }
            ship_box(
                &mut out,
                [-0.4, 0.0, deck + 0.05],
                [0.04, beam * 0.45, 0.05],
                white,
            );
        }
        1 => {
            ship_box(
                &mut out,
                [0.05, 0.0, deck + 0.006],
                [0.38, beam * 0.08, 0.006],
                [0.5, 0.5, 0.45],
            );
            ship_box(
                &mut out,
                [-0.42, 0.0, deck + 0.04],
                [0.04, beam * 0.45, 0.04],
                white,
            );
        }
        2 => {
            ship_box(
                &mut out,
                [-0.05, 0.0, deck + 0.05],
                [0.36, beam * 0.45, 0.05],
                white,
            );
            ship_box(
                &mut out,
                [-0.05, 0.0, deck + 0.11],
                [0.25, beam * 0.4, 0.015],
                [0.12, 0.25, 0.45],
            );
        }
        3 => {
            ship_box(
                &mut out,
                [0.15, 0.0, deck + 0.06],
                [0.1, beam * 0.4, 0.06],
                white,
            );
            ship_box(
                &mut out,
                [-0.2, 0.0, deck + 0.12],
                [0.005, 0.005, 0.12],
                [0.2, 0.2, 0.2],
            );
        }
        4 => ship_box(
            &mut out,
            [0.05, 0.0, deck + 0.08],
            [0.14, beam * 0.4, 0.08],
            white,
        ),
        5 => {
            ship_box(
                &mut out,
                [0.05, 0.0, deck + 0.3],
                [0.004, 0.004, 0.3],
                [0.8, 0.8, 0.8],
            );
            // Mainsail: a thin triangle-like slab.
            ship_box(
                &mut out,
                [-0.08, 0.0, deck + 0.25],
                [0.12, 0.002, 0.22],
                [0.95, 0.95, 0.92],
            );
        }
        _ => ship_box(
            &mut out,
            [-0.1, 0.0, deck + 0.05],
            [0.12, beam * 0.4, 0.05],
            white,
        ),
    }
    out
}

/// Tree models (model space: base at the origin, height 1, +z up):
/// 0 coconut palm, 1 broadleaf, 2 conifer.
pub(super) const TREE_KINDS: usize = 3;

fn vertex(p: Vec3, n: Vec3, colour: [f32; 3], rough: f32) -> DrawVertex {
    DrawVertex {
        position: [p.x as f32, p.y as f32, p.z as f32],
        normal: [n.x as f32, n.y as f32, n.z as f32],
        uv: [0.0, 0.0],
        albedo: colour,
        roughness: rough,
        metallic: 0.0,
        kind: 3.0,
    }
}

/// A tapered tube along a polyline (trunks).
fn tube(out: &mut Vec<DrawVertex>, spine: &[(Vec3, f64)], colour: [f32; 3]) {
    let sides = 6;
    for w in spine.windows(2) {
        let (a, ra) = w[0];
        let (b, rb) = w[1];
        for k in 0..sides {
            let t0 = k as f64 / sides as f64 * std::f64::consts::TAU;
            let t1 = (k + 1) as f64 / sides as f64 * std::f64::consts::TAU;
            let d = |t: f64| Vec3::new(t.cos(), t.sin(), 0.0);
            let q = [
                a + d(t0) * ra,
                b + d(t0) * rb,
                b + d(t1) * rb,
                a + d(t1) * ra,
            ];
            let n = [d(t0), d(t0), d(t1), d(t1)];
            for i in [0usize, 2, 1, 0, 3, 2] {
                out.push(vertex(q[i], n[i], colour, 0.9));
            }
        }
    }
}

/// A blob of foliage: an ellipsoid with radii (rx, ry, rz) at `c`.
fn crown(out: &mut Vec<DrawVertex>, c: Vec3, r: Vec3, colour: [f32; 3], rings: usize) {
    let sides = rings * 2;
    let point = |i: usize, j: usize| {
        let v = std::f64::consts::PI * i as f64 / rings as f64;
        let u = std::f64::consts::TAU * j as f64 / sides as f64;
        let n = Vec3::new(v.sin() * u.cos(), v.sin() * u.sin(), v.cos());
        (c + Vec3::new(n.x * r.x, n.y * r.y, n.z * r.z), n)
    };
    for i in 0..rings {
        for j in 0..sides {
            let q = [
                point(i, j),
                point(i + 1, j),
                point(i + 1, j + 1),
                point(i, j + 1),
            ];
            for k in [0usize, 1, 2, 0, 2, 3] {
                // Darker underneath, lit tops.
                let shade = 0.75 + 0.35 * q[k].1.z.max(0.0) as f32;
                let col = [colour[0] * shade, colour[1] * shade, colour[2] * shade];
                out.push(vertex(q[k].0, q[k].1, col, 0.8));
            }
        }
    }
}

pub(super) fn tree_mesh(kind: usize) -> Vec<DrawVertex> {
    let mut out = vec![];
    let bark = [0.4, 0.32, 0.24];
    match kind {
        0 => {
            // Coconut palm: a slender leaning trunk and drooping fronds.
            let lean = Vec3::new(0.08, 0.0, 0.0);
            let spine: Vec<(Vec3, f64)> = (0..=5)
                .map(|i| {
                    let t = i as f64 / 5.0;
                    (
                        lean * (t * t) + Vec3::new(0.0, 0.0, t * 0.92),
                        0.022 - 0.008 * t,
                    )
                })
                .collect();
            tube(&mut out, &spine, [0.62, 0.56, 0.46]);
            let top = lean + Vec3::new(0.0, 0.0, 0.92);
            let frond = [0.16, 0.3, 0.07];
            for f in 0..9 {
                let a = f as f64 / 9.0 * std::f64::consts::TAU;
                let dir = Vec3::new(a.cos(), a.sin(), 0.0);
                let side = Vec3::new(-a.sin(), a.cos(), 0.0);
                // An arching frond in three segments, narrowing to the tip.
                let p =
                    |t: f64| top + dir * (t * 0.38) + Vec3::new(0.0, 0.0, 0.12 * t - 0.28 * t * t);
                for k in 0..3 {
                    let (t0, t1) = (k as f64 / 3.0, (k + 1) as f64 / 3.0);
                    let (w0, w1) = (0.06 * (1.0 - t0 * 0.7), 0.06 * (1.0 - t1 * 0.7));
                    let q = [
                        p(t0) - side * w0,
                        p(t0) + side * w0,
                        p(t1) + side * w1,
                        p(t1) - side * w1,
                    ];
                    let n = Vec3::new(0.0, 0.0, 1.0);
                    for i in [0usize, 2, 1, 0, 3, 2] {
                        out.push(vertex(q[i], n, frond, 0.7));
                    }
                }
            }
            crown(
                &mut out,
                top,
                Vec3::new(0.04, 0.04, 0.04),
                [0.35, 0.3, 0.12],
                3,
            );
        }
        1 => {
            tube(
                &mut out,
                &[(Vec3::ZERO, 0.05), (Vec3::new(0.0, 0.0, 0.45), 0.035)],
                bark,
            );
            let leaf = [0.12, 0.22, 0.07];
            crown(
                &mut out,
                Vec3::new(0.0, 0.0, 0.62),
                Vec3::new(0.34, 0.34, 0.3),
                leaf,
                5,
            );
            crown(
                &mut out,
                Vec3::new(0.15, 0.08, 0.78),
                Vec3::new(0.2, 0.2, 0.18),
                leaf,
                4,
            );
            crown(
                &mut out,
                Vec3::new(-0.14, -0.1, 0.74),
                Vec3::new(0.2, 0.2, 0.17),
                leaf,
                4,
            );
        }
        _ => {
            tube(
                &mut out,
                &[(Vec3::ZERO, 0.035), (Vec3::new(0.0, 0.0, 0.25), 0.03)],
                bark,
            );
            let needles = [0.07, 0.14, 0.07];
            for (z, r, h) in [(0.18, 0.22, 0.4), (0.42, 0.17, 0.34), (0.64, 0.11, 0.36)] {
                // A cone of foliage.
                let sides = 8;
                for k in 0..sides {
                    let a0 = k as f64 / sides as f64 * std::f64::consts::TAU;
                    let a1 = (k + 1) as f64 / sides as f64 * std::f64::consts::TAU;
                    let rim = |a: f64| Vec3::new(r * a.cos(), r * a.sin(), z);
                    let apex = Vec3::new(0.0, 0.0, z + h);
                    let n = |a: f64| Vec3::new(a.cos(), a.sin(), r / h).normalized();
                    out.push(vertex(rim(a0), n(a0), needles, 0.8));
                    out.push(vertex(rim(a1), n(a1), needles, 0.8));
                    out.push(vertex(apex, n(0.5 * (a0 + a1)), needles, 0.8));
                }
            }
        }
    }
    out
}

/// AIS ship type to a hull model.
pub(super) fn ship_kind(ais_type: u16) -> usize {
    match ais_type {
        70..=79 => 0,
        80..=89 => 1,
        60..=69 => 2,
        30 => 3,
        31 | 32 | 50..=59 => 4,
        36 | 37 => 5,
        _ => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geo_round_trip_and_ship_models() {
        let g = Geo {
            lat: 10.5,
            lon: 72.6,
        };
        let (lat, lon) = g.to_latlon(12_000.0, -8_000.0);
        let (x, y) = g.to_xy(lat, lon);
        assert!((x - 12_000.0).abs() < 1e-6 && (y + 8_000.0).abs() < 1e-6);
        let across = Geo {
            lat: 0.0,
            lon: 179.9,
        };
        let (_, lon) = across.to_latlon(50_000.0, 0.0);
        assert!(lon < -179.0, "wraps the antimeridian: {lon}");
        for k in 0..SHIP_KINDS {
            let m = ship_mesh(k);
            assert!(!m.is_empty() && m.len().is_multiple_of(3));
            assert!(m.iter().all(|v| v.position[0].abs() <= 0.51));
        }
        assert_eq!(ship_kind(70), 0);
        assert_eq!(ship_kind(84), 1);
        assert_eq!(ship_kind(36), 5);
    }
}
