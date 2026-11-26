//! Detailed visual landscape for the GPU viewport: the scene seabed at full
//! grid resolution, a wide backdrop (real coast, islands and mountains out
//! to the horizon) stitched to it without cracks, and seabed life placed
//! from habitat maps: coral colonies, sea fans, sponges, rocks and swaying
//! seagrass. All of it is visual; physics uses only the scene grid.
use crate::{
    habitat::{Habitat, REEF, SEAGRASS},
    math::Vec3,
    ocean::{Bathymetry, Ocean},
    render_scene::{DrawVertex, RenderScene},
    terrain::HeightGrid,
};

/// Vertex kinds understood by the GPU shader (ocean.wgsl).
pub const TERRAIN_KIND: f32 = 6.0;
pub const CORAL_KIND: f32 = 7.0;
pub const FAN_KIND: f32 = 8.0;
pub const BLADE_KIND: f32 = 9.0;

pub struct Landscape<'a> {
    /// Wide, coarse elevation grid around the scene (visual only).
    pub backdrop: Option<&'a HeightGrid>,
    pub habitat: Option<&'a Habitat>,
    pub seed: u64,
    /// 0 = low, 1 = medium, 2 = high, 3 = ultra.
    pub quality: u8,
    /// Satellite colours for land in the scene and the backdrop.
    pub imagery: Option<&'a crate::habitat::Imagery>,
    pub backdrop_imagery: Option<&'a crate::habitat::Imagery>,
}

/// Geometry counts, for budgets and the UI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub terrain_triangles: usize,
    pub backdrop_triangles: usize,
    pub corals: usize,
    pub rocks: usize,
    pub seagrass: usize,
}

struct Random(u64);
impl Random {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[((self.next() * items.len() as f64) as usize).min(items.len() - 1)]
    }
    fn unit(&mut self) -> Vec3 {
        let z = self.range(-1.0, 1.0);
        let a = self.range(0.0, std::f64::consts::TAU);
        let r = (1.0 - z * z).sqrt();
        Vec3::new(r * a.cos(), r * a.sin(), z)
    }
}

fn xyz(p: Vec3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}
/// Satellite colour on land: the albedo becomes the imagery colour and the
/// (otherwise unused) metallic channel carries how much of it to use,
/// fading in above the waterline (ocean.wgsl, terrain).
fn land_colour(mut v: DrawVertex, imagery: Option<&crate::habitat::Imagery>) -> DrawVertex {
    let z = v.position[2] as f64;
    if z > 0.3
        && let Some(c) = imagery.and_then(|i| i.linear(v.position[0] as f64, v.position[1] as f64))
        // Water-coloured pixels on land (imagery and elevation a little out
        // of register at the shore) would paint the land navy: skip them.
        && !(c[2] > c[1] && c[2] > c[0] * 1.5)
    {
        let t = ((z - 0.3) / 1.7).clamp(0.0, 1.0) as f32;
        v.albedo = c;
        v.metallic = t * t * (3.0 - 2.0 * t);
    }
    v
}

fn vertex(p: Vec3, n: Vec3, uv: [f32; 2], color: [f32; 3], rough: f32, kind: f32) -> DrawVertex {
    DrawVertex {
        position: xyz(p),
        normal: xyz(n),
        uv,
        albedo: color,
        roughness: rough,
        metallic: 0.0,
        kind,
    }
}
fn triangle(out: &mut Vec<DrawVertex>, p: [Vec3; 3], color: [f32; 3], kind: f32) {
    let n = (p[1] - p[0]).cross(p[2] - p[0]);
    let n = if n.length() > 1e-12 {
        n.normalized()
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    for q in p {
        out.push(vertex(q, n, [q.x as f32, q.y as f32], color, 0.8, kind));
    }
}

/// Habitat tint applied to the seabed material (white = plain seabed).
fn tint(class: u8) -> [f32; 3] {
    match class {
        REEF => [1.0, 0.9, 0.82],
        SEAGRASS => [0.55, 0.72, 0.42],
        _ => [1.0, 1.0, 1.0],
    }
}

struct Detail<'a> {
    ocean: &'a Ocean,
    bounds: [f64; 4],
    step: f64,
}
impl Detail<'_> {
    fn elevation(&self, x: f64, y: f64) -> f64 {
        self.ocean.bottom.elevation(x, y)
    }
    fn normal(&self, x: f64, y: f64) -> Vec3 {
        let e = self.step * 0.5;
        Vec3::new(
            self.elevation(x - e, y) - self.elevation(x + e, y),
            self.elevation(x, y - e) - self.elevation(x, y + e),
            2.0 * e,
        )
        .normalized()
    }
    fn inside(&self, x: f64, y: f64) -> bool {
        let tol = self.step * 1e-3;
        x >= self.bounds[0] - tol
            && x <= self.bounds[2] + tol
            && y >= self.bounds[1] - tol
            && y <= self.bounds[3] + tol
    }
}

