//! GPU resources for the FFT ocean detail: initial spectrum, complex
//! fields, persistent foam, the three output texture arrays with mip
//! chains, and the compute pipelines that fill them each frame.
use crate::spectrum::{self, CASCADES, Detail, DetailSea, N};
use wgpu::util::DeviceExt;

pub const MIPS: u32 = 9;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    state: [f32; 4],
    sizes: [f32; 4],
    foam: [f32; 4],
    ramp: [f32; 4],
}

/// Statistics of the current detail spectrum used by the water shader.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub mss: [f64; CASCADES],
    pub variance: [f64; CASCADES],
    pub analytic_mss: f64,
    pub unresolved_mss: f64,
    pub whitecaps: f64,
}
impl Stats {
    pub fn total_mss(&self) -> f64 {
        self.mss.iter().sum::<f64>() + self.analytic_mss
    }
}

pub struct OceanGpu {
    params: wgpu::Buffer,
    spectrum: wgpu::Buffer,
    foam: wgpu::Buffer,
    main_group: wgpu::BindGroup,
    mip_groups: Vec<(wgpu::BindGroup, u32)>,
    evolve: wgpu::ComputePipeline,
    rows: wgpu::ComputePipeline,
    columns: wgpu::ComputePipeline,
    assemble: wgpu::ComputePipeline,
    downsample: wgpu::ComputePipeline,
    pub disp: wgpu::TextureView,
    pub slope: wgpu::TextureView,
    pub moment: wgpu::TextureView,
    #[cfg_attr(not(test), allow(dead_code))]
    textures: [wgpu::Texture; 3],
    key: Option<(u64, u64)>,
    detail: Option<Detail>,
    pub stats: Stats,
    depth: f64,
    last_time: Option<f64>,
}

