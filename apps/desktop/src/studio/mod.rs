//! GPU editor shell: winit window, wgpu swapchain, egui widgets and an
//! Unreal-style docked layout. The 3D viewport is rendered by the shared
//! `pelagic-render-gpu` pipeline straight into a texture the UI samples,
//! so camera motion never waits on the CPU or a readback.
mod a11y;
mod charts;
mod editor;
mod globe;
mod live;
mod theme;
mod world;

use std::sync::Arc;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    window::{Window, WindowId},
};

pub use editor::Studio;

/// Launch options for the GPU shell.
#[derive(Default, Clone)]
pub struct Options {
    pub open: Option<String>,
    pub storage: Option<std::path::PathBuf>,
    /// Render N frames to an offscreen image, save it as PNG and exit.
    pub capture: Option<(String, u32)>,
    pub capture_size: Option<(u32, u32)>,
    pub library_demo: bool,
    pub script: Vec<String>,
    /// Test hook: write the window handle (decimal) to this file so native
    /// UI Automation scripts can attach to this process's own window.
    pub automation_handle: Option<String>,
}

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

/// AccessKit (UI Automation) events from the screen-reader adapter.
type A11yEvent = egui_winit::accesskit_winit::Event;

struct App {
    options: Options,
    proxy: EventLoopProxy<A11yEvent>,
    gpu: Option<Gpu>,
    studio: Option<Studio>,
    ctx: egui::Context,
    frames: u32,
    failed: Option<String>,
    /// The pointer is hidden and confined for mouse-look.
    grabbed: bool,
}

pub fn run(options: Options) -> Result<(), String> {
    let _cpu_frames = CpuFrameGuard::new(&crate::session::CPU_FRAMES);
    // Also provides a deterministic way to exercise CPU recovery without
    // changing the laptop's display drivers or adapter configuration.
    if std::env::var_os("PELAGIC_DISABLE_GPU").as_deref() == Some(std::ffi::OsStr::new("1")) {
        return Err("GPU startup disabled by PELAGIC_DISABLE_GPU=1".into());
    }
    let event_loop = EventLoop::<A11yEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        proxy: event_loop.create_proxy(),
        options,
        gpu: None,
        studio: None,
        ctx: egui::Context::default(),
        frames: 0,
        failed: None,
        grabbed: false,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    match app.failed {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Restore CPU rendering on every exit, including event-loop/device errors.
struct CpuFrameGuard<'a> {
    flag: &'a std::sync::atomic::AtomicBool,
    previous: bool,
}
impl<'a> CpuFrameGuard<'a> {
    fn new(flag: &'a std::sync::atomic::AtomicBool) -> Self {
        let previous = flag.swap(false, std::sync::atomic::Ordering::Relaxed);
        Self { flag, previous }
    }
}
impl Drop for CpuFrameGuard<'_> {
    fn drop(&mut self) {
        self.flag
            .store(self.previous, std::sync::atomic::Ordering::Relaxed);
    }
}

fn icon() -> Option<winit::window::Icon> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!(
        "../../assets/pelagic-logo.png"
    )));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut data = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut data).ok()?;
    let channels = info.color_type.samples();
    // White emblem on transparent background, keyed from the logo luminance.
    let mut rgba = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    for px in data
        .chunks(channels)
        .take(info.width as usize * info.height as usize)
    {
        let alpha = if channels == 4 {
            px[3].min(px[0])
        } else {
            px[0]
        };
        rgba.extend_from_slice(&[255, 255, 255, alpha]);
    }
    winit::window::Icon::from_rgba(rgba, info.width, info.height).ok()
}