/// Streamed terrain beyond the scene (coasts, islands, seabed far away): a
/// grid mesh with satellite colours on land. Cells whose centre lies inside
/// one of `holes` ([x0, y0, x1, y1], drawn by finer layers) are left out,
/// and the edge next to a hole sits a little lower so the finer layer
/// always wins where the two overlap. `stride` thins the grid.
pub fn terrain_patch(
    grid: &HeightGrid,
    imagery: Option<&crate::habitat::Imagery>,
    holes: &[[f64; 4]],
    stride: usize,
    out: &mut Vec<DrawVertex>,
) {
    let stride = stride.max(1);
    let d = grid.spacing * stride as f64;
    let cols = (grid.columns - 1) / stride + 1;
    let rows = (grid.rows - 1) / stride + 1;
    let height = |i: usize, j: usize| {
        let (gi, gj) = (
            (i * stride).min(grid.columns - 1),
            (j * stride).min(grid.rows - 1),
        );
        grid.elevations[gj * grid.columns + gi]
    };
    let inside = |x: f64, y: f64, margin: f64| {
        holes.iter().any(|h| {
            x > h[0] - margin && x < h[2] + margin && y > h[1] - margin && y < h[3] + margin
        })
    };
    let white = [1.0, 1.0, 1.0];
    let mut nodes = Vec::with_capacity(cols * rows);
    for j in 0..rows {
        for i in 0..cols {
            let (x, y) = (grid.origin[0] + i as f64 * d, grid.origin[1] + j as f64 * d);
            let mut z = height(i, j);
            if inside(x, y, d * 1.5) {
                z -= d * 0.02 + 1.0;
            }
            let (l, r) = (
                height(i.saturating_sub(1), j),
                height((i + 1).min(cols - 1), j),
            );
            let (b, t) = (
                height(i, j.saturating_sub(1)),
                height(i, (j + 1).min(rows - 1)),
            );
            let n = Vec3::new(l - r, b - t, 2.0 * d).normalized();
            let p = Vec3::new(x, y, z);
            nodes.push(land_colour(
                vertex(p, n, [x as f32, y as f32], white, 0.9, TERRAIN_KIND),
                imagery,
            ));
        }
    }
    for j in 0..rows - 1 {
        for i in 0..cols - 1 {
            let (cx, cy) = (
                grid.origin[0] + (i as f64 + 0.5) * d,
                grid.origin[1] + (j as f64 + 0.5) * d,
            );
            if inside(cx, cy, 0.0) {
                continue;
            }
            let a = j * cols + i;
            for k in [a, a + 1, a + cols + 1, a, a + cols + 1, a + cols] {
                out.push(nodes[k]);
            }
        }
    }
}

/// Builds the GPU landscape layer: terrain, backdrop, structures and life.
pub fn build(ocean: &Ocean, look: &Landscape) -> (RenderScene, Counts) {
    let mut s = RenderScene::default();
    let mut counts = Counts::default();
    let (bounds, step, columns, rows) = match ocean.bottom.grid() {
        Some(g) => {
            let stride = if look.quality == 0 && g.columns > 129 {
                2
            } else {
                1
            };
            (
                g.bounds(),
                g.spacing * stride as f64,
                (g.columns - 1) / stride + 1,
                (g.rows - 1) / stride + 1,
            )
        }
        None => ([-256.0, -256.0, 256.0, 256.0], 4.0, 129, 129),
    };
    let detail = Detail {
        ocean,
        bounds,
        step,
    };
    // Scene terrain at full resolution.
    let grid: Vec<DrawVertex> = (0..rows * columns)
        .map(|i| {
            let x = bounds[0] + (i % columns) as f64 * step;
            let y = bounds[1] + (i / columns) as f64 * step;
            let p = Vec3::new(x, y, detail.elevation(x, y));
            let class = look.habitat.map_or(0, |h| h.at(x, y));
            land_colour(
                vertex(
                    p,
                    detail.normal(x, y),
                    [x as f32, y as f32],
                    tint(class),
                    0.9,
                    TERRAIN_KIND,
                ),
                look.imagery,
            )
        })
        .collect();
    for y in 0..rows - 1 {
        for x in 0..columns - 1 {
            let a = y * columns + x;
            for i in [a, a + 1, a + columns, a + 1, a + columns + 1, a + columns] {
                s.opaque.push(grid[i]);
            }
        }
    }
    counts.terrain_triangles = (rows - 1) * (columns - 1) * 2;
    let before = s.opaque.len();
    let synthetic;
    let backdrop = match look.backdrop {
        Some(b) => b,
        None => {
            synthetic = apron(&detail, look.seed);
            &synthetic
        }
    };
    backdrop_mesh(&mut s.opaque, &detail, backdrop, look);
    counts.backdrop_triangles = (s.opaque.len() - before) / 3;
    for b in &ocean.structures {
        s.cuboid(
            b.center,
            b.half,
            crate::math::Quat::IDENTITY,
            b.color.map(|v| (v / 255.).powf(2.2)),
            0.05,
        );
    }
    let mut rng = Random(look.seed ^ 0x5eab_ed11_fe00_0001);
    life(&mut s.opaque, &detail, look, &mut rng, &mut counts);
    (s, counts)
}

