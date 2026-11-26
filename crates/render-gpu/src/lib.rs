//! Native GPU backend. The headless core and CPU renderer do not depend on wgpu.
mod ocean_gpu;
pub mod sky;
pub mod spectrum;
use pelagic::{
    math::Vec3,
    render::Frame,
    render_scene::{DrawVertex, RenderScene},
    waves::SeaState,
};
pub use spectrum::DetailSea;
use wgpu::util::DeviceExt;
/// Source fingerprint for preview provenance, not a package security signature.
pub const RENDERER_BUILD_ID: u64 = {
    let sources: [&[u8]; 8] = [
        include_bytes!("lib.rs"),
        include_bytes!("ocean.wgsl"),
        include_bytes!("fft.wgsl"),
        include_bytes!("mips.wgsl"),
        include_bytes!("ocean_gpu.rs"),
        include_bytes!("spectrum.rs"),
        include_bytes!("sky.rs"),
        include_bytes!("../Cargo.toml"),
    ];
    let mut hash = 0xcbf29ce484222325u64;
    let mut s = 0;
    while s < sources.len() {
        let mut i = 0;
        while i < sources[s].len() {
            hash ^= sources[s][i] as u64;
            hash = hash.wrapping_mul(0x100000001b3);
            i += 1;
        }
        s += 1;
    }
    hash
};