impl App {
    fn init(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let capture = self.options.capture.is_some();
        let (w, h) = self.options.capture_size.unwrap_or((1600, 900));
        let attributes = Window::default_attributes()
            .with_title("Pelagic Studio")
            .with_inner_size(winit::dpi::LogicalSize::new(w as f64, h as f64))
            .with_min_inner_size(winit::dpi::LogicalSize::new(960.0, 600.0))
            // Shown after the accessibility adapter is attached, which
            // AccessKit requires to happen before the first show.
            .with_visible(false)
            .with_maximized(!capture)
            .with_theme(Some(winit::window::Theme::Dark))
            .with_window_icon(icon());
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| e.to_string())?,
        );
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("GPU surface: {e}"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| format!("No GPU adapter for the editor window: {e}"))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Pelagic Studio"),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("The GPU cannot present to this window.")?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 2;
        config.view_formats = vec![];
        surface.configure(&device, &config);
        theme::apply(&self.ctx);
        let mut egui_state = egui_winit::State::new(
            self.ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            Some(winit::window::Theme::Dark),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        // Exposes egui's widget tree to UI Automation (Narrator, NVDA,
        // Accessibility Insights and native test scripts).
        egui_state.init_accesskit(event_loop, &window, self.proxy.clone());
        if let Some(path) = &self.options.automation_handle {
            std::fs::write(path, hwnd(&window).to_string()).map_err(|e| e.to_string())?;
        }
        if !capture {
            window.set_visible(true);
        }
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let studio = Studio::new(
            &self.options,
            device.clone(),
            queue.clone(),
            info,
            &mut egui_renderer,
            hwnd(&window),
        )?;
        self.studio = Some(studio);
        self.gpu = Some(Gpu {
            window,
            surface,
            device,
            queue,
            config,
            egui_state,
            egui_renderer,
        });
        Ok(())
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(gpu), Some(studio)) = (self.gpu.as_mut(), self.studio.as_mut()) else {
            return;
        };
        let raw = gpu.egui_state.take_egui_input(&gpu.window);
        studio.before_ui();
        let mut output = self.ctx.run_ui(raw, |ui| studio.ui(ui));
        if let Some(update) = output.platform_output.accesskit_update.as_mut() {
            a11y::clean(update);
        }
        if studio.quit {
            event_loop.exit();
        }
        gpu.egui_state
            .handle_platform_output(&gpu.window, output.platform_output);
        studio.render_viewport(&mut gpu.egui_renderer);
        let ppp = output.pixels_per_point;
        let jobs = self.ctx.tessellate(output.shapes, ppp);
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                gpu.egui_renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.config.width, gpu.config.height],
            pixels_per_point: ppp,
        };
        let capture = self.options.capture.clone();
        let offscreen = capture.as_ref().map(|_| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("studio capture"),
                size: wgpu::Extent3d {
                    width: gpu.config.width,
                    height: gpu.config.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: gpu.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let surface_texture = if offscreen.is_none() {
            match gpu.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t)
                | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
                _ => {
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    gpu.window.request_redraw();
                    return;
                }
            }
        } else {
            None
        };
        let view = match (&surface_texture, &offscreen) {
            (Some(t), _) => t.texture.create_view(&Default::default()),
            (_, Some(t)) => t.create_view(&Default::default()),
            _ => return,
        };
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("studio ui"),
            });
        let extra =
            gpu.egui_renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("studio ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.01,
                            g: 0.01,
                            b: 0.01,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            gpu.egui_renderer
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        gpu.queue
            .submit(extra.into_iter().chain(Some(encoder.finish())));
        // V-Sync preference: unlimited frame rate when off.
        let mode = if studio.vsync() {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        if gpu.config.present_mode != mode && self.options.capture.is_none() {
            gpu.config.present_mode = mode;
            gpu.surface.configure(&gpu.device, &gpu.config);
        }
        if let Some(t) = surface_texture {
            gpu.window.pre_present_notify();
            gpu.queue.present(t);
        }
        // Mouse-look hides the pointer and keeps it in the window, so it
        // does not wander over other panels while the view turns.
        let grab = studio.mouse_look() && self.options.capture.is_none();
        if grab != self.grabbed {
            self.grabbed = grab;
            gpu.window.set_cursor_visible(!grab);
            let mode = if grab {
                winit::window::CursorGrabMode::Confined
            } else {
                winit::window::CursorGrabMode::None
            };
            let _ = gpu.window.set_cursor_grab(mode);
        }
        for id in &output.textures_delta.free {
            gpu.egui_renderer.free_texture(id);
        }
        // Captures count frames only after scripted work (batches,
        // campaigns) has finished, so the image shows the result.
        if !studio.scripted_work_pending() {
            self.frames += 1;
        }
        if let (Some((path, frames)), Some(texture)) = (capture, offscreen) {
            if self.frames >= frames {
                if let Err(error) = save_texture(&gpu.device, &gpu.queue, &texture, &path) {
                    self.failed = Some(error);
                }
                event_loop.exit();
                return;
            }
            gpu.window.request_redraw();
            return;
        }
        let repaint = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .unwrap_or(std::time::Duration::MAX);
        if studio.animating() || repaint.is_zero() {
            gpu.window.request_redraw();
        } else if repaint < std::time::Duration::from_secs(5) {
            event_loop
                .set_control_flow(ControlFlow::WaitUntil(std::time::Instant::now() + repaint));
        }
    }
}