/// Gently deepening, slightly rough seabed around a scene that has no real
/// backdrop, so the scene edge is not a cliff into nothing.
fn apron(detail: &Detail, seed: u64) -> HeightGrid {
    let (w, h) = (
        detail.bounds[2] - detail.bounds[0],
        detail.bounds[3] - detail.bounds[1],
    );
    let half = w.max(h) * 0.5;
    // Backdrop spacing: a whole number of scene cells that divides both
    // sides, near a sixth of the half-size, so the stitched ring lines up.
    let (nx, ny) = (
        (w / detail.step).round() as usize,
        (h / detail.step).round() as usize,
    );
    let target = (half / 6.0 / detail.step).max(1.0);
    let ratio = (1..=nx.max(1))
        .filter(|r| nx % r == 0 && ny % r == 0)
        .min_by(|a, b| {
            ((*a as f64) - target)
                .abs()
                .total_cmp(&((*b as f64) - target).abs())
        })
        .unwrap_or(1) as f64;
    let spacing = detail.step * ratio;
    let cells_x = (w / spacing).round() as i64;
    let cells_y = (h / spacing).round() as i64;
    let reach = ((6000.0f64.max(half * 6.0) / spacing).ceil() as i64)
        .min((256 - cells_x.max(cells_y)) / 2)
        .max(1);
    let (sx, sy) = (
        (cells_x + 2 * reach + 1) as usize,
        (cells_y + 2 * reach + 1) as usize,
    );
    let origin = [
        detail.bounds[0] - reach as f64 * spacing,
        detail.bounds[1] - reach as f64 * spacing,
    ];
    let phase = (seed % 1000) as f64 * 0.37;
    let mut elevations = Vec::with_capacity(sx * sy);
    for j in 0..sy {
        for i in 0..sx {
            let x = origin[0] + i as f64 * spacing;
            let y = origin[1] + j as f64 * spacing;
            let cx = x.clamp(detail.bounds[0], detail.bounds[2]);
            let cy = y.clamp(detail.bounds[1], detail.bounds[3]);
            let edge = detail.elevation(cx, cy);
            let dist = (x - cx).hypot(y - cy);
            let base = edge.min(-15.0) - dist * 0.02;
            let t = (dist / 1500.0).min(1.0);
            let w = t * t * (3.0 - 2.0 * t);
            let rough = ((x * 0.0011 + phase).sin() * (y * 0.0013 - phase).cos()) * 6.0;
            elevations.push(edge * (1.0 - w) + (base + rough) * w);
        }
    }
    HeightGrid::new(sx, sy, origin, spacing, elevations).expect("valid apron")
}

/// Backdrop cells outside the scene area. When the scene bounds lie on
/// backdrop grid lines and the spacings divide, the ring of cells touching
/// the scene is fanned onto the scene's boundary vertices (no T-junction
/// cracks); otherwise covered cells are sunk below the scene terrain.
fn backdrop_mesh(out: &mut Vec<DrawVertex>, detail: &Detail, b: &HeightGrid, look: &Landscape) {
    let quality = look.quality;
    let stride = if quality < 2 && b.columns > 129 { 2 } else { 1 };
    let d = b.spacing * stride as f64;
    let cols = (b.columns - 1) / stride + 1;
    let rows = (b.rows - 1) / stride + 1;
    let node =
        |p: usize, q: usize| Vec3::new(b.origin[0] + p as f64 * d, b.origin[1] + q as f64 * d, 0.0);
    let index_of = |v: f64, o: f64| (v - o) / d;
    let p0 = index_of(detail.bounds[0], b.origin[0]);
    let p1 = index_of(detail.bounds[2], b.origin[0]);
    let q0 = index_of(detail.bounds[1], b.origin[1]);
    let q1 = index_of(detail.bounds[3], b.origin[1]);
    let ratio = d / detail.step;
    let aligned = [p0, p1, q0, q1]
        .iter()
        .all(|v| (v - v.round()).abs() < 1e-6 && *v >= 0.0)
        && (ratio - ratio.round()).abs() < 1e-6
        && ratio >= 1.0
        && p1.round() < cols as f64
        && q1.round() < rows as f64;
    let height = |p: Vec3| -> f64 {
        if detail.inside(p.x, p.y) {
            let e = detail.elevation(p.x, p.y);
            if aligned { e } else { e - 3.0 }
        } else {
            b.elevation(p.x, p.y)
        }
    };
    let lift = |p: Vec3| Vec3::new(p.x, p.y, height(p));
    let normal = |p: Vec3| {
        let e = d * 0.5;
        Vec3::new(
            height(p - Vec3::new(e, 0.0, 0.0)) - height(p + Vec3::new(e, 0.0, 0.0)),
            height(p - Vec3::new(0.0, e, 0.0)) - height(p + Vec3::new(0.0, e, 0.0)),
            2.0 * e,
        )
        .normalized()
    };
    let white = [1.0, 1.0, 1.0];
    let push = |p: Vec3, out: &mut Vec<DrawVertex>| {
        let q = lift(p);
        let n = if detail.inside(p.x, p.y) {
            detail.normal(p.x, p.y)
        } else {
            normal(p)
        };
        let imagery = if detail.inside(p.x, p.y) {
            look.imagery
        } else {
            look.backdrop_imagery
        };
        out.push(land_colour(
            vertex(q, n, [q.x as f32, q.y as f32], white, 0.9, TERRAIN_KIND),
            imagery,
        ));
    };
    let (p0, p1, q0, q1) = (
        p0.round() as i64,
        p1.round() as i64,
        q0.round() as i64,
        q1.round() as i64,
    );
    let r = ratio.round() as usize;
    for q in 0..rows - 1 {
        for p in 0..cols - 1 {
            let (pi, qi) = (p as i64, q as i64);
            let inside_p = pi >= p0 && pi < p1;
            let inside_q = qi >= q0 && qi < q1;
            if aligned && inside_p && inside_q {
                continue;
            }
            let (a, bb, c, e) = (
                node(p, q),
                node(p + 1, q),
                node(p + 1, q + 1),
                node(p, q + 1),
            );
            // Which side (if any) of this cell lies on the scene boundary.
            let side = if !aligned {
                None
            } else if inside_p && qi == q0 - 1 {
                Some((a, bb, e, c)) // top edge e..c on y = y0
            } else if inside_p && qi == q1 {
                Some((e, c, a, bb)) // bottom edge a..bb on y = y1
            } else if inside_q && pi == p0 - 1 {
                Some((a, e, bb, c)) // right edge bb..c on x = x0
            } else if inside_q && pi == p1 {
                Some((bb, c, a, e)) // left edge a..e on x = x1
            } else {
                None
            };
            match side {
                Some((outer0, outer1, inner0, inner1)) => {
                    let points: Vec<Vec3> = (0..=r)
                        .map(|k| inner0 + (inner1 - inner0) * (k as f64 / r as f64))
                        .collect();
                    let mid = r / 2;
                    for k in 0..mid {
                        for v in [outer0, points[k], points[k + 1]] {
                            push(v, out);
                        }
                    }
                    for v in [outer0, points[mid], outer1] {
                        push(v, out);
                    }
                    for k in mid..r {
                        for v in [outer1, points[k], points[k + 1]] {
                            push(v, out);
                        }
                    }
                }
                None => {
                    for v in [a, bb, c, a, c, e] {
                        push(v, out);
                    }
                }
            }
        }
    }
}