/// Azimuth (radians, clockwise from north) of the fixed sun used before the
/// sun position became a scene setting.
pub const LEGACY_SUN_AZIMUTH: f64 = -0.390_607;

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_y: f64,
    pub sun_elevation: f64,
    /// Sun azimuth (radians, clockwise from north; scene Y is north).
    pub sun_azimuth: f64,
    /// Cloud cover fraction 0-1.
    pub clouds: f64,
    pub time: f64,
    pub attenuation: f64,
    pub underwater: bool,
}
impl View {
    fn validate(&self) -> Result<(), String> {
        if !self.eye.is_finite()
            || !self.target.is_finite()
            || !self.up.is_finite()
            || ![
                self.fov_y,
                self.sun_elevation,
                self.sun_azimuth,
                self.clouds,
                self.time,
                self.attenuation,
            ]
            .iter()
            .all(|v| v.is_finite())
            || !(20.0..=120.0).contains(&self.fov_y)
            || self.attenuation < 0.0
        {
            return Err("invalid render view".into());
        }
        Ok(())
    }
    pub fn sun(&self) -> Vec3 {
        sky::sun_direction(
            self.sun_elevation.to_degrees(),
            self.sun_azimuth.to_degrees(),
        )
    }
}
/// A moving vessel whose wake is drawn on the water (world metres, m/s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wake {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub length: f64,
    pub beam: f64,
}
pub const MAX_WAKES: usize = 8;
/// Shadow-map resolution (square, texels).
pub const SHADOW_SIZE: u32 = 2048;
/// Half the depth range (metres) of the shadow projection along the light.
const SHADOW_DEPTH: f64 = 1500.0;
/// Sunlight direction below the surface (Snell, n = 1.333), pointing toward
/// the light. Matches `sun_underwater` in the shader.
fn refracted_sun(sun: Vec3) -> Vec3 {
    let horizontal = (sun.x * sun.x + sun.y * sun.y).sqrt();
    let h = horizontal / 1.333;
    let scale = h / horizontal.max(0.001);
    Vec3::new(sun.x * scale, sun.y * scale, (1.0 - h * h).max(0.0).sqrt()).normalized()
}
/// Orthographic sun projection centred on the camera's focus, as three rows
/// (light clip x, y in -1..1 and depth in 0..1 = row . (p, 1)), and shadow
/// parameters (enabled, world size of one texel, map size, depth bias).
/// Shadows are cast along the refracted sun, which lights everything below
/// the surface; the centre is snapped to whole texels so the map does not
/// shimmer as the camera moves.
fn shadow_projection(sun: Vec3, eye: Vec3, target: Vec3) -> ([[f32; 4]; 4], [f32; 4]) {
    if sun.z <= 0.02 {
        return ([[0.; 4]; 4], [0.; 4]);
    }
    let toward = refracted_sun(sun);
    let travel = Vec3::new(-toward.x, -toward.y, -toward.z);
    let helper = if travel.z.abs() > 0.99 {
        Vec3::new(1.0, 0.0, 0.0)
    } else {
        Vec3::new(0.0, 0.0, 1.0)
    };
    let right = travel.cross(helper).normalized();
    let up = right.cross(travel).normalized();
    let half = ((target - eye).length() * 1.5 + 12.0).clamp(16.0, 400.0);
    let texel = 2.0 * half / SHADOW_SIZE as f64;
    let snap = |v: f64| (v / texel).round() * texel;
    let cx = snap(target.dot(right));
    let cy = snap(target.dot(up));
    let cz = target.dot(travel);
    let row = |axis: Vec3, scale: f64, offset: f64| {
        [
            (axis.x * scale) as f32,
            (axis.y * scale) as f32,
            (axis.z * scale) as f32,
            offset as f32,
        ]
    };
    let light = [
        row(right, 1.0 / half, -cx / half),
        row(up, 1.0 / half, -cy / half),
        row(travel, 0.5 / SHADOW_DEPTH, 0.5 - cz * 0.5 / SHADOW_DEPTH),
        [0., 0., 0., 1.],
    ];
    let bias = (0.04 + texel * 0.5) * 0.5 / SHADOW_DEPTH;
    (light, [1.0, texel as f32, SHADOW_SIZE as f32, bias as f32])
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    eye: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    sun: [f32; 4],
    environment: [f32; 4],
    screen: [f32; 4],
    light: [[f32; 4]; 4],
    shadow: [f32; 4],
    sky: [f32; 4],
    ocean: [f32; 4],
    foam: [f32; 4],
    foam_weights: [f32; 4],
    grid: [f32; 4],
    sun_light: [f32; 4],
    sky_light: [f32; 4],
}
/// Sun and sky light for a sun direction, cached because the sky
/// irradiance integral costs a few milliseconds on the CPU.
#[derive(Clone, Copy)]
struct Lighting {
    sun: Vec3,
    sun_light: [f64; 3],
    sky_light: [f64; 3],
}
impl Lighting {
    fn new(sun: Vec3) -> Self {
        Self {
            sun,
            sun_light: sky::sun_light(sun),
            sky_light: sky::sky_irradiance(sun),
        }
    }
    /// Exposure that partially adapts to the light level (like an eye or a
    /// camera on auto exposure): bright noon is about 1, dusk is lifted.
    fn exposure(&self) -> f64 {
        let lum = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let e = lum(self.sun_light) * self.sun.z.max(0.0) + lum(self.sky_light);
        (3.6 / (e + 0.01)).powf(0.6).clamp(0.6, 9.0)
    }
}
/// Fraction of the sea surface covered by whitecaps for a fully developed
/// sea of significant height `hs` (m): wind speed from the Pierson-Moskowitz
/// relation Hs = 0.21 U²/g, coverage from Monahan & O'Muircheartaigh (1980),
/// W = 3.84e-6 U^3.41. A visual estimate; swell under weak wind has less.
pub fn whitecap_coverage(hs: f64) -> f64 {
    if !hs.is_finite() || hs <= 0.0 {
        return 0.0;
    }
    whitecap_from_wind((hs * 9.81 / 0.21).sqrt())
}
/// Whitecap coverage for a known 10 m wind speed (m/s), Monahan &
/// O'Muircheartaigh (1980). Swell under a light wind stays clean.
pub fn whitecap_from_wind(wind: f64) -> f64 {
    spectrum::whitecap_fraction(wind)
}
/// Maximum wave components evaluated in the vertex shader.
pub const MAX_GPU_WAVES: usize = 64;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct WaveUniform {
    header: [f32; 4],
    components: [[f32; 4]; MAX_GPU_WAVES],
    wakes: [[f32; 4]; MAX_WAKES * 2],
}
impl WaveUniform {
    /// Per component: wave-number vector, amplitude and the phase at `time`,
    /// reduced in f64 so the shader's f32 arithmetic stays accurate for long
    /// simulations.
    fn new(sea: &SeaState, time: f64) -> Result<Self, String> {
        let list = sea.components();
        if list.len() > MAX_GPU_WAVES || !time.is_finite() {
            return Err("sea state exceeds GPU wave limits".into());
        }
        let mut u: Self = bytemuck::Zeroable::zeroed();
        u.header[0] = list.len() as f32;
        u.header[1] = sea.significant_height() as f32;
        u.header[2] = if sea.wind_speed > 0.0 {
            whitecap_from_wind(sea.wind_speed)
        } else {
            whitecap_coverage(sea.significant_height())
        } as f32;
        for (slot, c) in u.components.iter_mut().zip(list) {
            let phase = (c.phase - c.omega * time).rem_euclid(std::f64::consts::TAU);
            *slot = [
                (c.k * c.direction.cos()) as f32,
                (c.k * c.direction.sin()) as f32,
                c.amplitude as f32,
                phase as f32,
            ];
        }
        Ok(u)
    }
    fn set_wakes(&mut self, wakes: &[Wake]) {
        let mut count = 0;
        self.wakes = [[0.0; 4]; MAX_WAKES * 2];
        for w in wakes.iter().filter(|w| {
            [w.x, w.y, w.vx, w.vy, w.length, w.beam]
                .iter()
                .all(|v| v.is_finite())
        }) {
            if count == MAX_WAKES {
                break;
            }
            self.wakes[count * 2] = [w.x as f32, w.y as f32, w.vx as f32, w.vy as f32];
            self.wakes[count * 2 + 1] = [w.length as f32, w.beam as f32, 0.0, 0.0];
            count += 1;
        }
        self.header[3] = count as f32;
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex([f32; 14]);
impl From<&DrawVertex> for Vertex {
    fn from(v: &DrawVertex) -> Self {
        Self([
            v.position[0],
            v.position[1],
            v.position[2],
            v.normal[0],
            v.normal[1],
            v.normal[2],
            v.uv[0],
            v.uv[1],
            v.albedo[0],
            v.albedo[1],
            v.albedo[2],
            v.roughness,
            v.metallic,
            v.kind,
        ])
    }
}
struct Targets {
    opaque: wgpu::Texture,
    composite: wgpu::Texture,
    output: wgpu::Texture,
    depth: wgpu::Texture,
    bloom: wgpu::Texture,
    water_group: wgpu::BindGroup,
    tone_group: wgpu::BindGroup,
    bloom_group: wgpu::BindGroup,
    readback: wgpu::Buffer,
    row_bytes: u32,
    width: u32,
    height: u32,
}
/// Camera-projected ocean grid (index buffer only; positions come from
/// the vertex index).
struct Grid {
    indices: wgpu::Buffer,
    count: u32,
    columns: u32,
    rows: u32,
}
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    uniform: wgpu::Buffer,
    waves: wgpu::Buffer,
    wave_data: WaveUniform,
    camera_group: wgpu::BindGroup,
    image_layout: wgpu::BindGroupLayout,
    env_group: wgpu::BindGroup,
    dummy: wgpu::TextureView,
    sky: wgpu::RenderPipeline,
    sky_table: wgpu::RenderPipeline,
    sky_lut: wgpu::TextureView,
    opaque: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    ocean_surface: wgpu::RenderPipeline,
    bloom: wgpu::RenderPipeline,
    tone: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    shadow_map: wgpu::TextureView,
    shadow_group: wgpu::BindGroup,
    targets: Targets,
    vertices: wgpu::Buffer,
    capacity: u64,
    staging: Vec<Vertex>,
    opaque_count: u32,
    /// Cached time-independent opaque geometry (seabed, structures).
    static_vertices: wgpu::Buffer,
    static_capacity: u64,
    static_count: u32,
    /// Cached flat water (`RenderScene::flat_water`), displaced on the GPU.
    static_water: wgpu::Buffer,
    static_water_capacity: u64,
    static_water_count: u32,
    ocean: ocean_gpu::OceanGpu,
    /// Projected FFT ocean surface enabled (`set_ocean`) and its time.
    ocean_on: bool,
    ocean_time: f64,
    grid_step: u32,
    grid: Option<Grid>,
    lighting: Option<Lighting>,
    in_flight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    /// Direction the wind sea travels towards (radians), for foam streaks.
    wind_direction: f64,
    /// Instanced animals and their shadow casters.
    creature: wgpu::RenderPipeline,
    creature_shadow: wgpu::RenderPipeline,
    /// Streamed distant terrain (`upload_world`), drawn without shadows.
    world_vertices: wgpu::Buffer,
    world_capacity: u64,
    world_count: u32,
    /// Species meshes (`upload_animal_meshes`): vertex range per mesh.
    animal_meshes: wgpu::Buffer,
    animal_mesh_capacity: u64,
    animal_ranges: Vec<(u32, u32)>,
    /// Animals this frame (`set_animals`): (mesh, first instance, count).
    animal_instances: wgpu::Buffer,
    animal_instance_capacity: u64,
    animal_draws: Vec<(usize, u32, u32)>,
    /// Meteorological visibility (m) for aerial perspective.
    visibility: f64,
    pub adapter: wgpu::AdapterInfo,
}

/// One animal to draw with a species mesh (`GpuRenderer::set_animals`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Animal {
    /// Index into the meshes given to `upload_animal_meshes`.
    pub mesh: usize,
    pub position: Vec3,
    /// Unit heading (the snout direction).
    pub heading: Vec3,
    /// Body length (m).
    pub length: f64,
    /// Swimming: locomotion mode (0 body and tail, 1 whole body, 2 wings,
    /// 3 vertical flukes, 4 flippers), amplitude in body lengths, angular
    /// frequency (rad/s), body-wave number, and phase.
    pub mode: u8,
    pub amplitude: f64,
    pub frequency: f64,
    pub wavenumber: f64,
    pub phase: f64,
    /// Colour multiplier (individual variation) and body bend when turning.
    pub tint: [f32; 3],
    pub bend: f64,
}
const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SKY_TABLE: (u32, u32) = (256, 128);
fn texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}
fn targets(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    dummy: &wgpu::TextureView,
    width: u32,
    height: u32,
) -> Targets {
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_SRC
        | wgpu::TextureUsages::COPY_DST;
    let opaque = texture(device, width, height, HDR, usage, "opaque HDR");
    let composite = texture(device, width, height, HDR, usage, "water HDR");
    let bloom = texture(
        device,
        width.div_ceil(4),
        height.div_ceil(4),
        HDR,
        usage,
        "bloom",
    );
    let output = texture(
        device,
        width,
        height,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        "display output",
    );
    let depth = texture(
        device,
        width,
        height,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        "scene depth",
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("linear clamp"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let depth_view = depth.create_view(&Default::default());
    let group = |color: &wgpu::Texture, extra: &wgpu::TextureView| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene inputs"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &color.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(extra),
                },
            ],
        })
    };
    let water_group = group(&opaque, dummy);
    let tone_group = group(&composite, &bloom.create_view(&Default::default()));
    let bloom_group = group(&composite, dummy);
    let row_bytes = (width * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bounded preview readback"),
        size: row_bytes as u64 * height as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    Targets {
        opaque,
        composite,
        output,
        depth,
        bloom,
        water_group,
        tone_group,
        bloom_group,
        readback,
        row_bytes,
        width,
        height,
    }
}
fn texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    dimension: wgpu::TextureViewDimension,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}
impl GpuRenderer {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        pollster::block_on(Self::initialize(width, height))
    }
    async fn initialize(width: u32, height: u32) -> Result<Self, String> {
        dimensions(width, height)?;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("No native GPU adapter: {e}"))?;
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            return Err("Only a software GPU adapter was found; select the CPU renderer.".into());
        }
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Pelagic native GPU"),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        Self::build(device, queue, info, width, height).await
    }
    /// Renderer sharing an existing device (for example the editor window device),
    /// so frames can be displayed directly without CPU readback.
    pub fn with_device(
        device: wgpu::Device,
        queue: wgpu::Queue,
        adapter: wgpu::AdapterInfo,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        dimensions(width, height)?;
        pollster::block_on(Self::build(device, queue, adapter, width, height))
    }
    async fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        info: wgpu::AdapterInfo,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Pelagic ocean preview"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ocean.wgsl").into()),
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera state"),
            contents: bytemuck::bytes_of(&Uniform::zero()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let wave_data: WaveUniform = bytemuck::Zeroable::zeroed();
        let waves = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("wave components"),
            contents: bytemuck::bytes_of(&wave_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera layout"),
            entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: both,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }),
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: waves.as_entire_binding(),
                },
            ],
        });
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let d2 = wgpu::TextureViewDimension::D2;
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("color and depth input"),
            entries: &[
                texture_entry(0, fragment, d2, float),
                sampler_entry(1, fragment),
                texture_entry(2, fragment, d2, wgpu::TextureSampleType::Depth),
                texture_entry(3, fragment, d2, float),
            ],
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow map input"),
            entries: &[
                texture_entry(0, fragment, d2, wgpu::TextureSampleType::Depth),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let array = wgpu::TextureViewDimension::D2Array;
        let env_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment"),
            entries: &[
                texture_entry(0, both, array, float),
                texture_entry(1, both, array, float),
                texture_entry(2, both, array, float),
                sampler_entry(3, both),
                texture_entry(4, both, d2, float),
                sampler_entry(5, both),
            ],
        });
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun shadow map"),
            size: wgpu::Extent3d {
                width: SHADOW_SIZE,
                height: SHADOW_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_map = shadow_texture.create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow comparison"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow map"),
            layout: &shadow_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_map),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let ocean = ocean_gpu::OceanGpu::new(&device);
        let sky_lut = texture(
            &device,
            SKY_TABLE.0,
            SKY_TABLE.1,
            HDR,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            "sky view table",
        )
        .create_view(&Default::default());
        let ocean_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ocean repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let lut_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky table"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let env_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("environment"),
            layout: &env_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&ocean.disp),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&ocean.slope),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&ocean.moment),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&ocean_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&sky_lut),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&lut_sampler),
                },
            ],
        });
        let dummy = texture(
            &device,
            1,
            1,
            HDR,
            wgpu::TextureUsages::TEXTURE_BINDING,
            "unused input",
        )
        .create_view(&Default::default());
        let layout = |groups: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("pipeline layout"),
                bind_group_layouts: groups,
                immediate_size: 0,
            })
        };
        let opaque_layout = layout(&[
            Some(&camera_layout),
            None,
            Some(&shadow_layout),
            Some(&env_layout),
        ]);
        let water_layout = layout(&[
            Some(&camera_layout),
            Some(&image_layout),
            Some(&shadow_layout),
            Some(&env_layout),
        ]);
        let post_layout = layout(&[Some(&camera_layout), Some(&image_layout)]);
        let camera_only = layout(&[Some(&camera_layout)]);
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x3,2=>Float32x2,3=>Float32x3,4=>Float32,5=>Float32,6=>Float32];
        let buffer = wgpu::VertexBufferLayout {
            array_stride: 56,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let buffers = [Some(buffer)];
        let no_buffers: [Option<wgpu::VertexBufferLayout>; 0] = [];
        let pipeline = |vertex: &str,
                        entry: &str,
                        depth: Option<bool>,
                        format: wgpu::TextureFormat,
                        layout: &wgpu::PipelineLayout| {
            let geometry = vertex == "geometry";
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: Default::default(),
                    buffers: if geometry { &buffers } else { &no_buffers },
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: depth.map(|write| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(write),
                    // Reversed depth (near = 1, infinitely far = 0).
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sky = pipeline("fullscreen", "sky", Some(false), HDR, &opaque_layout);
        let sky_table = pipeline("fullscreen", "sky_table", None, HDR, &camera_only);
        let opaque = pipeline("geometry", "opaque", Some(true), HDR, &opaque_layout);
        let water = pipeline("geometry", "water", Some(false), HDR, &water_layout);
        let ocean_surface = pipeline("ocean_grid", "ocean_water", Some(false), HDR, &water_layout);
        let bloom = pipeline("fullscreen", "bloom_extract", None, HDR, &post_layout);
        let tone = pipeline(
            "fullscreen",
            "tone",
            None,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &post_layout,
        );
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow caster"),
            layout: Some(&camera_only),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("shadow_vertex"),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.5,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        // Animals: species meshes drawn once per animal (instanced).
        let animal_attributes =
            wgpu::vertex_attr_array![7=>Float32x4,8=>Float32x4,9=>Float32x4,10=>Float32x4];
        let animal_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: 56,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: 64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &animal_attributes,
            }),
        ];
        let creature = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("animals"),
            layout: Some(&opaque_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("creature"),
                compilation_options: Default::default(),
                buffers: &animal_buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("opaque"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let creature_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("animal shadows"),
            layout: Some(&camera_only),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("creature_shadow"),
                compilation_options: Default::default(),
                buffers: &animal_buffers,
            },
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.5,
                    clamp: 0.0,
                },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let targets = targets(&device, &image_layout, &dummy, width, height);
        let vertices = device_buffer(&device, 56, "shared scene vertices");
        if let Some(error) = errors.pop().await {
            return Err(format!("GPU initialization: {error}"));
        }
        let static_vertices = device_buffer(&device, 56, "static scene vertices");
        let static_water = device_buffer(&device, 56, "static water vertices");
        let world_vertices = device_buffer(&device, 56, "streamed terrain vertices");
        let animal_meshes = device_buffer(&device, 56, "animal meshes");
        let animal_instances = device_buffer(&device, 64, "animal instances");
        Ok(Self {
            creature,
            creature_shadow,
            world_vertices,
            world_capacity: 56,
            world_count: 0,
            animal_meshes,
            animal_mesh_capacity: 56,
            animal_ranges: vec![],
            animal_instances,
            animal_instance_capacity: 64,
            animal_draws: vec![],
            visibility: 40_000.0,
            device,
            queue,
            uniform,
            waves,
            wave_data,
            camera_group,
            image_layout,
            env_group,
            dummy,
            sky,
            sky_table,
            sky_lut,
            opaque,
            water,
            ocean_surface,
            bloom,
            tone,
            shadow,
            shadow_map,
            shadow_group,
            targets,
            vertices,
            capacity: 56,
            staging: vec![],
            opaque_count: 0,
            static_vertices,
            static_capacity: 56,
            static_count: 0,
            static_water,
            static_water_capacity: 56,
            static_water_count: 0,
            ocean,
            ocean_on: false,
            ocean_time: 0.0,
            grid_step: 6,
            grid: None,
            lighting: None,
            in_flight: Default::default(),
            wind_direction: 0.0,
            adapter: info,
        })
    }
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        dimensions(width, height)?;
        if (width, height) != (self.targets.width, self.targets.height) {
            self.targets = targets(&self.device, &self.image_layout, &self.dummy, width, height);
        }
        Ok(())
    }
    /// Uploads the complete visual geometry and clears the static layer.
    pub fn upload(&mut self, scene: &RenderScene) -> Result<(), String> {
        self.static_count = 0;
        self.static_water_count = 0;
        self.upload_dynamic(scene)
    }
    /// Uploads time-independent opaque geometry (`RenderScene::build_static`).
    /// It is drawn before the dynamic layer until replaced.
    pub fn upload_static(&mut self, scene: &RenderScene) -> Result<(), String> {
        if scene.opaque.len() > 4_000_000 || !scene.opaque.len().is_multiple_of(3) {
            return Err("invalid or oversized static geometry".into());
        }
        let staged: Vec<Vertex> = scene.opaque.iter().map(Vertex::from).collect();
        let data: &[u8] = bytemuck::cast_slice(&staged);
        if data.len() as u64 > self.static_capacity {
            self.static_capacity = (data.len() as u64).next_power_of_two();
            self.static_vertices =
                device_buffer(&self.device, self.static_capacity, "static scene vertices");
        }
        if !data.is_empty() {
            self.queue.write_buffer(&self.static_vertices, 0, data);
        }
        self.static_count = scene.opaque.len() as u32;
        Ok(())
    }
    /// Uploads a flat water mesh (`RenderScene::flat_water`) whose waves are
    /// evaluated in the vertex shader from `set_waves`. Drawn with the
    /// dynamic water until replaced; pass an empty slice to remove it.
    pub fn upload_water(&mut self, water: &[DrawVertex]) -> Result<(), String> {
        if water.len() > 2_000_000 || !water.len().is_multiple_of(3) {
            return Err("invalid or oversized water geometry".into());
        }
        let staged: Vec<Vertex> = water.iter().map(Vertex::from).collect();
        let data: &[u8] = bytemuck::cast_slice(&staged);
        if data.len() as u64 > self.static_water_capacity {
            self.static_water_capacity = (data.len() as u64).next_power_of_two();
            self.static_water = device_buffer(
                &self.device,
                self.static_water_capacity,
                "static water vertices",
            );
        }
        if !data.is_empty() {
            self.queue.write_buffer(&self.static_water, 0, data);
        }
        self.static_water_count = water.len() as u32;
        Ok(())
    }
    /// Uploads streamed distant terrain (coasts, islands and seabed beyond
    /// the scene). Drawn with the scene but casts no shadows; an empty
    /// slice removes it.
    pub fn upload_world(&mut self, world: &[DrawVertex]) -> Result<(), String> {
        if world.len() > 6_000_000 || !world.len().is_multiple_of(3) {
            return Err("invalid or oversized terrain geometry".into());
        }
        let staged: Vec<Vertex> = world.iter().map(Vertex::from).collect();
        let data: &[u8] = bytemuck::cast_slice(&staged);
        if data.len() as u64 > self.world_capacity {
            self.world_capacity = (data.len() as u64).next_power_of_two();
            self.world_vertices = device_buffer(
                &self.device,
                self.world_capacity,
                "streamed terrain vertices",
            );
        }
        if !data.is_empty() {
            self.queue.write_buffer(&self.world_vertices, 0, data);
        }
        self.world_count = world.len() as u32;
        Ok(())
    }
    /// Uploads one model-space mesh per species (snout at +x 0.5, length 1,
    /// +z up). Replaces earlier meshes and clears the animals.
    pub fn upload_animal_meshes(&mut self, meshes: &[Vec<DrawVertex>]) -> Result<(), String> {
        let total: usize = meshes.iter().map(Vec::len).sum();
        if total > 2_000_000 || meshes.iter().any(|m| !m.len().is_multiple_of(3)) {
            return Err("invalid or oversized animal meshes".into());
        }
        let staged: Vec<Vertex> = meshes.iter().flatten().map(Vertex::from).collect();
        let data: &[u8] = bytemuck::cast_slice(&staged);
        if data.len() as u64 > self.animal_mesh_capacity {
            self.animal_mesh_capacity = (data.len() as u64).next_power_of_two();
            self.animal_meshes =
                device_buffer(&self.device, self.animal_mesh_capacity, "animal meshes");
        }
        if !data.is_empty() {
            self.queue.write_buffer(&self.animal_meshes, 0, data);
        }
        let mut first = 0u32;
        self.animal_ranges = meshes
            .iter()
            .map(|m| {
                let r = (first, m.len() as u32);
                first += m.len() as u32;
                r
            })
            .collect();
        self.animal_draws.clear();
        Ok(())
    }
    /// The animals to draw this frame (meshes from `upload_animal_meshes`).
    pub fn set_animals(&mut self, animals: &[Animal]) {
        let mut sorted: Vec<&Animal> = animals
            .iter()
            .filter(|a| {
                a.mesh < self.animal_ranges.len()
                    && a.position.is_finite()
                    && a.heading.is_finite()
                    && a.length.is_finite()
                    && a.length > 0.0
            })
            .take(20_000)
            .collect();
        sorted.sort_by_key(|a| a.mesh);
        let mut data: Vec<[f32; 16]> = Vec::with_capacity(sorted.len());
        self.animal_draws.clear();
        for a in &sorted {
            match self.animal_draws.last_mut() {
                Some(d) if d.0 == a.mesh => d.2 += 1,
                _ => self.animal_draws.push((a.mesh, data.len() as u32, 1)),
            }
            data.push([
                a.position.x as f32,
                a.position.y as f32,
                a.position.z as f32,
                a.length as f32,
                a.heading.x as f32,
                a.heading.y as f32,
                a.heading.z as f32,
                a.phase as f32,
                a.mode as f32,
                a.amplitude as f32,
                a.frequency as f32,
                a.wavenumber as f32,
                a.tint[0],
                a.tint[1],
                a.tint[2],
                a.bend as f32,
            ]);
        }
        let bytes: &[u8] = bytemuck::cast_slice(&data);
        if bytes.len() as u64 > self.animal_instance_capacity {
            self.animal_instance_capacity = (bytes.len() as u64).next_power_of_two();
            self.animal_instances = device_buffer(
                &self.device,
                self.animal_instance_capacity,
                "animal instances",
            );
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.animal_instances, 0, bytes);
        }
    }
    /// Meteorological visibility (m) used for haze over distance (default
    /// 40 km).
    pub fn set_visibility(&mut self, metres: f64) {
        if metres.is_finite() {
            self.visibility = metres.clamp(200.0, 200_000.0);
        }
    }
    /// Sets the sea state and time used to displace `upload_water` geometry.
    pub fn set_waves(&mut self, sea: &SeaState, time: f64) -> Result<(), String> {
        let wakes = self.wave_data.wakes;
        let count = self.wave_data.header[3];
        self.wave_data = WaveUniform::new(sea, time)?;
        self.wave_data.wakes = wakes;
        self.wave_data.header[3] = count;
        self.queue
            .write_buffer(&self.waves, 0, bytemuck::bytes_of(&self.wave_data));
        Ok(())
    }
    /// Draws the open sea as a camera-projected FFT ocean: the physics
    /// components of `sea` exactly, plus the short-wave tail of `detail`,
    /// whitecaps and wakes. `quality` 0-2 sets the grid density.
    pub fn set_ocean(
        &mut self,
        sea: &SeaState,
        detail: &DetailSea,
        time: f64,
        quality: u8,
    ) -> Result<(), String> {
        if !time.is_finite()
            || ![
                detail.hs,
                detail.tp,
                detail.direction,
                detail.spread,
                detail.depth,
                detail.wind,
            ]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err("invalid ocean settings".into());
        }
        self.set_waves(sea, time)?;
        self.ocean.set_sea(&self.queue, detail, sea);
        self.ocean_on = true;
        self.ocean_time = time;
        self.wind_direction = detail.direction;
        self.grid_step = match quality {
            0 => 9,
            1 => 6,
            2 => 4,
            _ => 3,
        };
        Ok(())
    }
    /// Stops drawing the projected ocean (legacy water meshes still draw).
    pub fn clear_ocean(&mut self) {
        self.ocean_on = false;
    }
    /// Vessels whose wakes are drawn (at most `MAX_WAKES`).
    pub fn set_wakes(&mut self, wakes: &[Wake]) {
        self.wave_data.set_wakes(wakes);
        self.queue
            .write_buffer(&self.waves, 0, bytemuck::bytes_of(&self.wave_data));
    }
    /// Uploads time-dependent geometry. Call only when the simulation state
    /// changed; camera-only frames reuse the uploaded buffers.
    pub fn upload_dynamic(&mut self, scene: &RenderScene) -> Result<(), String> {
        if scene.opaque.len() + scene.water.len() > 2_000_000
            || !scene.opaque.len().is_multiple_of(3)
            || !scene.water.len().is_multiple_of(3)
        {
            return Err("invalid or oversized visual geometry".into());
        }
        self.staging.clear();
        self.staging
            .extend(scene.opaque.iter().chain(&scene.water).map(Vertex::from));
        self.opaque_count = scene.opaque.len() as u32;
        let data = bytemuck::cast_slice(&self.staging);
        if data.len() as u64 > self.capacity {
            self.capacity = (data.len() as u64).next_power_of_two();
            self.vertices = device_buffer(&self.device, self.capacity, "shared scene vertices");
        }
        if !data.is_empty() {
            self.queue.write_buffer(&self.vertices, 0, data);
        }
        Ok(())
    }
    fn uniform(&mut self, view: &View) -> Result<Uniform, String> {
        view.validate()?;
        let (width, height) = (self.targets.width, self.targets.height);
        let forward = (view.target - view.eye).normalized();
        let right = forward.cross(view.up).normalized();
        if forward.length() < 0.9 || right.length() < 0.9 {
            return Err("degenerate camera basis".into());
        }
        let up = right.cross(forward);
        let sun = view.sun();
        let lighting = match self.lighting {
            Some(l) if (l.sun - sun).length() < 1e-4 => l,
            _ => {
                let l = Lighting::new(sun);
                self.lighting = Some(l);
                l
            }
        };
        let pack = |p: Vec3| [p.x as f32, p.y as f32, p.z as f32, 0.];
        let pack3 = |c: [f64; 3]| [c[0] as f32, c[1] as f32, c[2] as f32, 0.];
        let (light, shadow) = shadow_projection(sun, view.eye, view.target);
        let (columns, rows) = self.grid.as_ref().map_or((0, 0), |g| (g.columns, g.rows));
        let (threshold, ramp) = self.ocean.foam_threshold();
        let weights = self.ocean.foam_weights();
        let detail_on = self.ocean_on && self.ocean.active();
        Ok(Uniform {
            eye: pack(view.eye),
            forward: pack(forward),
            right: pack(right),
            up: pack(up),
            sun: pack(sun),
            environment: [
                view.time as f32,
                view.attenuation as f32,
                if view.underwater { 1. } else { 0. },
                lighting.exposure() as f32,
            ],
            screen: [
                width as f32,
                height as f32,
                (view.fov_y.to_radians() * 0.5).tan() as f32,
                width as f32 / height as f32,
            ],
            light,
            shadow,
            sky: [
                view.clouds.clamp(0.0, 1.0) as f32,
                self.visibility as f32,
                self.wind_direction.cos() as f32,
                self.wind_direction.sin() as f32,
            ],
            ocean: [
                spectrum::SIZES[0] as f32,
                spectrum::SIZES[1] as f32,
                spectrum::SIZES[2] as f32,
                spectrum::CHOP as f32,
            ],
            foam: [
                threshold as f32,
                ramp as f32,
                self.ocean.stats.unresolved_mss as f32,
                if detail_on && self.ocean.stats.whitecaps > 0.0 {
                    1.0
                } else {
                    0.0
                },
            ],
            foam_weights: [
                weights[0] as f32,
                weights[1] as f32,
                weights[2] as f32,
                spectrum::ANALYTIC_CHOP as f32,
            ],
            grid: [
                columns as f32,
                rows as f32,
                // Underwater light-shaft march steps by quality.
                match self.grid_step {
                    9 => 4.0,
                    6 => 6.0,
                    4 => 8.0,
                    _ => 10.0,
                },
                if detail_on { 1.0 } else { 0.0 },
            ],
            sun_light: pack3(lighting.sun_light),
            sky_light: pack3(lighting.sky_light),
        })
    }
    fn ensure_grid(&mut self) {
        let (w, h) = (self.targets.width, self.targets.height);
        let columns = (w / self.grid_step).clamp(16, 1024) + 1;
        let rows = (h / self.grid_step).clamp(16, 1024) + 1;
        if self
            .grid
            .as_ref()
            .is_some_and(|g| g.columns == columns && g.rows == rows)
        {
            return;
        }
        let mut indices = Vec::with_capacity(((columns - 1) * (rows - 1) * 6) as usize);
        for y in 0..rows - 1 {
            for x in 0..columns - 1 {
                let a = y * columns + x;
                indices.extend_from_slice(&[
                    a,
                    a + 1,
                    a + columns,
                    a + 1,
                    a + columns + 1,
                    a + columns,
                ]);
            }
        }
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ocean grid indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.grid = Some(Grid {
            indices: buffer,
            count: indices.len() as u32,
            columns,
            rows,
        });
    }
    /// Draws the uploaded geometry into the display texture (`output_view`)
    /// and submits, without waiting for the GPU or reading back.
    pub fn draw(&mut self, view: View) -> Result<(), String> {
        let encoder = self.prepare(&view)?;
        let index = self.queue.submit(Some(encoder.finish()));
        // At most two frames in flight: without a presenting swapchain (offscreen
        // captures) the CPU could otherwise queue work faster than the GPU
        // finishes it until memory runs out.
        self.in_flight.push_back(index);
        while self.in_flight.len() > 2 {
            if let Some(old) = self.in_flight.pop_front() {
                let _ = self.device.poll(wgpu::PollType::Wait {
                    submission_index: Some(old),
                    timeout: Some(std::time::Duration::from_secs(5)),
                });
            }
        }
        Ok(())
    }
    fn prepare(&mut self, view: &View) -> Result<wgpu::CommandEncoder, String> {
        if self.ocean_on {
            self.ensure_grid();
        }
        let u = self.uniform(view)?;
        self.queue
            .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ocean frame"),
            });
        if self.ocean_on {
            self.ocean
                .encode(&self.queue, &mut encoder, self.ocean_time);
        }
        self.encode(&mut encoder, self.opaque_count, self.staging.len() as u32);
        Ok(encoder)
    }
    /// sRGB display texture written by `draw`; recreated by `resize`.
    pub fn output_view(&self) -> wgpu::TextureView {
        self.targets.output.create_view(&Default::default())
    }
    pub fn size(&self) -> (u32, u32) {
        (self.targets.width, self.targets.height)
    }
    fn draw_animals(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.animal_meshes.slice(..));
        pass.set_vertex_buffer(1, self.animal_instances.slice(..));
        for &(mesh, first, count) in &self.animal_draws {
            let (start, len) = self.animal_ranges[mesh];
            pass.draw(start..start + len, first..first + count);
        }
    }
    fn encode(&self, encoder: &mut wgpu::CommandEncoder, opaque: u32, total: u32) {
        let t = &self.targets;
        let extent = wgpu::Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        };
        let opaque_view = t.opaque.create_view(&Default::default());
        let composite_view = t.composite.create_view(&Default::default());
        let bloom_view = t.bloom.create_view(&Default::default());
        let depth = t.depth.create_view(&Default::default());
        let output = t.output.create_view(&Default::default());
        {
            // Sky-view table for this sun (cheap; recomputed every frame).
            let attachments = [Some(attachment(
                &self.sky_lut,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            ))];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky table"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_bind_group(0, &self.camera_group, &[]);
            pass.set_pipeline(&self.sky_table);
            pass.draw(0..3, 0..1);
        }
        {
            // Sun shadow map from every opaque caster (water casts none).
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sun shadow map"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.camera_group, &[]);
            pass.set_pipeline(&self.shadow);
            if self.static_count > 0 {
                pass.set_vertex_buffer(0, self.static_vertices.slice(..));
                pass.draw(0..self.static_count, 0..1);
            }
            if opaque > 0 {
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.draw(0..opaque, 0..1);
            }
            if !self.animal_draws.is_empty() {
                pass.set_pipeline(&self.creature_shadow);
                self.draw_animals(&mut pass);
            }
        }
        {
            let attachments = [Some(attachment(
                &opaque_view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            ))];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky and opaque"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.camera_group, &[]);
            pass.set_bind_group(2, &self.shadow_group, &[]);
            pass.set_bind_group(3, &self.env_group, &[]);
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);
            if self.static_count > 0 {
                pass.set_pipeline(&self.opaque);
                pass.set_vertex_buffer(0, self.static_vertices.slice(..));
                pass.draw(0..self.static_count, 0..1);
            }
            if opaque > 0 {
                pass.set_pipeline(&self.opaque);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.draw(0..opaque, 0..1);
            }
            if self.world_count > 0 {
                pass.set_pipeline(&self.opaque);
                pass.set_vertex_buffer(0, self.world_vertices.slice(..));
                pass.draw(0..self.world_count, 0..1);
            }
            if !self.animal_draws.is_empty() {
                pass.set_pipeline(&self.creature);
                self.draw_animals(&mut pass);
            }
        }
        encoder.copy_texture_to_texture(
            t.opaque.as_image_copy(),
            t.composite.as_image_copy(),
            extent,
        );
        let ocean = self.ocean_on.then_some(self.grid.as_ref()).flatten();
        if total > opaque || self.static_water_count > 0 || ocean.is_some() {
            let attachments = [Some(attachment(&composite_view, wgpu::LoadOp::Load))];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("water transmission"),
                color_attachments: &attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.camera_group, &[]);
            pass.set_bind_group(1, &t.water_group, &[]);
            pass.set_bind_group(2, &self.shadow_group, &[]);
            pass.set_bind_group(3, &self.env_group, &[]);
            if let Some(grid) = ocean {
                pass.set_pipeline(&self.ocean_surface);
                pass.set_index_buffer(grid.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..grid.count, 0, 0..1);
            }
            pass.set_pipeline(&self.water);
            if self.static_water_count > 0 {
                pass.set_vertex_buffer(0, self.static_water.slice(..));
                pass.draw(0..self.static_water_count, 0..1);
            }
            if total > opaque {
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.draw(opaque..total, 0..1);
            }
        }
        {
            let attachments = [Some(attachment(
                &bloom_view,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            ))];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bloom"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_bind_group(0, &self.camera_group, &[]);
            pass.set_bind_group(1, &t.bloom_group, &[]);
            pass.set_pipeline(&self.bloom);
            pass.draw(0..3, 0..1);
        }
        {
            let attachments = [Some(attachment(
                &output,
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            ))];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tone mapping"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_bind_group(0, &self.camera_group, &[]);
            pass.set_bind_group(1, &t.tone_group, &[]);
            pass.set_pipeline(&self.tone);
            pass.draw(0..3, 0..1);
        }
    }
    /// Offscreen preview with synchronized readback (CPU-presented UI, tests
    /// and captures). Timings include upload, GPU work and readback.
    pub fn render(&mut self, scene: &RenderScene, view: View) -> Result<Frame, String> {
        self.upload(scene)?;
        self.frame(view)
    }
    /// Draws what is uploaded (static layer, dynamic layer, ocean) and reads
    /// the display image back.
    pub fn frame(&mut self, view: View) -> Result<Frame, String> {
        let error_scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = self.prepare(&view)?;
        let t = &self.targets;
        let extent = wgpu::Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_buffer(
            t.output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &t.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(t.row_bytes),
                    rows_per_image: Some(t.height),
                },
            },
            extent,
        );
        self.queue.submit(Some(encoder.finish()));
        let (send, receive) = std::sync::mpsc::channel();
        t.readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .map_err(|e| e.to_string())?;
        receive
            .recv_timeout(std::time::Duration::from_secs(1))
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let bytes = t
            .readback
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let mut rgb = Vec::with_capacity(t.width as usize * t.height as usize * 3);
        for row in bytes.chunks_exact(t.row_bytes as usize) {
            for pixel in row[..t.width as usize * 4].as_chunks::<4>().0 {
                rgb.extend_from_slice(&pixel[..3]);
            }
        }
        drop(bytes);
        t.readback.unmap();
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(format!("GPU frame: {error}"));
        }
        Ok(Frame {
            width: t.width as usize,
            height: t.height as usize,
            rgb,
        })
    }
}
impl Uniform {
    fn zero() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}
