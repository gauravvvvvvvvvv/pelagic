//! Backend-neutral geometry in world metres. Builds visual state from immutable
//! truth; quality controls sampling, never the physical world's population/dt.
use crate::{
    Snapshot,
    math::{Quat, Vec3},
    ocean::Ocean,
};
#[derive(Clone, Copy, Debug)]
pub struct DrawVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub albedo: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub kind: f32,
}
#[derive(Clone, Debug, Default)]
pub struct RenderScene {
    pub opaque: Vec<DrawVertex>,
    pub water: Vec<DrawVertex>,
}
fn xyz(p: Vec3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}
impl RenderScene {
    fn triangle(&mut self, p: [Vec3; 3], color: [f32; 3], rough: f32, metal: f32, kind: f32) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalized();
        for p in p {
            self.opaque.push(DrawVertex {
                position: xyz(p),
                normal: xyz(n),
                uv: [p.x as f32, p.y as f32],
                albedo: color,
                roughness: rough,
                metallic: metal,
                kind,
            });
        }
    }
    pub(crate) fn cuboid(
        &mut self,
        center: Vec3,
        half: Vec3,
        q: Quat,
        color: [f32; 3],
        metal: f32,
    ) {
        let p = [
            [-1., -1., -1.],
            [1., -1., -1.],
            [1., 1., -1.],
            [-1., 1., -1.],
            [-1., -1., 1.],
            [1., -1., 1.],
            [1., 1., 1.],
            [-1., 1., 1.],
        ]
        .map(|v| center + q.rotate(Vec3::new(v[0] * half.x, v[1] * half.y, v[2] * half.z)));
        for [a, b, c, d] in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            self.triangle([p[a], p[b], p[c]], color, 0.42, metal, 2.0);
            self.triangle([p[a], p[c], p[d]], color, 0.42, metal, 2.0);
        }
    }
    /// Complete visual geometry: `build_static` followed by `build_dynamic`.
    pub fn build(
        ocean: &Ocean,
        state: &Snapshot,
        quality: u8,
        material: u8,
    ) -> Result<Self, &'static str> {
        let mut dynamic = Self::build_dynamic(ocean, state, quality)?;
        let mut s = Self::build_static(ocean, quality, material);
        s.opaque.append(&mut dynamic.opaque);
        s.water = dynamic.water;
        Ok(s)
    }
    /// Geometry that depends only on the ocean description (seabed and
    /// structures), not on simulation time. Renderers can cache it across
    /// ticks. Water is always empty.
    pub fn build_static(ocean: &Ocean, quality: u8, material: u8) -> Self {
        let mut s = Self::default();
        let size = if quality == 0 { 65 } else { 129 };
        let bounds = ocean
            .bottom
            .grid()
            .map_or([-256., -256., 256., 256.], |g| g.bounds());
        let grid: Vec<_> = (0..size * size)
            .map(|i| {
                let x = bounds[0] + (bounds[2] - bounds[0]) * (i % size) as f64 / (size - 1) as f64;
                let y = bounds[1] + (bounds[3] - bounds[1]) * (i / size) as f64 / (size - 1) as f64;
                let z = ocean.bottom.elevation(x, y);
                let n = ocean
                    .bottom
                    .grid()
                    .map_or(Vec3::new(0., 0., 1.), |g| g.normal(x, y));
                let rock = match material {
                    1 => 0.,
                    2 => 1.,
                    _ => ((1. - n.z) * 4.).clamp(0., 1.) as f32,
                };
                let sand = [0.53, 0.43, 0.26];
                let stone = [0.25, 0.28, 0.27];
                DrawVertex {
                    position: [x as f32, y as f32, z as f32],
                    normal: xyz(n),
                    uv: [x as f32, y as f32],
                    albedo: std::array::from_fn(|j| sand[j] * (1. - rock) + stone[j] * rock),
                    roughness: 0.88,
                    metallic: 0.,
                    kind: 0.,
                }
            })
            .collect();
        for y in 0..size - 1 {
            for x in 0..size - 1 {
                let a = y * size + x;
                for i in [a, a + 1, a + size, a + 1, a + size + 1, a + size] {
                    s.opaque.push(grid[i]);
                }
            }
        }
        for b in &ocean.structures {
            s.cuboid(
                b.center,
                b.half,
                Quat::IDENTITY,
                b.color.map(|v| (v / 255.).powf(2.2)),
                0.05,
            );
        }
        s
    }
    /// Geometry that changes with simulation time: bodies, marine life,
    /// particles and the water surface.
    pub fn build_dynamic(
        ocean: &Ocean,
        state: &Snapshot,
        quality: u8,
    ) -> Result<Self, &'static str> {
        let mut s = Self::build_bodies(ocean, state)?;
        s.water = water_mesh(quality, Some((ocean, state.time)))?;
        Ok(s)
    }
    /// Undisplaced water mesh with the same layout as `build_dynamic`'s
    /// surface, marked `WAVE_KIND` so a GPU vertex shader can evaluate the
    /// wave model per frame. Depends only on quality.
    pub fn flat_water(quality: u8) -> Vec<DrawVertex> {
        water_mesh(quality, None).unwrap_or_default()
    }
    /// Bodies, marine life and particles only (no water).
    pub fn build_bodies(ocean: &Ocean, state: &Snapshot) -> Result<Self, &'static str> {
        if state.vehicles.len() > 32 || state.fish.len() > 256 || state.particles.len() > 512 {
            return Err("render population exceeds supported limits");
        }
        let mut s = Self::default();
        for body in &state.vehicles {
            let p = body.position;
            let q = body.orientation;
            let h = body.half;
            // Assigned library material replaces the body's primary surface.
            let material = ocean.materials.get(&body.id);
            let paint = |color: [f32; 3], rough: f32, metal: f32| {
                material.map_or((color, rough, metal), |m| {
                    (m.linear(), m.roughness as f32, m.metallic as f32)
                })
            };
            if let Some(mesh) = body.asset.and_then(|id| ocean.assets.get(&id)) {
                let (color, rough, metal) = paint([0.45, 0.52, 0.57], 0.42, 0.4);
                if s.opaque.len() + mesh.triangles.len() * 3 > 2_000_000 {
                    return Err("visual mesh budget exceeded");
                }
                for face in &mesh.triangles {
                    s.triangle(
                        face.map(|i| p + q.rotate(mesh.vertices[i].component_mul(h))),
                        color,
                        rough,
                        metal,
                        2.,
                    );
                }
            } else if body.kind == 1 {
                let hull = [
                    Vec3::new(-h.x, -h.y, 0.),
                    Vec3::new(h.x * 0.6, -h.y, 0.),
                    Vec3::new(h.x, 0., 0.),
                    Vec3::new(h.x * 0.6, h.y, 0.),
                    Vec3::new(-h.x, h.y, 0.),
                    Vec3::new(-h.x * 0.7, 0., -h.z),
                ]
                .map(|v| p + q.rotate(v));
                for face in [
                    [0, 1, 5],
                    [1, 2, 5],
                    [2, 3, 5],
                    [3, 4, 5],
                    [4, 0, 5],
                    [0, 4, 3],
                    [0, 3, 1],
                    [1, 3, 2],
                ] {
                    let (color, rough, metal) = paint([0.64, 0.69, 0.68], 0.35, 0.15);
                    s.triangle(face.map(|i| hull[i]), color, rough, metal, 2.);
                }
                s.cuboid(
                    p + q.rotate(Vec3::new(-0.2, 0., h.z * 0.4)),
                    Vec3::new(h.x * 0.25, h.y * 0.6, h.z * 0.6),
                    q,
                    [0.58, 0.64, 0.65],
                    0.1,
                );
            } else if body.kind == 2 {
                let (color, _, metal) = paint([0.9, 0.35, 0.04], 0.42, 0.05);
                s.cuboid(p, h, q, color, metal);
            } else {
                // Visual frame stays inside the declared physical proxy.
                for x in [-0.8, 0.8] {
                    for y in [-0.8, 0.8] {
                        s.cuboid(
                            p + q.rotate(Vec3::new(h.x * x, h.y * y, 0.)),
                            Vec3::new(h.x * 0.065, h.y * 0.065, h.z),
                            q,
                            [0.18, 0.22, 0.24],
                            0.75,
                        );
                    }
                }
                for z in [-0.82, 0.82] {
                    for y in [-0.8, 0.8] {
                        s.cuboid(
                            p + q.rotate(Vec3::new(0., h.y * y, h.z * z)),
                            Vec3::new(h.x, h.y * 0.065, h.z * 0.06),
                            q,
                            [0.18, 0.22, 0.24],
                            0.75,
                        );
                    }
                }
                for y in [-0.55, 0.55] {
                    s.cuboid(
                        p + q.rotate(Vec3::new(0., h.y * y, h.z * 0.64)),
                        Vec3::new(h.x * 0.88, h.y * 0.23, h.z * 0.27),
                        q,
                        paint([0.95, 0.64, 0.035], 0.42, 0.05).0,
                        paint([0.95, 0.64, 0.035], 0.42, 0.05).2,
                    );
                }
                s.cuboid(
                    p,
                    Vec3::new(h.x * 0.58, h.y * 0.56, h.z * 0.38),
                    q,
                    [0.10, 0.14, 0.16],
                    0.6,
                );
                s.cuboid(
                    p + q.rotate(Vec3::new(h.x * 0.64, 0., 0.)),
                    Vec3::new(h.x * 0.06, h.y * 0.29, h.z * 0.22),
                    q,
                    [0.015, 0.035, 0.045],
                    0.25,
                );
            }
        }
        for f in &state.fish {
            let forward = if f.velocity.length() > 0.01 {
                f.velocity.normalized()
            } else {
                Vec3::new(1., 0., 0.)
            };
            let right = forward.cross(Vec3::new(0., 0., 1.)).normalized();
            let up = right.cross(forward);
            let point = |x: f64, y: f64, z: f64| {
                f.position
                    + (forward * x
                        + right * (y + f.phase.sin() * (0.5 - x).max(0.).powi(2) * 0.13)
                        + up * z)
                        * f.size
            };
            let rings = [
                (-0.45, 0.025),
                (-0.25, 0.12),
                (0.05, 0.16),
                (0.35, 0.1),
                (0.55, 0.01),
            ];
            for pair in rings.windows(2) {
                for i in 0..8 {
                    let a = i as f64 * std::f64::consts::TAU / 8.;
                    let b = (i + 1) as f64 * std::f64::consts::TAU / 8.;
                    let p = [
                        point(pair[0].0, pair[0].1 * a.cos(), pair[0].1 * a.sin()),
                        point(pair[1].0, pair[1].1 * a.cos(), pair[1].1 * a.sin()),
                        point(pair[1].0, pair[1].1 * b.cos(), pair[1].1 * b.sin()),
                        point(pair[0].0, pair[0].1 * b.cos(), pair[0].1 * b.sin()),
                    ];
                    let color = if a.sin() > 0.2 {
                        [0.08, 0.23, 0.29]
                    } else {
                        [0.51, 0.61, 0.57]
                    };
                    s.triangle([p[0], p[1], p[2]], color, 0.36, 0.25, 3.);
                    s.triangle([p[0], p[2], p[3]], color, 0.36, 0.25, 3.);
                }
            }
            for z in [-0.32, 0.32] {
                s.triangle(
                    [
                        point(-0.4, 0., 0.),
                        point(-0.9, 0., z),
                        point(-0.73, 0., 0.),
                    ],
                    [0.12, 0.28, 0.31],
                    0.45,
                    0.1,
                    3.,
                );
            }
        }
        for p in &state.particles {
            let r = 0.015;
            s.triangle(
                [
                    p.position + Vec3::new(-r, 0., -r),
                    p.position + Vec3::new(r, 0., -r),
                    p.position + Vec3::new(0., 0., r),
                ],
                [0.6, 0.7, 0.65],
                1.,
                0.,
                4.,
            );
        }
        Ok(s)
    }
}
/// Vertex `kind` of undisplaced water whose height and normal are evaluated
/// in the vertex shader from the sea state.
pub const WAVE_KIND: f32 = 5.0;
/// Water surface: a fine grid around the origin plus coarse rings to the
/// horizon. With `sampled`, heights and normals come from the exact wave
/// model at that time (kind 1); without, the mesh is flat (`WAVE_KIND`).
fn water_mesh(
    quality: u8,
    sampled: Option<(&Ocean, f64)>,
) -> Result<Vec<DrawVertex>, &'static str> {
    let size: usize = match quality {
        0 => 65,
        1 => 129,
        _ => 193,
    };
    let radius = 512.;
    let spacing = radius * 2. / (size - 1) as f64;
    let kind = if sampled.is_some() { 1. } else { WAVE_KIND };
    let vertex = |p: [f64; 3], normal: [f32; 3], uv: [f32; 2]| DrawVertex {
        position: [p[0] as f32, p[1] as f32, p[2] as f32],
        normal,
        uv,
        albedo: [0.012, 0.12, 0.17],
        roughness: 0.12,
        metallic: 0.,
        kind,
    };
    let heights: Vec<(f64, [f32; 3])> = match sampled {
        Some((ocean, time)) => ocean
            .waves
            .surface_grid([-radius, -radius], spacing, size, size, time)?
            .iter()
            .map(|w| (w.height, xyz(w.normal)))
            .collect(),
        None => vec![(0., [0., 0., 1.]); size * size],
    };
    let vertices: Vec<_> = heights
        .iter()
        .enumerate()
        .map(|(i, &(height, normal))| {
            let (x, y) = ((i % size) as f64, (i / size) as f64);
            vertex(
                [-radius + x * spacing, -radius + y * spacing, height],
                normal,
                [(x * spacing) as f32, (y * spacing) as f32],
            )
        })
        .collect();
    let mut water = Vec::with_capacity((size - 1) * (size - 1) * 6 + 4 * 4 * 16 * 6);
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let a = y * size + x;
            for i in [a, a + 1, a + size, a + 1, a + size + 1, a + size] {
                water.push(vertices[i]);
            }
        }
    }
    // Extend to the horizon using the same exact point-query wave model.
    // Each ring is sampled once (4 sides x 17 points) and shared by the
    // quads on both sides of it.
    let ring = |r: f64| -> [[DrawVertex; 17]; 4] {
        std::array::from_fn(|side| {
            std::array::from_fn(|i| {
                let t = -r + 2. * r * i as f64 / 16.;
                let p = match side {
                    0 => [t, -r],
                    1 => [r, t],
                    2 => [-t, r],
                    _ => [-r, -t],
                };
                let (height, normal) = sampled.map_or((0., [0., 0., 1.]), |(ocean, time)| {
                    let w = ocean.surface(p[0], p[1], time);
                    (w.height, xyz(w.normal))
                });
                vertex([p[0], p[1], height], normal, [p[0] as f32, p[1] as f32])
            })
        })
    };
    let rings = [radius, 1024., 2048., 8192., 32768.].map(ring);
    for pair in rings.windows(2) {
        for (inner, outer) in pair[0].iter().zip(&pair[1]) {
            for segment in 0..16 {
                let v = [
                    inner[segment],
                    inner[segment + 1],
                    outer[segment + 1],
                    outer[segment],
                ];
                for i in [0, 1, 2, 0, 2, 3] {
                    water.push(v[i]);
                }
            }
        }
    }
    Ok(water)
}