const CORAL_COLOURS: [[f32; 3]; 10] = [
    [0.32, 0.20, 0.10],
    [0.45, 0.33, 0.12],
    [0.30, 0.12, 0.30],
    [0.55, 0.25, 0.32],
    [0.25, 0.28, 0.12],
    [0.55, 0.50, 0.25],
    [0.15, 0.25, 0.40],
    [0.55, 0.18, 0.08],
    [0.60, 0.55, 0.42],
    [0.40, 0.42, 0.20],
];

/// Orthonormal frame with `up` as the third axis.
fn frame(up: Vec3, yaw: f64) -> [Vec3; 3] {
    let helper = if up.z.abs() < 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let x0 = helper.cross(up).normalized();
    let y0 = up.cross(x0);
    let x = x0 * yaw.cos() + y0 * yaw.sin();
    let y = up.cross(x);
    [x, y, up]
}

/// Tapered n-sided tube from a to b (sides only).
#[allow(clippy::too_many_arguments)]
fn tube(
    out: &mut Vec<DrawVertex>,
    a: Vec3,
    b: Vec3,
    ra: f64,
    rb: f64,
    sides: usize,
    color: [f32; 3],
    kind: f32,
) {
    let axis = b - a;
    if axis.length() < 1e-6 {
        return;
    }
    let [x, y, _] = frame(axis.normalized(), 0.0);
    let ring = |c: Vec3, r: f64, i: usize| {
        let t = i as f64 / sides as f64 * std::f64::consts::TAU;
        c + (x * t.cos() + y * t.sin()) * r
    };
    for i in 0..sides {
        let (p, q) = (ring(a, ra, i), ring(a, ra, i + 1));
        let (u, v) = (ring(b, rb, i), ring(b, rb, i + 1));
        triangle(out, [p, q, v], color, kind);
        triangle(out, [p, v, u], color, kind);
    }
}

fn branching(
    out: &mut Vec<DrawVertex>,
    rng: &mut Random,
    base: Vec3,
    up: Vec3,
    size: f64,
    color: [f32; 3],
) {
    let mut stack = vec![(
        base,
        (up + rng.unit() * 0.15).normalized(),
        size * 0.32,
        size * 0.055,
        0,
    )];
    while let Some((start, dir, length, radius, level)) = stack.pop() {
        let end = start + dir * length;
        tube(out, start, end, radius, radius * 0.75, 4, color, CORAL_KIND);
        if level < 2 {
            let children = if level == 0 {
                3
            } else {
                2 + (rng.next() * 2.0) as usize
            };
            for _ in 0..children {
                let d = (dir + rng.unit() * 0.75 + up * 0.35).normalized();
                stack.push((end, d, length * 0.78, radius * 0.72, level + 1));
            }
        }
    }
}

fn massive(
    out: &mut Vec<DrawVertex>,
    rng: &mut Random,
    base: Vec3,
    up: Vec3,
    size: f64,
    color: [f32; 3],
) {
    let [x, y, z] = frame(up, rng.range(0.0, std::f64::consts::TAU));
    let (rings, segments) = (4, 9);
    let r = size * 0.5;
    let squash = rng.range(0.55, 0.85);
    let wobble: Vec<f64> = (0..(rings + 1) * segments)
        .map(|_| rng.range(0.85, 1.12))
        .collect();
    let point = |i: usize, j: usize| {
        let lat = i as f64 / rings as f64 * std::f64::consts::FRAC_PI_2;
        let lon = (j % segments) as f64 / segments as f64 * std::f64::consts::TAU;
        let w = if i == rings {
            1.0
        } else {
            wobble[i * segments + j % segments]
        };
        base - z * (r * 0.15)
            + (x * (lat.cos() * lon.cos()) + y * (lat.cos() * lon.sin())) * (r * w)
            + z * (lat.sin() * r * squash * w)
    };
    for i in 0..rings {
        for j in 0..segments {
            let (a, b, c, d) = (
                point(i, j),
                point(i, j + 1),
                point(i + 1, j + 1),
                point(i + 1, j),
            );
            triangle(out, [a, b, c], color, CORAL_KIND);
            if i + 1 < rings {
                triangle(out, [a, c, d], color, CORAL_KIND);
            }
        }
    }
}