fn device_buffer(device: &wgpu::Device, size: u64, label: &'static str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
fn attachment(
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    }
}
fn dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || width as u64 * height as u64 > 16_777_216
    {
        Err("GPU preview dimensions exceed the 16-megapixel budget".into())
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// The vertex shader's sum, in f32, over the packed components.
    fn shader_height(u: &WaveUniform, x: f32, y: f32) -> (f32, [f32; 2]) {
        let mut h = 0.;
        let mut slope = [0f32; 2];
        for c in &u.components[..u.header[0] as usize] {
            let phase = c[0] * x + c[1] * y + c[3];
            h += c[2] * phase.cos();
            slope[0] += c[2] * c[0] * phase.sin();
            slope[1] += c[2] * c[1] * phase.sin();
        }
        (h, slope)
    }
    #[test]
    fn packed_waves_reproduce_the_cpu_model_late_in_a_run() {
        let sea = SeaState::spectrum(2.5, 9.0, 0.6, 0.5, 60.0, 7).unwrap();
        let hs = sea.significant_height();
        for time in [0.0, 37.25, 86_400.5] {
            let u = WaveUniform::new(&sea, time).unwrap();
            for (x, y) in [(0.0, 0.0), (-511.0, 300.0), (480.0, -512.0)] {
                let cpu = sea.sample(x, y, time);
                let (h, slope) = shader_height(&u, x as f32, y as f32);
                assert!((cpu.height - h as f64).abs() < 0.01 * hs, "{time} {x} {y}");
                let n = Vec3::new(slope[0] as f64, slope[1] as f64, 1.0).normalized();
                assert!((n - cpu.normal).length() < 0.01, "{time} {x} {y}");
            }
        }
        assert!(WaveUniform::new(&sea, f64::NAN).is_err());
    }
}
#[cfg(test)]
mod shadow_tests {
    use super::*;
    fn quad(out: &mut Vec<DrawVertex>, c: [Vec3; 4], kind: f32) {
        let n = (c[1] - c[0]).cross(c[2] - c[0]).normalized();
        for i in [0, 1, 2, 0, 2, 3] {
            let p = c[i];
            out.push(DrawVertex {
                position: [p.x as f32, p.y as f32, p.z as f32],
                normal: [n.x as f32, n.y as f32, n.z as f32],
                uv: [p.x as f32, p.y as f32],
                albedo: [0.6, 0.6, 0.55],
                roughness: 0.9,
                metallic: 0.,
                kind,
            });
        }
    }
    fn centre_luma(frame: &Frame) -> f64 {
        let (w, h) = (frame.width, frame.height);
        let mut sum = 0.;
        for y in h / 2 - 4..h / 2 + 4 {
            for x in w / 2 - 4..w / 2 + 4 {
                let i = (y * w + x) * 3;
                sum += frame.rgb[i + 1] as f64 + frame.rgb[i + 2] as f64;
            }
        }
        sum
    }
    /// A plate kept outside the view, between the sun and the floor, must
    /// darken the floor at the centre of the image only through its shadow.
    #[test]
    fn hidden_caster_shades_the_floor_along_the_refracted_sun() {
        let Some(mut gpu) = test_gpu(320, 200) else {
            eprintln!("no GPU adapter; skipped");
            return;
        };
        let floor = -20.0;
        let mut scene = RenderScene::default();
        let s = 200.0;
        quad(
            &mut scene.opaque,
            [
                Vec3::new(-s, -s, floor),
                Vec3::new(s, -s, floor),
                Vec3::new(s, s, floor),
                Vec3::new(-s, s, floor),
            ],
            2.,
        );
        let view = View {
            eye: Vec3::new(0., 0.01, -6.),
            target: Vec3::new(0., 0., floor),
            up: Vec3::new(0., 1., 0.),
            fov_y: 40.,
            sun_elevation: 60f64.to_radians(),
            sun_azimuth: LEGACY_SUN_AZIMUTH,
            clouds: 0.0,
            time: 0.,
            attenuation: 0.035,
            underwater: true,
        };
        let lit = centre_luma(&gpu.render(&scene, view).unwrap());
        // Plate centred on the refracted sun ray through the floor centre,
        // 1 m below the surface: above and behind the camera.
        let sun = view.sun();
        let toward = refracted_sun(sun);
        let c = Vec3::new(0., 0., floor) + toward * ((-1.0 - floor) / toward.z);
        let h = 3.0;
        quad(
            &mut scene.opaque,
            [
                c + Vec3::new(-h, -h, 0.),
                c + Vec3::new(h, -h, 0.),
                c + Vec3::new(h, h, 0.),
                c + Vec3::new(-h, h, 0.),
            ],
            2.,
        );
        let shaded = centre_luma(&gpu.render(&scene, view).unwrap());
        // Most of the remaining light is in-scattered haze over the 14 m path.
        assert!(shaded < lit * 0.9, "lit {lit} shaded {shaded}");
        // Without sun (below the horizon) nothing is shadowed.
        let night = View {
            sun_elevation: -0.2,
            ..view
        };
        let open = centre_luma(&gpu.render(&scene, night).unwrap());
        scene.opaque.truncate(6);
        let empty = centre_luma(&gpu.render(&scene, night).unwrap());
        assert!((open - empty).abs() <= empty * 0.02, "{open} {empty}");
    }
}
#[cfg(test)]
mod whitecap_tests {
    #[test]
    fn whitecap_coverage_grows_with_sea_state_within_observed_range() {
        let c = |hs| super::whitecap_coverage(hs);
        assert_eq!(c(0.0), 0.0);
        // Moderate sea: under 1 %; rough sea (Hs 4 m, ~14 m/s wind): a few %.
        assert!(c(1.8) > 0.003 && c(1.8) < 0.012, "{}", c(1.8));
        assert!(c(4.0) > 0.02 && c(4.0) < 0.05, "{}", c(4.0));
        assert!(c(1.0) < c(2.0) && c(2.0) < c(6.0));
        assert!(c(30.0) <= 0.2);
    }
}
#[cfg(test)]
mod ocean_tests {
    use super::*;
    /// The GPU evolution + FFT reproduces a direct sum over the same modes
    /// (heights, Gerstner displacement and slopes), so the sign conventions
    /// and normalisation in fft.wgsl are right.
    #[test]
    fn gpu_fft_matches_a_direct_sum_over_the_spectrum() {
        let Some(mut gpu) = test_gpu(64, 64) else {
            eprintln!("no GPU adapter; skipped");
            return;
        };
        let sea = SeaState::spectrum(1.5, 6.0, 0.3, 0.4, 60.0, 3).unwrap();
        let look = DetailSea {
            hs: 1.5,
            tp: 6.0,
            direction: 0.3,
            spread: 0.4,
            depth: 60.0,
            wind: 9.0,
            seed: 3,
        };
        assert!(gpu.ocean.set_sea(&gpu.queue, &look, &sea));
        assert!(!gpu.ocean.set_sea(&gpu.queue, &look, &sea));
        let detail = spectrum::Detail::new(&look, spectrum::physics_cutoff(&sea));
        let time = 612.3;
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        gpu.ocean.encode(&gpu.queue, &mut encoder, time);
        let disp = ocean_gpu::tests::read_layer(
            &gpu.device,
            &gpu.queue,
            encoder,
            &gpu.ocean.texture(0),
            1,
        );
        let slope = ocean_gpu::tests::read_layer(
            &gpu.device,
            &gpu.queue,
            gpu.device.create_command_encoder(&Default::default()),
            &gpu.ocean.texture(1),
            1,
        );
        let sigma = detail.variance[1].sqrt();
        let slope_sigma = detail.mss[1].sqrt();
        assert!(sigma > 0.01, "{sigma}");
        for (x, y) in [(0, 0), (37, 201), (255, 128), (100, 3)] {
            let d = ocean_gpu::tests::direct(&detail, 1, look.depth, time, x, y);
            let g = disp[y * spectrum::N + x];
            let s = slope[y * spectrum::N + x];
            assert!(
                (g[2] as f64 - d[0]).abs() < 0.04 * sigma,
                "h {x},{y}: {} vs {}",
                g[2],
                d[0]
            );
            assert!(
                (g[0] as f64 - d[1]).abs() < 0.04 * sigma,
                "dx {x},{y}: {} vs {}",
                g[0],
                d[1]
            );
            assert!(
                (g[1] as f64 - d[2]).abs() < 0.04 * sigma,
                "dy {x},{y}: {} vs {}",
                g[1],
                d[2]
            );
            assert!(
                (s[0] as f64 - d[3]).abs() < 0.04 * slope_sigma,
                "sx {x},{y}: {} vs {}",
                s[0],
                d[3]
            );
        }
        // Field statistics: the GPU heights have the spectrum's variance.
        let mean_sq: f64 =
            disp.iter().map(|v| (v[2] as f64).powi(2)).sum::<f64>() / disp.len() as f64;
        assert!(
            (mean_sq / detail.variance[1] - 1.0).abs() < 0.35,
            "{mean_sq} {}",
            detail.variance[1]
        );
    }
}
/// Renderer for GPU tests: `None` only when the machine has no hardware
/// adapter; any other failure (for example a shader error) fails the test.
#[cfg(test)]
fn test_gpu(width: u32, height: u32) -> Option<GpuRenderer> {
    match GpuRenderer::new(width, height) {
        Ok(gpu) => Some(gpu),
        Err(e) if e.starts_with("No native GPU adapter") || e.starts_with("Only a software") => {
            None
        }
        Err(e) => panic!("GPU renderer failed to initialise: {e}"),
    }
}
#[cfg(test)]
mod ocean_view_tests {
    use super::*;
    fn view(eye: Vec3, target: Vec3) -> View {
        View {
            eye,
            target,
            up: Vec3::new(0., 0., 1.),
            fov_y: 50.,
            sun_elevation: 40f64.to_radians(),
            sun_azimuth: 2.0,
            clouds: 0.0,
            time: 3.0,
            attenuation: 0.035,
            underwater: false,
        }
    }
    /// An animal mesh drawn with `set_animals` appears where it is placed,
    /// facing its heading, and nowhere else.
    #[test]
    fn animals_are_drawn_where_they_are() {
        let Some(mut gpu) = test_gpu(160, 120) else {
            return;
        };
        let v = view(Vec3::new(0., 0., 5.0), Vec3::new(10., 0., 5.0));
        let empty = gpu.frame(v).unwrap();
        // A flat plate one unit long, spanning y and z.
        let corner = |y: f32, z: f32| DrawVertex {
            position: [0.0, y, z],
            normal: [-1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
            albedo: [0.9, 0.2, 0.1],
            roughness: 0.5,
            metallic: 0.0,
            kind: 3.0,
        };
        let plate = vec![
            corner(-0.5, -0.5),
            corner(0.5, -0.5),
            corner(0.5, 0.5),
            corner(-0.5, -0.5),
            corner(0.5, 0.5),
            corner(-0.5, 0.5),
        ];
        gpu.upload_animal_meshes(&[plate]).unwrap();
        gpu.set_animals(&[Animal {
            mesh: 0,
            position: Vec3::new(10.0, 0.0, 5.0),
            heading: Vec3::new(1.0, 0.0, 0.0),
            length: 4.0,
            tint: [1.0, 1.0, 1.0],
            ..Default::default()
        }]);
        let with = gpu.frame(v).unwrap();
        let centre = |f: &Frame| {
            let i = (60 * f.width + 80) * 3;
            [f.rgb[i] as i32, f.rgb[i + 1] as i32, f.rgb[i + 2] as i32]
        };
        let (a, b) = (centre(&empty), centre(&with));
        assert!(
            (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs() > 60,
            "animal not drawn: {a:?} vs {b:?}"
        );
        assert!(b[0] > b[2], "reddish plate expected: {b:?}");
        // Behind the camera it is not drawn.
        gpu.set_animals(&[Animal {
            mesh: 0,
            position: Vec3::new(-10.0, 0.0, 5.0),
            heading: Vec3::new(1.0, 0.0, 0.0),
            length: 4.0,
            tint: [1.0, 1.0, 1.0],
            ..Default::default()
        }]);
        let behind = gpu.frame(v).unwrap();
        assert_eq!(centre(&behind), a);
    }
    /// Long physics waves displace the projected sea: from a low eye the
    /// crests break the horizon line, which a flat sea does not.
    #[test]
    fn physics_waves_raise_crests_above_the_horizon() {
        let Some(mut gpu) = test_gpu(320, 200) else {
            return;
        };
        let look = DetailSea {
            hs: 0.0,
            tp: 6.0,
            direction: 0.0,
            spread: 0.3,
            depth: 100.0,
            wind: 0.0,
            seed: 1,
        };
        let v = view(Vec3::new(0., 0., 2.0), Vec3::new(100., 0., 2.0));
        let flat = SeaState::regular(0.0, 9.0, 0.0, 100.0).unwrap();
        gpu.set_ocean(&flat, &look, 3.0, 1).unwrap();
        let calm = gpu.frame(v).unwrap();
        let swell = SeaState::regular(4.0, 9.0, std::f64::consts::PI, 100.0).unwrap();
        gpu.set_ocean(&swell, &look, 3.0, 1).unwrap();
        let rough = gpu.frame(v).unwrap();
        let diff: f64 = calm
            .rgb
            .iter()
            .zip(&rough.rgb)
            .map(|(a, b)| (*a as f64 - *b as f64).abs())
            .sum::<f64>()
            / calm.rgb.len() as f64;
        assert!(diff > 4.0, "mean pixel change {diff}");
    }
}