fn storage_buffer(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn storage_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: FORMAT,
            view_dimension: wgpu::TextureViewDimension::D2Array,
        },
        count: None,
    }
}
fn sampled_array(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}
fn level_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("ocean level"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

impl OceanGpu {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: N as u32,
                    height: N as u32,
                    depth_or_array_layers: CASCADES as u32,
                },
                mip_level_count: MIPS,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let textures = [
            texture("ocean displacement"),
            texture("ocean slope"),
            texture("ocean moments"),
        ];
        let cells = (CASCADES * N * N) as u64;
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean parameters"),
            contents: bytemuck::bytes_of(&<Params as bytemuck::Zeroable>::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let spectrum = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean spectrum"),
            size: cells * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fields = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean fields"),
            size: cells * 4 * 8,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let foam = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean foam"),
            size: cells * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let main_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean compute"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage_buffer(1, true),
                storage_buffer(2, false),
                storage_buffer(3, false),
                storage_texture(4),
                storage_texture(5),
                storage_texture(6),
            ],
        });
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean mips"),
            entries: &[
                sampled_array(0),
                sampled_array(1),
                sampled_array(2),
                storage_texture(3),
                storage_texture(4),
                storage_texture(5),
            ],
        });
        let main_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean compute"),
            layout: &main_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: spectrum.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: fields.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: foam.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&level_view(&textures[0], 0)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&level_view(&textures[1], 0)),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&level_view(&textures[2], 0)),
                },
            ],
        });
        let mip_groups = (1..MIPS)
            .map(|level| {
                let entries: Vec<wgpu::TextureView> = textures
                    .iter()
                    .map(|t| level_view(t, level - 1))
                    .chain(textures.iter().map(|t| level_view(t, level)))
                    .collect();
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("ocean mip"),
                    layout: &mip_layout,
                    entries: &entries
                        .iter()
                        .enumerate()
                        .map(|(i, view)| wgpu::BindGroupEntry {
                            binding: i as u32,
                            resource: wgpu::BindingResource::TextureView(view),
                        })
                        .collect::<Vec<_>>(),
                });
                (group, (N as u32 >> level).max(1))
            })
            .collect();
        let fft = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean FFT"),
            source: wgpu::ShaderSource::Wgsl(include_str!("fft.wgsl").into()),
        });
        let mips = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean mips"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mips.wgsl").into()),
        });
        let main_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean compute"),
            bind_group_layouts: &[Some(&main_layout)],
            immediate_size: 0,
        });
        let mip_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean mips"),
            bind_group_layouts: &[Some(&mip_layout)],
            immediate_size: 0,
        });
        let pipeline = |module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let full = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                label: Some("ocean array"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        Self {
            evolve: pipeline(&fft, &main_pipeline_layout, "evolve"),
            rows: pipeline(&fft, &main_pipeline_layout, "fft_rows"),
            columns: pipeline(&fft, &main_pipeline_layout, "fft_columns"),
            assemble: pipeline(&fft, &main_pipeline_layout, "assemble"),
            downsample: pipeline(&mips, &mip_pipeline_layout, "downsample"),
            disp: full(&textures[0]),
            slope: full(&textures[1]),
            moment: full(&textures[2]),
            textures,
            params,
            spectrum,
            foam,
            main_group,
            mip_groups,
            key: None,
            detail: None,
            stats: Stats::default(),
            depth: 100.0,
            last_time: None,
        }
    }
    /// Uploads a new detail spectrum when the sea changes; returns whether
    /// one was rebuilt. Foam is cleared with it.
    pub fn set_sea(
        &mut self,
        queue: &wgpu::Queue,
        sea: &DetailSea,
        physics: &pelagic::waves::SeaState,
    ) -> bool {
        let cutoff = spectrum::physics_cutoff(physics);
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for v in [
            sea.hs,
            sea.tp,
            sea.direction,
            sea.spread,
            sea.depth,
            sea.wind,
            cutoff,
        ] {
            hash = (hash ^ v.to_bits()).wrapping_mul(0x100_0000_01b3);
        }
        let key = (hash, sea.seed);
        self.stats.analytic_mss = spectrum::analytic_mss(physics);
        if self.key == Some(key) {
            return false;
        }
        let detail = Detail::new(sea, cutoff);
        queue.write_buffer(&self.spectrum, 0, bytemuck::cast_slice(&detail.h0));
        queue.write_buffer(
            &self.foam,
            0,
            bytemuck::cast_slice(&vec![0f32; CASCADES * N * N]),
        );
        self.stats.mss = detail.mss;
        self.stats.variance = detail.variance;
        self.stats.unresolved_mss = detail.unresolved_mss;
        self.stats.whitecaps = detail.whitecaps;
        self.depth = sea.depth;
        self.detail = Some(detail);
        self.key = Some(key);
        true
    }
    /// Whitecap threshold and ramp on the total Jacobian.
    pub fn foam_threshold(&self) -> (f64, f64) {
        let mss = self.stats.total_mss();
        match &self.detail {
            Some(d) if mss > 0.0 => (
                d.foam_threshold(mss),
                (spectrum::CHOP * mss.sqrt() * 0.6).max(1e-3),
            ),
            _ => (-1e3, 1.0),
        }
    }
    /// Persistent-foam weight per cascade: its share of the detail wave
    /// height, so foam patches follow the larger breaking waves rather than
    /// the shortest ripples.
    pub fn foam_weights(&self) -> [f64; CASCADES] {
        let total: f64 = self.stats.variance.iter().sum();
        std::array::from_fn(|c| {
            if total > 0.0 {
                (self.stats.variance[c] / total).sqrt()
            } else {
                0.0
            }
        })
    }
    /// Records the compute work that evolves the detail field to `time`.
    pub fn encode(&mut self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, time: f64) {
        let Some(detail) = &self.detail else {
            return;
        };
        let dt = self
            .last_time
            .map_or(0.0, |last| (time - last).clamp(0.0, 0.25));
        self.last_time = Some(time);
        let thresholds: [f64; CASCADES] =
            std::array::from_fn(|c| detail.foam_threshold(self.stats.mss[c]));
        let ramps: [f64; CASCADES] =
            std::array::from_fn(|c| (spectrum::CHOP * self.stats.mss[c].sqrt() * 0.6).max(1e-3));
        let p = Params {
            state: [
                time.rem_euclid(spectrum::REPEAT) as f32,
                dt as f32,
                spectrum::CHOP as f32,
                self.depth as f32,
            ],
            sizes: [
                spectrum::SIZES[0] as f32,
                spectrum::SIZES[1] as f32,
                spectrum::SIZES[2] as f32,
                (std::f64::consts::TAU / spectrum::REPEAT) as f32,
            ],
            foam: [
                thresholds[0] as f32,
                thresholds[1] as f32,
                thresholds[2] as f32,
                3.0,
            ],
            ramp: [
                ramps[0] as f32,
                ramps[1] as f32,
                ramps[2] as f32,
                if detail.whitecaps > 0.0 { 1.0 } else { 0.0 },
            ],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&p));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ocean detail"),
            timestamp_writes: None,
        });
        let groups = (N / 16) as u32;
        pass.set_bind_group(0, &self.main_group, &[]);
        pass.set_pipeline(&self.evolve);
        pass.dispatch_workgroups(groups, groups, CASCADES as u32);
        pass.set_pipeline(&self.rows);
        pass.dispatch_workgroups(N as u32, 4, CASCADES as u32);
        pass.set_pipeline(&self.columns);
        pass.dispatch_workgroups(N as u32, 4, CASCADES as u32);
        pass.set_pipeline(&self.assemble);
        pass.dispatch_workgroups(groups, groups, CASCADES as u32);
        pass.set_pipeline(&self.downsample);
        for (group, size) in &self.mip_groups {
            pass.set_bind_group(0, group, &[]);
            let g = size.div_ceil(8);
            pass.dispatch_workgroups(g, g, CASCADES as u32);
        }
    }
    pub fn active(&self) -> bool {
        self.detail.is_some()
    }
}