fn save_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    path: &str,
) -> Result<(), String> {
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("capture readback"),
        size: row as u64 * h as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let (send, receive) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = send.send(r);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            // Long background jobs can leave many frames queued on an
            // integrated GPU; captures wait for them.
            timeout: Some(std::time::Duration::from_secs(60)),
        })
        .map_err(|e| e.to_string())?;
    receive
        .recv()
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|e| e.to_string())?;
    let bgra = matches!(
        texture.format(),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for line in data.chunks_exact(row as usize) {
        for px in line[..(w * 4) as usize].as_chunks::<4>().0 {
            if bgra {
                rgb.extend_from_slice(&[px[2], px[1], px[0]]);
            } else {
                rgb.extend_from_slice(&px[..3]);
            }
        }
    }
    drop(data);
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?),
        w,
        h,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&rgb))
        .map_err(|e| e.to_string())
}

pub fn hwnd(window: &Window) -> isize {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

impl ApplicationHandler<A11yEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gpu.is_none() {
            if let Err(error) = self.init(event_loop) {
                self.failed = Some(error);
                event_loop.exit();
                return;
            }
            if let Some(gpu) = &self.gpu {
                gpu.window.request_redraw();
            }
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let response = gpu.egui_state.on_window_event(&gpu.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                if self.studio.as_mut().is_none_or(|s| s.confirm_close()) {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                gpu.config.width = size.width.max(1);
                gpu.config.height = size.height.max(1);
                gpu.surface.configure(&gpu.device, &gpu.config);
                gpu.window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.frame(event_loop),
            _ => {
                if response.repaint {
                    gpu.window.request_redraw();
                }
            }
        }
    }
    fn user_event(&mut self, _: &ActiveEventLoop, event: A11yEvent) {
        use egui_winit::accesskit_winit::WindowEvent as A11y;
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        match event.window_event {
            A11y::InitialTreeRequested => self.ctx.enable_accesskit(),
            A11y::ActionRequested(request) => gpu.egui_state.on_accesskit_action_request(request),
            A11y::AccessibilityDeactivated => self.ctx.disable_accesskit(),
        }
        gpu.window.request_redraw();
    }
    fn device_event(
        &mut self,
        _: &ActiveEventLoop,
        _: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        // Raw mouse motion drives fly/orbit cameras without cursor clamping.
        if let (winit::event::DeviceEvent::MouseMotion { delta }, Some(studio), Some(gpu)) =
            (event, self.studio.as_mut(), self.gpu.as_ref())
            && studio.raw_mouse(delta)
        {
            gpu.window.request_redraw();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.options.capture.is_some() && self.gpu.is_some() {
            // Hidden capture windows receive no paint messages; drive frames.
            event_loop.set_control_flow(ControlFlow::Poll);
            self.frame(event_loop);
            return;
        }
        if let (Some(gpu), Some(studio)) = (&self.gpu, &self.studio)
            && studio.animating()
        {
            gpu.window.request_redraw();
        }
    }
}

#[cfg(test)]
mod startup_tests {
    use super::CpuFrameGuard;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn device_error_restores_cpu_frames() {
        let frames = AtomicBool::new(true);
        let fail_device = || -> Result<(), &'static str> {
            let _guard = CpuFrameGuard::new(&frames);
            assert!(!frames.load(Ordering::Relaxed));
            Err("device unavailable")
        };
        assert!(fail_device().is_err());
        assert!(frames.load(Ordering::Relaxed));
    }
}