fn table(
    out: &mut Vec<DrawVertex>,
    rng: &mut Random,
    base: Vec3,
    up: Vec3,
    size: f64,
    color: [f32; 3],
) {
    let [x, y, z] = frame(up, rng.range(0.0, std::f64::consts::TAU));
    let stem = size * rng.range(0.25, 0.4);
    let top = base + z * stem;
    tube(
        out,
        base,
        top,
        size * 0.06,
        size * 0.08,
        4,
        color,
        CORAL_KIND,
    );
    let n = 11;
    let rim: Vec<Vec3> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let r = size * 0.5 * rng.range(0.8, 1.05);
            top + (x * a.cos() + y * a.sin()) * r + z * (rng.range(-0.04, 0.06) * size)
        })
        .collect();
    let centre = top + z * (size * 0.05);
    for i in 0..n {
        let (a, b) = (rim[i], rim[(i + 1) % n]);
        triangle(out, [centre, a, b], color, CORAL_KIND);
        let low = z * (size * -0.04);
        triangle(out, [centre + low, b + low, a + low], color, CORAL_KIND);
    }
}

fn fan(
    out: &mut Vec<DrawVertex>,
    rng: &mut Random,
    base: Vec3,
    up: Vec3,
    size: f64,
    color: [f32; 3],
) {
    let [x, _, z] = frame(up, rng.range(0.0, std::f64::consts::TAU));
    let stalk = base + z * (size * 0.12);
    tube(
        out,
        base,
        stalk,
        size * 0.03,
        size * 0.025,
        3,
        color,
        CORAL_KIND,
    );
    let n = 8;
    let arc = |i: usize| {
        let t = i as f64 / n as f64;
        let a = (t - 0.5) * 2.1;
        stalk + (x * a.sin() + z * a.cos()) * size * rng_free_radius(t)
    };
    for i in 0..n {
        let (a, b) = (arc(i), arc(i + 1));
        let ta = i as f32 / n as f32;
        let tb = (i + 1) as f32 / n as f32;
        let normal = x.cross(z);
        out.push(vertex(stalk, normal, [0.5, 0.0], color, 0.8, FAN_KIND));
        out.push(vertex(a, normal, [ta, 1.0], color, 0.8, FAN_KIND));
        out.push(vertex(b, normal, [tb, 1.0], color, 0.8, FAN_KIND));
    }
}
fn rng_free_radius(t: f64) -> f64 {
    0.9 + 0.1 * (t * 9.0).sin()
}

fn sponge(
    out: &mut Vec<DrawVertex>,
    rng: &mut Random,
    base: Vec3,
    up: Vec3,
    size: f64,
    color: [f32; 3],
) {
    let height = size * rng.range(0.6, 1.0);
    let r = size * rng.range(0.22, 0.32);
    tube(
        out,
        base,
        base + up * height,
        r,
        r * 1.25,
        7,
        color,
        CORAL_KIND,
    );
    tube(
        out,
        base + up * (height * 0.95),
        base + up * (height * 0.1),
        r * 1.05,
        r * 0.7,
        7,
        color,
        CORAL_KIND,
    );
}

fn whips(
    out: &mut Vec<DrawVertex>,
    rng: &mut Random,
    base: Vec3,
    up: Vec3,
    size: f64,
    color: [f32; 3],
) {
    for _ in 0..4 {
        let mut p = base + rng.unit().component_mul(Vec3::new(0.15, 0.15, 0.0)) * size;
        let mut d = (up + rng.unit() * 0.3).normalized();
        for k in 0..3 {
            let next = p + d * (size * 0.3);
            let r = size * 0.025 * (1.0 - k as f64 * 0.3);
            tube(out, p, next, r, r * 0.7, 3, color, CORAL_KIND);
            p = next;
            d = (d + rng.unit() * 0.25).normalized();
        }
    }
}

fn rock(out: &mut Vec<DrawVertex>, rng: &mut Random, base: Vec3, size: f64) {
    // Octahedron subdivided once, scaled per vertex, half buried.
    let axes = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, -1.0),
    ];
    let faces = [
        [0, 2, 4],
        [2, 1, 4],
        [1, 3, 4],
        [3, 0, 4],
        [2, 0, 5],
        [1, 2, 5],
        [3, 1, 5],
        [0, 3, 5],
    ];
    let stretch = Vec3::new(
        rng.range(0.7, 1.3),
        rng.range(0.7, 1.3),
        rng.range(0.45, 0.8),
    );
    let seed = rng.next() * 100.0;
    let shape = |v: Vec3| {
        let v = v.normalized();
        let bump = 1.0
            + 0.18
                * ((v.x * 5.1 + seed).sin() * (v.y * 4.3 - seed).cos() + (v.z * 6.7).sin() * 0.5);
        base + v.component_mul(stretch) * (size * 0.5 * bump) - Vec3::new(0.0, 0.0, size * 0.12)
    };
    for f in faces {
        let (a, b, c) = (axes[f[0]], axes[f[1]], axes[f[2]]);
        let (ab, bc, ca) = ((a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5);
        for t in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            triangle(out, t.map(shape), [1.0, 1.0, 1.0], TERRAIN_KIND);
        }
    }
}