#[cfg(test)]
impl OceanGpu {
    pub(crate) fn texture(&self, i: usize) -> wgpu::Texture {
        self.textures[i].clone()
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn half(bits: u16) -> f32 {
        let sign = if bits >> 15 == 1 { -1.0 } else { 1.0 };
        let exp = ((bits >> 10) & 0x1f) as i32;
        let frac = (bits & 0x3ff) as f32;
        sign * match exp {
            0 => frac * 2f32.powi(-24),
            31 => f32::INFINITY,
            e => (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
        }
    }
    /// Reads mip 0 of one layer of an ocean texture (rgba16float).
    pub(crate) fn read_layer(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        layer: u32,
    ) -> Vec<[f32; 4]> {
        let mut encoder = encoder;
        let row = (N as u32 * 8).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: row as u64 * N as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(N as u32),
                },
            },
            wgpu::Extent3d {
                width: N as u32,
                height: N as u32,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        let bytes = buffer.slice(..).get_mapped_range().unwrap();
        let mut out = Vec::with_capacity(N * N);
        for y in 0..N {
            for x in 0..N {
                let o = y * row as usize + x * 8;
                let c =
                    |i: usize| half(u16::from_le_bytes([bytes[o + i * 2], bytes[o + i * 2 + 1]]));
                out.push([c(0), c(1), c(2), c(3)]);
            }
        }
        out
    }
    /// Direct (slow) evaluation of height, displacement and x-slope of one
    /// cascade at texel (x, y), with the same quantised dispersion.
    pub(crate) fn direct(
        detail: &Detail,
        c: usize,
        depth: f64,
        time: f64,
        x: usize,
        y: usize,
    ) -> [f64; 4] {
        use std::f64::consts::TAU;
        let size = spectrum::SIZES[c];
        let dk = TAU / size;
        let quantum = TAU / spectrum::REPEAT;
        let t = time.rem_euclid(spectrum::REPEAT);
        let (px, py) = (x as f64 * size / N as f64, y as f64 * size / N as f64);
        let mut out = [0.0; 4];
        let n = N as i64;
        for j in 0..n {
            for i in 0..n {
                let s = detail.h0[(c * N + j as usize) * N + i as usize];
                if s == [0.0; 4] {
                    continue;
                }
                let kx = (if i >= n / 2 { i - n } else { i }) as f64 * dk;
                let ky = (if j >= n / 2 { j - n } else { j }) as f64 * dk;
                let k = kx.hypot(ky);
                let w = (9.80665 * k * (k * depth).min(20.0).tanh()).sqrt();
                let wq = (w as f32 / quantum as f32).floor() as f64 * quantum;
                let (e_re, e_im) = ((wq * t).cos(), (wq * t).sin());
                let h_re = s[0] as f64 * e_re - s[1] as f64 * e_im
                    + s[2] as f64 * e_re
                    + s[3] as f64 * e_im;
                let h_im = s[0] as f64 * e_im + s[1] as f64 * e_re - s[2] as f64 * e_im
                    + s[3] as f64 * e_re;
                let (b_re, b_im) = ((kx * px + ky * py).cos(), (kx * px + ky * py).sin());
                // Re(h e^{ik.x}), Re(i k̂x h e^{ik.x}), Re(i kx h e^{ik.x})
                let re = h_re * b_re - h_im * b_im;
                let im = h_re * b_im + h_im * b_re;
                out[0] += re;
                out[1] += -im * kx / k * spectrum::CHOP;
                out[2] += -im * ky / k * spectrum::CHOP;
                out[3] += -im * kx;
            }
        }
        out
    }
}
