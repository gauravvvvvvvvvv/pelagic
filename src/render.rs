//! Depth-buffered CPU triangle rendering, independent of the simulation loop.
use crate::{Snapshot, math::Vec3, ocean::Ocean};
pub trait Renderer {
    type Frame;
    fn render(&mut self, ocean: &Ocean, state: &Snapshot) -> Self::Frame;
}
#[derive(Clone)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}
impl Frame {
    pub fn write_ppm(&self, mut w: impl std::io::Write) -> std::io::Result<()> {
        write!(w, "P6\n{} {}\n255\n", self.width, self.height)?;
        w.write_all(&self.rgb)
    }
}
#[derive(Clone, Copy)]
struct Vertex {
    p: Vec3,
    c: [f32; 3],
    uv: [f32; 2],
    material: u8,
}
#[derive(Clone, Copy)]
struct Screen {
    x: f32,
    y: f32,
    iz: f32,
    c: [f32; 3],
    uv: [f32; 2],
    material: u8,
}
#[derive(Clone, Copy)]
struct Ground {
    p: Vec3,
    n: Vec3,
    shadow: f64,
}
#[derive(Clone, Copy)]
struct Camera {
    position: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    focal: f64,
}
impl Camera {
    fn new(position: Vec3, target: Vec3, height: usize, fov: f64, up: Vec3) -> Self {
        let forward = (target - position).normalized();
        let mut right = forward.cross(up).normalized();
        if right.length() < 0.01 {
            right = Vec3::new(1.0, 0.0, 0.0);
        }
        Self {
            position,
            forward,
            right,
            up: right.cross(forward),
            focal: height as f64 / (2.0 * (fov.clamp(20.0, 120.0).to_radians() * 0.5).tan()),
        }
    }
    fn view(self, p: Vec3) -> Vec3 {
        let p = p - self.position;
        Vec3::new(p.dot(self.right), p.dot(self.up), p.dot(self.forward))
    }
}
pub struct SoftwareRenderer {
    width: usize,
    height: usize,
    pub camera: Vec3,
    pub target: Vec3,
    pub quality: u8,
    pub sun_elevation: f64,
    pub wireframe: bool,
    /// 0 automatic slope materials, 1 sand, 2 rock.
    pub terrain_material: u8,
    pub vertical_fov: f64,
    pub camera_up: Vec3,
    pub selected: Option<u64>,
    depth: Vec<f32>,
    pixels: Vec<[f32; 3]>,
    ground: Vec<Ground>,
    ground_size: usize,
    sky_pixels: Vec<[f32; 3]>,
    sky_key: Option<(Vec3, Vec3, f64, bool, f64, Vec3)>,
    ground_key: Option<(usize, u64, u64, u8)>,
    terrain_vertices: Vec<Vertex>,
    water_vertices: Vec<Vertex>,
}
impl SoftwareRenderer {
    pub fn new(width: usize, height: usize) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || width.checked_mul(height).is_none_or(|n| n > 16_777_216) {
            return Err("invalid frame size");
        }
        Ok(Self {
            width,
            height,
            camera: Vec3::new(-95.0, -125.0, 34.0),
            target: Vec3::new(0.0, 15.0, 0.0),
            quality: 1,
            sun_elevation: 0.62,
            wireframe: false,
            terrain_material: 0,
            vertical_fov: 58.497,
            camera_up: Vec3::new(0.0, 0.0, 1.0),
            selected: None,
            depth: vec![f32::INFINITY; width * height],
            pixels: vec![[0.0; 3]; width * height],
            ground: vec![],
            ground_size: 0,
            sky_pixels: vec![],
            sky_key: None,
            ground_key: None,
            terrain_vertices: vec![],
            water_vertices: vec![],
        })
    }
    pub fn resize(&mut self, width: usize, height: usize) -> Result<(), &'static str> {
        if width == 0 || height == 0 || width.checked_mul(height).is_none_or(|n| n > 4_194_304) {
            return Err("invalid viewport size");
        }
        if width != self.width || height != self.height {
            self.width = width;
            self.height = height;
            self.depth.resize(width * height, f32::INFINITY);
            self.pixels.resize(width * height, [0.0; 3]);
            self.sky_key = None;
        }
        Ok(())
    }
    pub fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }
    /// CPU preview of the backend-neutral geometry. Uses approximate vertex
    /// lighting; GPU material/refraction parity is explicitly not implied.
    pub fn render_shared(
        &mut self,
        ocean: &Ocean,
        state: &Snapshot,
        scene: &crate::render_scene::RenderScene,
    ) -> Frame {
        let camera = Camera::new(
            self.camera,
            self.target,
            self.height,
            self.vertical_fov,
            self.camera_up,
        );
        let under = self.camera.z
            < ocean
                .surface(self.camera.x, self.camera.y, state.time)
                .height;
        let sun = self.sun();
        self.depth.fill(f32::INFINITY);
        let key = (
            self.camera,
            self.target,
            self.sun_elevation,
            under,
            self.vertical_fov,
            self.camera_up,
        );
        if self.sky_key != Some(key) {
            self.sky_pixels.clear();
            for y in 0..self.height {
                for x in 0..self.width {
                    let ray = (camera.forward
                        + camera.right * ((x as f64 - self.width as f64 * 0.5) / camera.focal)
                        + camera.up * ((self.height as f64 * 0.5 - y as f64) / camera.focal))
                        .normalized();
                    self.sky_pixels.push(if under {
                        mix([5., 28., 46.], [18., 86., 105.], (ray.z * 0.5 + 0.5) as f32)
                    } else {
                        Self::sky(ray, sun)
                    });
                }
            }
            self.sky_key = Some(key);
        }
        self.pixels.copy_from_slice(&self.sky_pixels);
        for (vertices, water) in [(&scene.opaque, false), (&scene.water, true)] {
            for tri in vertices.as_chunks::<3>().0 {
                let v = tri.map(|v| {
                    let p = Vec3::new(
                        v.position[0] as f64,
                        v.position[1] as f64,
                        v.position[2] as f64,
                    );
                    let n = Vec3::new(v.normal[0] as f64, v.normal[1] as f64, v.normal[2] as f64);
                    let light = (0.35 + 0.65 * n.dot(sun).abs()) as f32;
                    let color = if water {
                        let view = (self.camera - p).normalized();
                        let reflected = n * (2. * n.dot(view)) - view;
                        mix(
                            if under {
                                [22., 131., 146.]
                            } else {
                                [9., 67., 91.]
                            },
                            Self::sky(reflected, sun),
                            (0.04 + 0.96 * (1. - n.dot(view).abs()).powi(5)) as f32,
                        )
                    } else {
                        v.albedo.map(|c| (c * light).max(0.).powf(1. / 2.2) * 255.)
                    };
                    Vertex {
                        p,
                        c: self.fog(color, p, under, ocean),
                        uv: v.uv,
                        material: if v.kind == 0. { 1 } else { 0 },
                    }
                });
                self.triangle(v, camera);
            }
        }
        Frame {
            width: self.width,
            height: self.height,
            rgb: self
                .pixels
                .iter()
                .flat_map(|c| c.map(|v| v.clamp(0., 255.) as u8))
                .collect(),
        }
    }
    fn sun(&self) -> Vec3 {
        Vec3::new(
            -0.35,
            self.sun_elevation.cos() * 0.85,
            self.sun_elevation.sin(),
        )
        .normalized()
    }
    fn cache_ground(&mut self, ocean: &Ocean) {
        let key = (
            (&*ocean.bottom as *const dyn crate::ocean::Bathymetry) as *const () as usize,
            ocean.bottom.revision(),
            self.sun_elevation.to_bits(),
            self.quality,
        );
        if self.ground_key == Some(key) {
            return;
        }
        self.ground_key = Some(key);
        self.ground.clear();
        let size = if self.quality == 0 { 65 } else { 129 };
        self.ground_size = size;
        let sun = self.sun();
        let b = ocean
            .bottom
            .grid()
            .map_or([-256.0, -256.0, 256.0, 256.0], |g| g.bounds());
        for j in 0..size {
            for i in 0..size {
                let x = b[0] + (b[2] - b[0]) * i as f64 / (size - 1) as f64;
                let y = b[1] + (b[3] - b[1]) * j as f64 / (size - 1) as f64;
                let z = ocean.bottom.elevation(x, y);
                let n = Vec3::new(
                    ocean.bottom.elevation(x - 1.0, y) - ocean.bottom.elevation(x + 1.0, y),
                    ocean.bottom.elevation(x, y - 1.0) - ocean.bottom.elevation(x, y + 1.0),
                    2.0,
                )
                .normalized();
                let mut shadow = 1.0;
                for step in 1..12 {
                    let q = Vec3::new(x, y, z) + sun * (step as f64 * 4.0);
                    if ocean.bottom.elevation(q.x, q.y) > q.z + 0.6 {
                        shadow = 0.4;
                        break;
                    }
                }
                self.ground.push(Ground {
                    p: Vec3::new(x, y, z),
                    n,
                    shadow,
                });
            }
        }
    }
    fn sky(d: Vec3, sun: Vec3) -> [f32; 3] {
        let h = d.z.max(0.0);
        let glow = d.dot(sun).max(0.0);
        let mut c = mix(
            [166.0, 199.0, 211.0],
            [40.0, 103.0, 157.0],
            (h * 1.5).min(1.0) as f32,
        );
        let disc = glow.powf(1400.0) as f32;
        let haze = glow.powf(12.0) as f32 * 20.0;
        c[0] += disc * 180.0 + haze;
        c[1] += disc * 161.0 + haze * 0.8;
        c[2] += disc * 115.0 + haze * 0.5;
        let cloud = ((d.x * 9.0 + d.y * 2.5).sin() * (d.y * 12.0 - d.x * 3.0).cos() * 0.5 + 0.5)
            .powi(5)
            * (1.0 - h).powi(4)
            * 0.28;
        mix(c, [221.0, 229.0, 225.0], cloud as f32)
    }
    fn fog(&self, color: [f32; 3], p: Vec3, under: bool, ocean: &Ocean) -> [f32; 3] {
        let distance = (p - self.camera).length();
        if under {
            let mut out = color;
            for i in 0..3 {
                let t =
                    (-([0.03, 0.012, 0.006][i] + ocean.attenuation * 0.2) * distance).exp() as f32;
                out[i] = out[i] * t + [8.0, 53.0, 72.0][i] * (1.0 - t);
            }
            out
        } else {
            mix(
                color,
                [166.0, 199.0, 211.0],
                (1.0 - (-distance / 1100.0).exp()) as f32,
            )
        }
    }
    fn triangle(&mut self, vertices: [Vertex; 3], camera: Camera) {
        let input = vertices.map(|v| Vertex {
            p: camera.view(v.p),
            ..v
        });
        let horizontal = self.width as f64 * 0.5 / camera.focal;
        let vertical = self.height as f64 * 0.5 / camera.focal;
        let codes = input.map(|v| {
            let p = v.p;
            u8::from(p.z < 0.2)
                | (u8::from(p.x < -horizontal * p.z) << 1)
                | (u8::from(p.x > horizontal * p.z) << 2)
                | (u8::from(p.y < -vertical * p.z) << 3)
                | (u8::from(p.y > vertical * p.z) << 4)
        });
        if codes[0] & codes[1] & codes[2] != 0 {
            return;
        }
        if codes[0] | codes[1] | codes[2] == 0 {
            self.raster(input.map(|v| Screen {
                x: (self.width as f64 * 0.5 + v.p.x / v.p.z * camera.focal) as f32,
                y: (self.height as f64 * 0.5 - v.p.y / v.p.z * camera.focal) as f32,
                iz: (1.0 / v.p.z) as f32,
                c: v.c,
                uv: v.uv,
                material: v.material,
            }));
            return;
        }
        // Clip all five frustum planes before projection. Near-plane-only
        // clipping creates enormous screen coordinates and unstable horizons.
        let mut clipped = [input[0]; 12];
        clipped[..3].copy_from_slice(&input);
        let mut count = 3;
        for plane in 0..5 {
            let distance = |p: Vec3| match plane {
                0 => p.z - 0.2,
                1 => p.x + horizontal * p.z,
                2 => -p.x + horizontal * p.z,
                3 => p.y + vertical * p.z,
                _ => -p.y + vertical * p.z,
            };
            let mut out = [input[0]; 12];
            let mut n = 0;
            for edge in 0..count {
                let a = clipped[edge];
                let b = clipped[(edge + 1) % count];
                let da = distance(a.p);
                let db = distance(b.p);
                if da >= 0.0 {
                    out[n] = a;
                    n += 1;
                }
                if (da >= 0.0) != (db >= 0.0) {
                    let t = da / (da - db);
                    out[n] = Vertex {
                        p: a.p + (b.p - a.p) * t,
                        c: mix(a.c, b.c, t as f32),
                        uv: [
                            a.uv[0] + (b.uv[0] - a.uv[0]) * t as f32,
                            a.uv[1] + (b.uv[1] - a.uv[1]) * t as f32,
                        ],
                        material: a.material,
                    };
                    n += 1;
                }
            }
            if n < 3 {
                return;
            }
            clipped = out;
            count = n;
        }
        let screens = clipped.map(|v| Screen {
            x: (self.width as f64 * 0.5 + v.p.x / v.p.z * camera.focal) as f32,
            y: (self.height as f64 * 0.5 - v.p.y / v.p.z * camera.focal) as f32,
            iz: (1.0 / v.p.z) as f32,
            c: v.c,
            uv: v.uv,
            material: v.material,
        });
        for i in 1..count - 1 {
            self.raster([screens[0], screens[i], screens[i + 1]]);
        }
    }
    fn raster(&mut self, v: [Screen; 3]) {
        let edge = |a: Screen, b: Screen, x: f32, y: f32| {
            (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
        };
        let area = edge(v[0], v[1], v[2].x, v[2].y);
        if area.abs() < 0.001 {
            return;
        }
        let x0 = v
            .iter()
            .map(|p| p.x)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let x1 = v
            .iter()
            .map(|p| p.x)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .max(0.0)
            .min(self.width as f32) as usize;
        let y0 = v
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.0) as usize;
        let y1 = v
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .max(0.0)
            .min(self.height as f32) as usize;
        let inv = 1.0 / area;
        let color_z = v.map(|p| p.c.map(|c| c * p.iz));
        let uv_z = v.map(|p| p.uv.map(|u| u * p.iz));
        let dax = -(v[2].y - v[1].y) * inv;
        let dbx = -(v[0].y - v[2].y) * inv;
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let mut a = edge(v[1], v[2], x0 as f32 + 0.5, py) * inv;
            let mut b = edge(v[2], v[0], x0 as f32 + 0.5, py) * inv;
            for x in x0..x1 {
                let (a, b) = {
                    let values = (a, b);
                    a += dax;
                    b += dbx;
                    values
                };
                let c = 1.0 - a - b;
                if a < 0.0 || b < 0.0 || c < 0.0 {
                    continue;
                }
                let iz = a * v[0].iz + b * v[1].iz + c * v[2].iz;
                let z = 1.0 / iz;
                let i = y * self.width + x;
                if z >= self.depth[i] {
                    continue;
                }
                self.depth[i] = z;
                for (channel, value) in self.pixels[i].iter_mut().enumerate() {
                    *value = (a * color_z[0][channel]
                        + b * color_z[1][channel]
                        + c * color_z[2][channel])
                        * z;
                }
                if v[0].material == 1 {
                    let u = (a * uv_z[0][0] + b * uv_z[1][0] + c * uv_z[2][0]) * z;
                    let vv = (a * uv_z[0][1] + b * uv_z[1][1] + c * uv_z[2][1]) * z;
                    let hx = (u * 8.0).floor() as i32 as u32;
                    let hy = (vv * 8.0).floor() as i32 as u32;
                    let mut hash = hx.wrapping_mul(374761393) ^ hy.wrapping_mul(668265263);
                    hash = (hash ^ (hash >> 13)).wrapping_mul(1274126177);
                    let grain = (hash & 1023) as f32 / 1023.0;
                    let ripple = ((u * 1.25 + vv * 0.15).rem_euclid(1.0) - 0.5).abs() * 2.0;
                    // Band-limit material detail as its world footprint becomes
                    // smaller than a pixel. Prevents shimmering/moiré at grazing angles.
                    let footprint = z / self.height as f32;
                    let grain_weight = (1.0 - footprint * 32.0).clamp(0.0, 1.0);
                    let ripple_weight = 1.0 / (1.0 + (footprint * 6.0).powi(2));
                    let factor = 1.0
                        + (grain - 0.5) * 0.12 * grain_weight
                        + (ripple - 0.5) * 0.12 * ripple_weight;
                    for value in &mut self.pixels[i] {
                        *value *= factor;
                    }
                }
                if self.wireframe && a.min(b).min(c) < 0.025 {
                    self.pixels[i] = [100.0, 198.0, 190.0];
                }
            }
        }
    }
    fn grid(&mut self, v: &[Vertex], size: usize, camera: Camera) {
        for y in 0..size - 1 {
            for x in 0..size - 1 {
                let a = y * size + x;
                let b = a + 1;
                let c = a + size;
                let d = c + 1;
                self.triangle([v[a], v[b], v[c]], camera);
                self.triangle([v[b], v[d], v[c]], camera);
            }
        }
    }
    fn water(&self, ocean: &Ocean, x: f64, y: f64, time: f64, under: bool) -> Vertex {
        let wave = ocean.surface(x, y, time);
        self.water_sample(ocean, x, y, wave, under)
    }
    fn water_sample(
        &self,
        ocean: &Ocean,
        x: f64,
        y: f64,
        wave: crate::waves::SurfaceSample,
        under: bool,
    ) -> Vertex {
        let p = Vec3::new(x, y, wave.height);
        let view = (self.camera - p).normalized();
        let n = wave.normal;
        let fresnel = 0.02 + 0.98 * (1.0 - n.dot(view).abs().clamp(0.0, 1.0)).powi(5);
        let reflected = n * (2.0 * n.dot(view)) - view;
        let reflection = Self::sky(reflected, self.sun());
        let shallow = (-(wave.height - ocean.bottom.elevation(x, y)).max(0.0) * 0.09).exp() as f32;
        let water = if under {
            [22.0, 131.0, 146.0]
        } else {
            mix([9.0, 67.0, 91.0], [56.0, 151.0, 145.0], shallow)
        };
        let mut color = mix(water, reflection, if under { 0.15 } else { fresnel as f32 });
        let spec = n.dot((view + self.sun()).normalized()).abs().powf(110.0) as f32 * 185.0;
        color[0] += spec;
        color[1] += spec * 0.95;
        color[2] += spec * 0.81;
        let foam = (((1.0 - n.z) * 3.0).max(0.0) * wave.height.max(0.0) * 0.4).min(0.4) as f32;
        if !under {
            color = mix(color, [221.0, 234.0, 224.0], foam);
        }
        Vertex {
            p,
            c: self.fog(color, p, under, ocean),
            uv: [x as f32, y as f32],
            material: 2,
        }
    }
    #[allow(clippy::too_many_arguments)] // Explicit raster context; no heap allocation per primitive.
    fn cuboid(
        &mut self,
        center: Vec3,
        half: Vec3,
        base: [f32; 3],
        rotation: crate::math::Quat,
        camera: Camera,
        ocean: &Ocean,
        under: bool,
    ) {
        let corners = [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ]
        .map(|p| center + rotation.rotate(Vec3::new(p[0] * half.x, p[1] * half.y, p[2] * half.z)));
        for face in [
            [0, 3, 2, 1],
            [4, 5, 6, 7],
            [0, 1, 5, 4],
            [1, 2, 6, 5],
            [2, 3, 7, 6],
            [3, 0, 4, 7],
        ] {
            let n = (corners[face[1]] - corners[face[0]])
                .cross(corners[face[2]] - corners[face[0]])
                .normalized();
            let light = 0.32 + 0.68 * n.dot(self.sun()).max(0.0) as f32;
            let v = face.map(|i| Vertex {
                p: corners[i],
                c: self.fog(base.map(|v| v * light), corners[i], under, ocean),
                uv: [0.0; 2],
                material: 0,
            });
            self.triangle([v[0], v[1], v[2]], camera);
            self.triangle([v[0], v[2], v[3]], camera);
        }
    }
    fn shaded(
        &mut self,
        p: [Vec3; 3],
        color: [f32; 3],
        camera: Camera,
        ocean: &Ocean,
        under: bool,
    ) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalized();
        let light = (0.4 + 0.6 * n.dot(self.sun()).abs()) as f32;
        self.triangle(
            p.map(|p| Vertex {
                p,
                c: self.fog(color.map(|v| v * light), p, under, ocean),
                uv: [0.0; 2],
                material: 0,
            }),
            camera,
        );
    }
    fn fish(&mut self, f: &crate::life::Fish, camera: Camera, ocean: &Ocean, under: bool) {
        if (f.position - self.camera).length() > 100.0 {
            return;
        }
        let view = camera.view(f.position);
        let radius = f.size * 1.2;
        let h = self.width as f64 * 0.5 / camera.focal;
        let v = self.height as f64 * 0.5 / camera.focal;
        if view.z + radius < 0.2
            || view.x.abs() > h * view.z + radius * (1.0 + h * h).sqrt()
            || view.y.abs() > v * view.z + radius * (1.0 + v * v).sqrt()
        {
            return;
        }
        let forward = if f.velocity.length() > 0.01 {
            f.velocity.normalized()
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let right = forward.cross(Vec3::new(0.0, 0.0, 1.0)).normalized();
        let up = right.cross(forward);
        let point = |x: f64, y: f64, z: f64| {
            f.position
                + (forward * x
                    + right * (y + f.phase.sin() * (0.5 - x).max(0.0).powi(2) * 0.13)
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
        for r in rings.windows(2) {
            for i in 0..6 {
                let a = i as f64 * std::f64::consts::TAU / 6.0;
                let b = (i + 1) as f64 * std::f64::consts::TAU / 6.0;
                let p = [
                    point(r[0].0, r[0].1 * a.cos(), r[0].1 * a.sin()),
                    point(r[1].0, r[1].1 * a.cos(), r[1].1 * a.sin()),
                    point(r[1].0, r[1].1 * b.cos(), r[1].1 * b.sin()),
                    point(r[0].0, r[0].1 * b.cos(), r[0].1 * b.sin()),
                ];
                let color = if a.sin() > 0.2 {
                    [62.0, 113.0, 133.0]
                } else {
                    [174.0, 191.0, 177.0]
                };
                self.shaded([p[0], p[1], p[2]], color, camera, ocean, under);
                self.shaded([p[0], p[2], p[3]], color, camera, ocean, under);
            }
        }
        self.shaded(
            [
                point(-0.43, 0.0, 0.0),
                point(-0.9, 0.0, 0.32),
                point(-0.73, 0.0, 0.0),
            ],
            [67.0, 111.0, 123.0],
            camera,
            ocean,
            under,
        );
        self.shaded(
            [
                point(-0.43, 0.0, 0.0),
                point(-0.73, 0.0, 0.0),
                point(-0.9, 0.0, -0.32),
            ],
            [67.0, 111.0, 123.0],
            camera,
            ocean,
            under,
        );
        self.shaded(
            [
                point(0.05, 0.0, 0.15),
                point(-0.2, 0.0, 0.4),
                point(-0.3, 0.0, 0.08),
            ],
            [72.0, 121.0, 136.0],
            camera,
            ocean,
            under,
        );
        for side in [-1.0, 1.0] {
            self.shaded(
                [
                    point(0.37, side * 0.08, 0.02),
                    point(0.44, side * 0.07, 0.02),
                    point(0.4, side * 0.08, 0.08),
                ],
                [8.0, 13.0, 16.0],
                camera,
                ocean,
                under,
            );
        }
    }
}
impl Renderer for SoftwareRenderer {
    type Frame = Frame;
    fn render(&mut self, ocean: &Ocean, state: &Snapshot) -> Frame {
        self.cache_ground(ocean);
        self.depth.fill(f32::INFINITY);
        let camera = Camera::new(
            self.camera,
            self.target,
            self.height,
            self.vertical_fov,
            self.camera_up,
        );
        let under = self.camera.z
            < ocean
                .surface(self.camera.x, self.camera.y, state.time)
                .height;
        let sun = self.sun();
        let key = (
            self.camera,
            self.target,
            self.sun_elevation,
            under,
            self.vertical_fov,
            self.camera_up,
        );
        if self.sky_key != Some(key) {
            for y in 0..self.height {
                for x in 0..self.width {
                    let d = (camera.forward
                        + camera.right * ((x as f64 - self.width as f64 * 0.5) / camera.focal)
                        + camera.up * ((self.height as f64 * 0.5 - y as f64) / camera.focal))
                        .normalized();
                    self.pixels[y * self.width + x] = if under {
                        mix(
                            [5.0, 28.0, 46.0],
                            [18.0, 86.0, 105.0],
                            (d.z * 0.5 + 0.5).max(0.0) as f32,
                        )
                    } else {
                        Self::sky(d, sun)
                    };
                }
            }
            self.sky_pixels = self.pixels.clone();
            self.sky_key = Some(key);
        } else {
            self.pixels.copy_from_slice(&self.sky_pixels);
        }
        let mut terrain = std::mem::take(&mut self.terrain_vertices);
        terrain.clear();
        for v in &self.ground {
            let rock = match self.terrain_material {
                1 => 0.0,
                2 => 1.0,
                _ => ((1.0 - v.n.z) * 3.5).clamp(0.0, 1.0) as f32,
            };
            let ripple = ((v.p.x * 0.7 + (v.p.y * 0.2).sin()).sin() * 2.5) as f32;
            let mut color = if v.p.z > 2.0 {
                mix([89.0, 112.0, 63.0], [117.0, 115.0, 99.0], rock)
            } else {
                mix(
                    [190.0 + ripple, 175.0 + ripple, 132.0 + ripple],
                    [98.0, 107.0, 104.0],
                    rock,
                )
            };
            let light = (0.32 + 0.68 * v.n.dot(sun).max(0.0) * v.shadow) as f32;
            let caustic = if v.p.z < 0.0 {
                ((v.p.x * 0.38 + state.time * 0.8).sin() * (v.p.y * 0.43 - state.time * 0.5).cos())
                    .abs()
                    .powi(10) as f32
                    * 20.0
                    * (v.p.z * 0.03).exp() as f32
            } else {
                0.0
            };
            for value in &mut color {
                *value = *value * light + caustic;
            }
            if !under && v.p.z < 0.0 {
                color = mix([13.0, 76.0, 93.0], color, (v.p.z * 0.1).exp() as f32);
            }
            terrain.push(Vertex {
                p: v.p,
                c: self.fog(color, v.p, under, ocean),
                uv: [v.p.x as f32, v.p.y as f32],
                material: 1,
            });
        }
        self.grid(&terrain, self.ground_size, camera);
        self.terrain_vertices = terrain;
        for object in &ocean.structures {
            self.cuboid(
                object.center,
                object.half,
                object.color,
                crate::math::Quat::IDENTITY,
                camera,
                ocean,
                under,
            );
        }
        for vehicle in &state.vehicles {
            let q = vehicle.orientation;
            let p = vehicle.position;
            // CPU reference uses an assigned material's base colour only.
            let material = ocean.materials.get(&vehicle.id);
            let paint = |color: [f32; 3]| material.map_or(color, |m| m.srgb());
            if let Some(mesh) = vehicle.asset.and_then(|id| ocean.assets.get(&id)) {
                for face in &mesh.triangles {
                    let points =
                        face.map(|i| p + q.rotate(mesh.vertices[i].component_mul(vehicle.half)));
                    self.shaded(points, paint([157.0, 173.0, 184.0]), camera, ocean, under);
                }
                continue;
            }
            if vehicle.kind == 1 {
                let h = vehicle.half;
                let points = [
                    Vec3::new(-h.x, -h.y, 0.0),
                    Vec3::new(h.x * 0.6, -h.y, 0.0),
                    Vec3::new(h.x, 0.0, 0.0),
                    Vec3::new(h.x * 0.6, h.y, 0.0),
                    Vec3::new(-h.x, h.y, 0.0),
                    Vec3::new(-h.x * 0.8, 0.0, -h.z),
                ];
                let points = points.map(|v| p + q.rotate(v));
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
                    self.shaded(
                        face.map(|i| points[i]),
                        paint([194.0, 204.0, 204.0]),
                        camera,
                        ocean,
                        under,
                    );
                }
                self.cuboid(
                    p + q.rotate(Vec3::new(-0.3, 0.0, 0.35)),
                    Vec3::new(0.45, 0.4, 0.35),
                    [164.0, 185.0, 194.0],
                    q,
                    camera,
                    ocean,
                    under,
                );
                continue;
            }
            if vehicle.kind == 2 {
                self.cuboid(
                    p,
                    vehicle.half,
                    paint([235.0, 137.0, 48.0]),
                    q,
                    camera,
                    ocean,
                    under,
                );
                continue;
            }
            self.cuboid(
                p,
                vehicle.half.component_mul(Vec3::new(0.7, 0.55, 0.5)),
                [74.0, 91.0, 96.0],
                q,
                camera,
                ocean,
                under,
            );
            let fit = |v: Vec3| {
                v.component_mul(Vec3::new(
                    vehicle.half.x / 0.75,
                    vehicle.half.y / 0.5,
                    vehicle.half.z / 0.48,
                ))
            };
            for y in [-0.4, 0.4] {
                for z in [-0.3, 0.3] {
                    self.cuboid(
                        p + q.rotate(fit(Vec3::new(0.0, y, z))),
                        fit(Vec3::new(0.72, 0.035, 0.035)),
                        [111.0, 125.0, 130.0],
                        q,
                        camera,
                        ocean,
                        under,
                    );
                }
            }
            for x in [-0.64, 0.64] {
                for y in [-0.4, 0.4] {
                    self.cuboid(
                        p + q.rotate(fit(Vec3::new(x, y, 0.0))),
                        fit(Vec3::new(0.035, 0.035, 0.3)),
                        [111.0, 125.0, 130.0],
                        q,
                        camera,
                        ocean,
                        under,
                    );
                }
            }
            for y in [-0.38, 0.38] {
                self.cuboid(
                    p + q.rotate(fit(Vec3::new(0.0, y, 0.35))),
                    fit(Vec3::new(0.75, 0.12, 0.13)),
                    paint([223.0, 181.0, 56.0]),
                    q,
                    camera,
                    ocean,
                    under,
                );
            }
            self.cuboid(
                p + q.rotate(fit(Vec3::new(0.64, 0.0, 0.0))),
                fit(Vec3::new(0.04, 0.18, 0.15)),
                [139.0, 195.0, 205.0],
                q,
                camera,
                ocean,
                under,
            );
        }
        for fish in &state.fish {
            self.fish(fish, camera, ocean, under);
        }
        if let Some(selected) = self.selected
            && let Some(body) = state.vehicles.iter().find(|p| p.id == selected)
        {
            for (axis, color) in [
                (Vec3::new(1.0, 0.0, 0.0), [214.0, 84.0, 82.0]),
                (Vec3::new(0.0, 1.0, 0.0), [103.0, 193.0, 128.0]),
                (Vec3::new(0.0, 0.0, 1.0), [100.0, 153.0, 233.0]),
            ] {
                let side = if axis.z > 0.5 {
                    Vec3::new(0.0, 0.04, 0.0)
                } else {
                    Vec3::new(0.0, 0.0, 0.04)
                };
                let a = body.position + axis * 0.8;
                let b = body.position + axis * 2.0;
                self.shaded([a - side, a + side, b + side], color, camera, ocean, under);
                self.shaded([a - side, b + side, b - side], color, camera, ocean, under);
            }
        }
        for particle in &state.particles {
            if (particle.position - self.camera).length() < 80.0 {
                self.cuboid(
                    particle.position,
                    Vec3::new(0.025, 0.025, 0.025),
                    [148.0, 204.0, 201.0],
                    crate::math::Quat::IDENTITY,
                    camera,
                    ocean,
                    under,
                );
            }
        }
        if under {
            // A small authored seagrass patch; motion is cosmetic, no force model.
            for i in 0..18 {
                let x = -10.0 + (i % 6) as f64 * 2.0;
                let y = -5.0 + (i / 6) as f64 * 2.0;
                let z = ocean.bottom.elevation(x, y);
                for blade in 0..3 {
                    let sway = (state.time * 0.6 + i as f64 + blade as f64).sin() * 0.25;
                    let p = [
                        Vec3::new(x - 0.08, y, z),
                        Vec3::new(x + 0.08, y, z),
                        Vec3::new(
                            x + sway,
                            y + 0.15 * blade as f64,
                            z + 1.2 + blade as f64 * 0.3,
                        ),
                    ];
                    self.shaded(p, [44.0, 100.0, 61.0], camera, ocean, under);
                }
            }
        }
        let r = 384.0;
        // Concentric far-field LOD avoids interpolating fog across a single
        // kilometre-wide quad (which produces diagonal bands at the horizon).
        for radii in [r, 512.0, 768.0, 1024.0, 1536.0, 3072.0, 8192.0, 32768.0].windows(2) {
            for side in 0..4 {
                for segment in 0..16 {
                    let point = |radius: f64, i: usize| {
                        let along = -radius + 2.0 * radius * i as f64 / 16.0;
                        match side {
                            0 => [along, -radius],
                            1 => [radius, along],
                            2 => [-along, radius],
                            _ => [-radius, -along],
                        }
                    };
                    let v = [
                        point(radii[0], segment),
                        point(radii[0], segment + 1),
                        point(radii[1], segment + 1),
                        point(radii[1], segment),
                    ]
                    .map(|p| self.water(ocean, p[0], p[1], state.time, under));
                    self.triangle([v[0], v[1], v[2]], camera);
                    self.triangle([v[0], v[2], v[3]], camera);
                }
            }
        }
        let size = match self.quality {
            0 => 65,
            1 => 97,
            _ => 129,
        };
        let mut water = std::mem::take(&mut self.water_vertices);
        water.clear();
        let spacing = 2.0 * r / (size - 1) as f64;
        let samples = ocean
            .waves
            .surface_grid([-r, -r], spacing, size, size, state.time)
            .expect("bounded water grid");
        for y in 0..size {
            for x in 0..size {
                water.push(self.water_sample(
                    ocean,
                    -r + 2.0 * r * x as f64 / (size - 1) as f64,
                    -r + 2.0 * r * y as f64 / (size - 1) as f64,
                    samples[y * size + x],
                    under,
                ));
            }
        }
        self.grid(&water, size, camera);
        self.water_vertices = water;
        let mut rgb = vec![0; self.width * self.height * 3];
        for (i, c) in self.pixels.iter().enumerate() {
            for ch in 0..3 {
                rgb[i * 3 + ch] = c[ch].clamp(0.0, 255.0) as u8;
            }
        }
        Frame {
            width: self.width,
            height: self.height,
            rgb,
        }
    }
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