fn tuft(out: &mut Vec<DrawVertex>, rng: &mut Random, base: Vec3) {
    let blades = 5;
    for _ in 0..blades {
        let offset = Vec3::new(rng.range(-0.12, 0.12), rng.range(-0.12, 0.12), 0.0);
        let height = rng.range(0.25, 0.75);
        let width = rng.range(0.012, 0.022);
        let a = rng.range(0.0, std::f64::consts::TAU);
        let side = Vec3::new(a.cos(), a.sin(), 0.0) * width;
        let lean = Vec3::new(rng.range(-0.1, 0.1), rng.range(-0.1, 0.1), 1.0).normalized();
        let p = base + offset;
        let top = p + lean * height;
        let normal = side.cross(lean).normalized();
        let phase = rng.range(0.0, std::f64::consts::TAU) as f32;
        let green = [
            0.07 + rng.next() as f32 * 0.05,
            0.17 + rng.next() as f32 * 0.08,
            0.04,
        ];
        let v = |q: Vec3, h: f32| vertex(q, normal, [phase, h], green, 0.6, BLADE_KIND);
        out.push(v(p - side, 0.0));
        out.push(v(p + side, 0.0));
        out.push(v(top, 1.0));
    }
}

fn life(
    out: &mut Vec<DrawVertex>,
    detail: &Detail,
    look: &Landscape,
    rng: &mut Random,
    counts: &mut Counts,
) {
    let (coral_budget, rock_budget, tuft_budget) = match look.quality {
        0 => (500, 120, 700),
        1 => (1600, 260, 1800),
        2 => (3200, 450, 3600),
        _ => (6000, 800, 7000),
    };
    // Life concentrates around a focus: the mapped reef nearest the scene
    // centre (where the reef viewpoint looks), else the centre itself.
    let focus = look
        .habitat
        .and_then(|h| {
            let mut best: Option<(f64, f64, f64)> = None;
            for j in 0..h.rows {
                for i in 0..h.columns {
                    if h.classes[j * h.columns + i] != REEF {
                        continue;
                    }
                    let x = h.origin[0] + i as f64 * h.spacing;
                    let y = h.origin[1] + j as f64 * h.spacing;
                    let d = -detail.elevation(x, y);
                    let r = x.hypot(y) + if (4.0..12.0).contains(&d) { 0.0 } else { 600.0 };
                    if (3.0..22.0).contains(&d) && best.is_none_or(|b| r < b.2) {
                        best = Some((x, y, r));
                    }
                }
            }
            best.map(|b| (b.0, b.1))
        })
        .unwrap_or((0.0, 0.0));
    let weight = |x: f64, y: f64| (-((x - focus.0).hypot(y - focus.1)) / 260.0).exp() + 0.015;
    // Candidate cells per class, with weights favouring the scene centre.
    let mut reef: Vec<(f64, f64, f64)> = vec![];
    let mut grass: Vec<(f64, f64, f64)> = vec![];
    if let Some(h) = look.habitat {
        for j in 0..h.rows {
            for i in 0..h.columns {
                let class = h.classes[j * h.columns + i];
                if class == 0 {
                    continue;
                }
                let x = h.origin[0] + i as f64 * h.spacing;
                let y = h.origin[1] + j as f64 * h.spacing;
                if !detail.inside(x, y) {
                    continue;
                }
                let depth = -detail.elevation(x, y);
                if class == REEF && (0.3..40.0).contains(&depth) {
                    let light = if depth < 15.0 { 1.0 } else { 15.0 / depth };
                    reef.push((x, y, weight(x, y) * light));
                } else if class == SEAGRASS && (0.3..18.0).contains(&depth) {
                    grass.push((x, y, weight(x, y)));
                }
            }
        }
    }
    let spacing = look.habitat.map_or(detail.step, |h| h.spacing);
    // Weighted sampling by binary search on prefix sums of the weights.
    let cumulative = |cells: &[(f64, f64, f64)]| -> Vec<(f64, f64, f64)> {
        let mut sum = 0.0;
        cells
            .iter()
            .map(|c| {
                sum += c.2;
                (c.0, c.1, sum)
            })
            .collect()
    };
    let sample = |cells: &[(f64, f64, f64)], rng: &mut Random| -> Option<(f64, f64)> {
        let total = cells.last()?.2;
        if total <= 0.0 {
            return None;
        }
        let t = rng.next() * total;
        let i = cells.partition_point(|c| c.2 < t).min(cells.len() - 1);
        let c = cells[i];
        Some((
            c.0 + rng.range(-0.5, 0.5) * spacing,
            c.1 + rng.range(-0.5, 0.5) * spacing,
        ))
    };
    let reef_cells = cumulative(&reef);
    if !reef_cells.is_empty() {
        // Colonies cluster: each sampled point seeds a small group.
        let mut placed = 0;
        let mut attempts = 0;
        while placed < coral_budget && attempts < coral_budget * 4 {
            attempts += 1;
            let Some((cx, cy)) = sample(&reef_cells, rng) else {
                break;
            };
            let group = 2 + (rng.next() * 6.0) as usize;
            for _ in 0..group {
                let (x, y) = (cx + rng.range(-2.5, 2.5), cy + rng.range(-2.5, 2.5));
                if !detail.inside(x, y) {
                    continue;
                }
                let z = detail.elevation(x, y);
                let depth = -z;
                let n = detail.normal(x, y);
                if !(0.4..40.0).contains(&depth) || n.z < 0.6 {
                    continue;
                }
                let up = (n + Vec3::new(0.0, 0.0, 2.0)).normalized();
                let base = Vec3::new(x, y, z);
                let color = rng.pick(&CORAL_COLOURS);
                let kind = rng.next();
                let size = rng.range(0.35, 1.0) * if depth < 3.0 { 0.8 } else { 1.0 };
                if depth < 8.0 && kind < 0.35 {
                    branching(out, rng, base, up, size * 1.6, color);
                } else if depth < 12.0 && kind < 0.5 {
                    table(out, rng, base, up, size * 2.0, color);
                } else if kind < 0.75 {
                    massive(out, rng, base, up, size * 1.8, color);
                } else if kind < 0.85 {
                    let tone =
                        rng.pick(&[[0.45, 0.08, 0.30], [0.55, 0.12, 0.06], [0.6, 0.5, 0.12]]);
                    fan(out, rng, base, up, size * 1.3, tone);
                } else if kind < 0.93 {
                    let tone =
                        rng.pick(&[[0.6, 0.25, 0.05], [0.35, 0.12, 0.35], [0.45, 0.38, 0.15]]);
                    sponge(out, rng, base, up, size * 1.2, tone);
                } else {
                    whips(out, rng, base, up, size * 1.5, color);
                }
                placed += 1;
            }
        }
        counts.corals = placed;
    }
    // Rocks: on steep or rough seabed and along the shore.
    let mut rocks = 0;
    let mut attempts = 0;
    let (w, h) = (
        detail.bounds[2] - detail.bounds[0],
        detail.bounds[3] - detail.bounds[1],
    );
    while rocks < rock_budget && attempts < rock_budget * 20 {
        attempts += 1;
        // Biased towards the centre: radius from a squared uniform.
        let r = rng.next().powi(2) * 0.5 * w.max(h);
        let a = rng.range(0.0, std::f64::consts::TAU);
        let (x, y) = (r * a.cos(), r * a.sin());
        if !detail.inside(x, y) {
            continue;
        }
        let z = detail.elevation(x, y);
        let n = detail.normal(x, y);
        let steep = 1.0 - n.z;
        if z > 3.0 || (steep < 0.12 && rng.next() > 0.12) {
            continue;
        }
        let size = rng.range(0.4, 2.2) * if steep > 0.3 { 1.6 } else { 1.0 };
        rock(out, rng, Vec3::new(x, y, z), size);
        rocks += 1;
    }
    counts.rocks = rocks;
    let grass_cells = cumulative(&grass);
    let mut tufts = 0;
    if !grass_cells.is_empty() {
        let mut attempts = 0;
        while tufts < tuft_budget && attempts < tuft_budget * 3 {
            attempts += 1;
            let Some((cx, cy)) = sample(&grass_cells, rng) else {
                break;
            };
            for _ in 0..3 {
                let (x, y) = (cx + rng.range(-1.5, 1.5), cy + rng.range(-1.5, 1.5));
                let z = detail.elevation(x, y);
                if !detail.inside(x, y) || z > -0.3 {
                    continue;
                }
                tuft(out, rng, Vec3::new(x, y, z));
                tufts += 1;
            }
        }
    }
    counts.seagrass = tufts;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ocean::Bathymetry, terrain::HeightGrid};
    struct Grid(HeightGrid);
    impl Bathymetry for Grid {
        fn elevation(&self, x: f64, y: f64) -> f64 {
            self.0.elevation(x, y)
        }
        fn grid(&self) -> Option<&HeightGrid> {
            Some(&self.0)
        }
    }
    fn island(size: usize, spacing: f64) -> HeightGrid {
        let half = (size - 1) as f64 * spacing * 0.5;
        let elevations = (0..size * size)
            .map(|i| {
                let x = -half + (i % size) as f64 * spacing;
                let y = -half + (i / size) as f64 * spacing;
                12.0 - x.hypot(y) * 0.08
            })
            .collect();
        HeightGrid::new(size, size, [-half, -half], spacing, elevations).unwrap()
    }
    #[test]
    fn landscape_has_full_terrain_a_crack_free_backdrop_and_reef_life() {
        let grid = island(65, 8.0);
        let ocean = Ocean {
            bottom: Box::new(Grid(grid.clone())),
            ..Default::default()
        };
        // Backdrop aligned with the scene: spacing 32 = 4 cells, bounds ±256.
        let size = 49;
        let half = (size - 1) as f64 * 32.0 * 0.5;
        let backdrop = HeightGrid::new(
            size,
            size,
            [-half, -half],
            32.0,
            (0..size * size)
                .map(|i| {
                    let x = -half + (i % size) as f64 * 32.0;
                    let y = -half + (i / size) as f64 * 32.0;
                    12.0 - x.hypot(y) * 0.08
                })
                .collect(),
        )
        .unwrap();
        // Reef ring between 3 and 25 m deep.
        let classes = (0..65 * 65)
            .map(|i| {
                let x = -256.0 + (i % 65) as f64 * 8.0;
                let y = -256.0 + (i / 65) as f64 * 8.0;
                let depth = -(12.0 - x.hypot(y) * 0.08);
                if (3.0..25.0).contains(&depth) {
                    REEF
                } else if (0.5..3.0).contains(&depth) {
                    SEAGRASS
                } else {
                    0
                }
            })
            .collect();
        let habitat = Habitat::new(65, 65, [-256.0, -256.0], 8.0, classes, "synthetic").unwrap();
        let look = Landscape {
            backdrop: Some(&backdrop),
            habitat: Some(&habitat),
            seed: 3,
            quality: 2,
            imagery: None,
            backdrop_imagery: None,
        };
        let (scene, counts) = build(&ocean, &look);
        assert_eq!(counts.terrain_triangles, 64 * 64 * 2);
        // 48x48 backdrop cells minus the 16x16 under the scene.
        assert!(
            counts.backdrop_triangles >= (48 * 48 - 256) * 2,
            "{counts:?}"
        );
        assert!(counts.corals > 500 && counts.seagrass > 500, "{counts:?}");
        assert!(scene.opaque.len().is_multiple_of(3));
        assert!(
            scene
                .opaque
                .iter()
                .all(|v| v.position.iter().all(|c| c.is_finite()))
        );
        // Every backdrop vertex on the scene boundary has the scene's own
        // elevation, so the two surfaces meet without cracks.
        let terrain_end = counts.terrain_triangles * 3;
        for v in &scene.opaque[terrain_end..terrain_end + counts.backdrop_triangles * 3] {
            let (x, y) = (v.position[0] as f64, v.position[1] as f64);
            if (x.abs() - 256.0).abs() < 1e-3 && y.abs() <= 256.0 + 1e-3
                || (y.abs() - 256.0).abs() < 1e-3 && x.abs() <= 256.0 + 1e-3
            {
                let e = grid.elevation(x, y);
                assert!((v.position[2] as f64 - e).abs() < 1e-3, "{x} {y}");
            }
        }
        // Corals only where mapped and underwater.
        for v in scene.opaque.iter().filter(|v| v.kind == CORAL_KIND) {
            assert!(v.position[2] < 2.0);
        }
        // Without a backdrop, an apron is generated.
        let plain = Landscape {
            backdrop: None,
            habitat: None,
            seed: 3,
            quality: 0,
            imagery: None,
            backdrop_imagery: None,
        };
        let (_, c) = build(&ocean, &plain);
        assert!(c.backdrop_triangles > 1000 && c.corals == 0, "{c:?}");
    }
}

/// A named camera position for a scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Viewpoint {
    pub name: &'static str,
    pub eye: [f64; 3],
    pub target: [f64; 3],
}

/// Useful camera positions for a scene: over the open water, on the
/// surface, at the nearest mapped reef (underwater), facing the nearest
/// shore, and above the seabed.
pub fn viewpoints(
    grid: Option<&HeightGrid>,
    habitat: Option<&Habitat>,
    depth: f64,
) -> Vec<Viewpoint> {
    let elevation =
        |x: f64, y: f64| grid.map_or(-depth, |g| crate::ocean::Bathymetry::elevation(g, x, y));
    let mut out = vec![
        Viewpoint {
            name: "Aerial",
            eye: [-1500.0, -1500.0, 700.0],
            target: [0.0, 0.0, 0.0],
        },
        Viewpoint {
            name: "Sea surface",
            eye: [-60.0, -80.0, 3.0],
            target: [140.0, 120.0, 1.0],
        },
    ];
    // Seaward direction at a point: down the local slope.
    let seaward = |x: f64, y: f64| {
        let e = 10.0;
        let g = Vec3::new(
            elevation(x - e, y) - elevation(x + e, y),
            elevation(x, y - e) - elevation(x, y + e),
            0.0,
        );
        if g.length() < 1e-6 {
            Vec3::new(-1.0, -1.0, 0.0).normalized()
        } else {
            g.normalized()
        }
    };
    if let Some(h) = habitat {
        let mut best: Option<(f64, f64, f64)> = None;
        for j in 0..h.rows {
            for i in 0..h.columns {
                if h.classes[j * h.columns + i] != REEF {
                    continue;
                }
                let x = h.origin[0] + i as f64 * h.spacing;
                let y = h.origin[1] + j as f64 * h.spacing;
                let d = -elevation(x, y);
                if !(3.0..22.0).contains(&d) {
                    continue;
                }
                // Prefer well-lit reef (4-12 m) near the centre.
                let r = x.hypot(y) + if (4.0..12.0).contains(&d) { 0.0 } else { 600.0 };
                if best.is_none_or(|b| r < b.2) {
                    best = Some((x, y, r));
                }
            }
        }
        if let Some((x, y, _)) = best {
            let away = seaward(x, y);
            let mut eye = Vec3::new(x, y, 0.0) + away * 16.0;
            let floor = elevation(eye.x, eye.y);
            eye.z = (floor + 4.0).min(-2.0).max(floor + 1.5);
            let base = elevation(x, y);
            out.push(Viewpoint {
                name: "Coral reef (underwater)",
                eye: [eye.x, eye.y, eye.z],
                target: [x, y, base + 1.0],
            });
        }
    }
    if let Some(g) = grid {
        // Nearest land: look at it from 400 m out to sea, 3 m up.
        let mut best: Option<(f64, f64, f64)> = None;
        for j in (0..g.rows).step_by(2) {
            for i in (0..g.columns).step_by(2) {
                let e = g.elevations[j * g.columns + i];
                if e < 2.0 {
                    continue;
                }
                let x = g.origin[0] + i as f64 * g.spacing;
                let y = g.origin[1] + j as f64 * g.spacing;
                let r = x.hypot(y);
                if best.is_none_or(|b| r < b.2) {
                    best = Some((x, y, r));
                }
            }
        }
        if let Some((x, y, r)) = best {
            let toward = Vec3::new(x, y, 0.0) * (1.0 / r.max(1.0));
            let eye = Vec3::new(x, y, 0.0) - toward * 400.0;
            out.push(Viewpoint {
                name: "Shore",
                eye: [eye.x, eye.y, 3.0],
                target: [x, y, 15.0],
            });
        }
    }
    let floor = elevation(0.0, 0.0);
    out.push(Viewpoint {
        name: "Seabed",
        eye: [-20.0, -25.0, (floor + 3.0).min(-1.5)],
        target: [20.0, 25.0, floor + 2.0],
    });
    out
}
