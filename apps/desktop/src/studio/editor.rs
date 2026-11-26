//! Editor state and panels for the GPU shell. All scene changes go through
//! `crate::editor::edit` (validation, undo/redo, recovery, dirty state).
use super::theme;
#[path = "campaign_ui.rs"]
mod campaign_ui;
#[path = "experiments.rs"]
mod experiments;
#[path = "graphics.rs"]
pub(super) mod graphics;
#[path = "inspection_ui.rs"]
mod inspection_ui;
#[path = "overview.rs"]
mod overview;
#[path = "sar_ui.rs"]
mod sar_ui;
#[path = "seakeeping_ui.rs"]
mod seakeeping_ui;
#[path = "sensors.rs"]
mod sensors;
#[path = "ship_ui.rs"]
mod ship_ui;
#[path = "spill_ui.rs"]
mod spill_ui;
#[path = "survey_ui.rs"]
mod survey_ui;
#[path = "trials_ui.rs"]
mod trials_ui;
#[path = "vehicle_ui.rs"]
mod vehicle_ui;
use crate::{
    library::{self, Item},
    project::{Document, Scene},
    view::Model,
    viewport_edit::{self, Actor, Drag, Gizmo, Handle, Snap, Tool},
};
use egui::{
    Align2, Color32, FontId, Key, PointerButton, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use pelagic::math::Vec3;
use std::{collections::HashMap, sync::mpsc::Receiver, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Viewport,
    Outliner,
    Details,
    Content,
    Log,
    Globe,
    Experiments,
    Sensors,
    Trials,
    Seakeeping,
    Sar,
    Survey,
    Ship,
    Inspection,
    Spill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selection {
    None,
    Ocean,
    Seabed,
    Life,
    Sensors,
    Actor(Actor),
}

/// Free-flying editor camera (Unreal conventions: RMB + WASD fly,
/// Alt + LMB orbit, MMB pan, wheel dolly, F frame).
#[derive(Clone, Copy)]
struct FlyCam {
    eye: Vec3,
    yaw: f64,
    pitch: f64,
    focus: f64,
    speed: f64,
}
impl FlyCam {
    fn from_scene(scene: &Scene) -> Self {
        let eye = Vec3::new(scene.camera[0], scene.camera[1], scene.camera[2]);
        let target = Vec3::new(scene.target[0], scene.target[1], scene.target[2]);
        let d = target - eye;
        let len = d.length().max(0.1);
        Self {
            eye,
            yaw: d.y.atan2(d.x),
            pitch: (d.z / len).clamp(-1.0, 1.0).asin(),
            focus: len,
            speed: (len * 0.6).clamp(2.0, 60.0),
        }
    }
    fn forward(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.cos(),
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
        )
    }
    fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::new(0.0, 0.0, 1.0)).normalized()
    }
    fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }
    fn target(&self) -> Vec3 {
        self.eye + self.forward() * self.focus
    }
    fn look(&mut self, dx: f64, dy: f64) {
        self.yaw -= dx * 0.0032;
        self.pitch = (self.pitch - dy * 0.0032).clamp(-1.5, 1.5);
    }
    fn orbit(&mut self, dx: f64, dy: f64) {
        let target = self.target();
        self.look(dx, dy);
        self.eye = target - self.forward() * self.focus;
    }
}

const FOV: f64 = 55.0;

pub struct Studio {
    pub quit: bool,
    dock: DockState<Tab>,
    s: State,
}

struct State {
    model: Model,
    device: wgpu::Device,
    renderer: Option<pelagic_render_gpu::GpuRenderer>,
    adapter: String,
    viewport_tex: Option<egui::TextureId>,
    viewport_px: (u32, u32),
    viewport_rect: Rect,
    uploaded: u64,
    /// (world revision, quality, terrain material) of the cached static layer.
    /// Hash of everything the cached static landscape layer depends on.
    static_key: u64,
    landscape: pelagic::landscape::Counts,
    landscape_ms: f64,
    /// Sun elevation, azimuth and cloud cover while a quick-bar slider is
    /// dragged (committed to the scene on release).
    env_preview: Option<(f64, f64, f64)>,
    /// Local solar time shown on the quick bar (hours).
    time_of_day: f64,
    /// Viewport graphics settings (app preference).
    graphics: graphics::Graphics,
    /// When the simulation was paused, for the ambient sea animation.
    paused_since: Option<Instant>,
    /// The viewport was drawn this frame.
    viewport_visible: bool,
    /// Real terrain and seabed streamed around the camera beyond the scene,
    /// and the place it was started for.
    world: Option<super::world::World>,
    world_place: Option<(f64, f64)>,
    /// Marine animals around the camera: the species recorded in the area
    /// (OBIS, looked up per half-degree cell), their meshes and clock.
    fauna: pelagic::fauna::Population,
    fauna_rx: Option<Receiver<(usize, Vec<pelagic::fauna::Species>)>>,
    fauna_cell: Option<(i64, i64)>,
    fauna_upload: bool,
    fauna_clock: Instant,
    /// Species with records in the area (0 = none found, representative
    /// species shown).
    fauna_recorded: usize,
    /// Live conditions here (observations first) and their request.
    live: Option<super::live::LiveNow>,
    live_rx: Option<Receiver<Result<super::live::LiveNow, String>>>,
    /// Live mode: conditions refresh every 10 minutes and real ships show.
    live_on: bool,
    live_at: Option<Instant>,
    /// Hours ahead of the forecast now shown (None = live or manual).
    forecast_ahead: Option<u32>,
    /// Live AIS ships near the scene.
    ships: Vec<LiveShip>,
    ships_at: Option<Instant>,
    /// Camera flight in progress (Go to land, views).
    flight: Option<Flight>,
    /// Camera velocity (smoothed) and left-drag looking.
    velocity: Vec3,
    looking: bool,
    /// Nearest land to the camera (distance, x, y) and when last searched.
    nearest_land: Option<(f64, f64, f64)>,
    land_at: Option<Instant>,
    /// Bottom of the quick bar (the compass goes below it).
    quick_bottom: f32,
    /// Scripts wait for terrain, species, live data and flights.
    wait_world: bool,
    /// Live, but with the sun set to daytime (it is night at the place).
    daylight: bool,
    /// Satellite colours being re-fetched for a scene whose stored colours
    /// came from masked tiles, and the place last checked.
    imagery_rx: Option<Receiver<SceneImagery>>,
    imagery_checked: Option<(f64, f64)>,
    /// Trees on vegetated land near the camera and where they were placed.
    trees: Vec<pelagic_render_gpu::Animal>,
    trees_at: Option<Vec3>,
    /// The default real sea is being built at startup.
    startup_build: bool,
    /// Refresh the opened real-world scene to the live sea once.
    auto_live: bool,
    /// Forecast being fetched for the scene's location.
    live_sea:
        Option<std::sync::mpsc::Receiver<Result<(pelagic::metocean::Conditions, String), String>>>,
    view_dirty: bool,
    cam: FlyCam,
    selection: Selection,
    tool: Tool,
    snap: Snap,
    drag: Option<(Drag, Option<Scene>)>,
    flying: bool,
    orbiting: bool,
    panning: bool,
    mouse: Vec2,
    last: Instant,
    frame_ms: f64,
    build_ms: f64,
    thumbs: HashMap<String, egui::TextureHandle>,
    search: String,
    content_filter: u8,
    pending: Option<Scene>,
    import: Option<Receiver<Result<(String, pelagic::assets::Mesh), String>>>,
    hwnd: isize,
    reset_layout: bool,
    globe: super::globe::Globe,
    error: Option<String>,
    unattended: bool,
    focus_viewport: bool,
    focus_experiments: bool,
    /// Background experiment task (batch, verify, exports) and its progress.
    job: Option<crate::jobs::Job>,
    job_status: String,
    script_queue: std::collections::VecDeque<String>,
    /// Viewport shows the mounted sensor camera instead of the editor camera.
    sensor_camera: bool,
    focus_sensors: bool,
    campaign: campaign_ui::Panel,
    focus_trials: bool,
    trials: trials_ui::Panel,
    focus_seakeeping: bool,
    seakeeping: seakeeping_ui::Panel,
    focus_sar: bool,
    sar: sar_ui::Panel,
    focus_survey: bool,
    survey: survey_ui::Panel,
    focus_ship: bool,
    ship: ship_ui::Panel,
    focus_inspection: bool,
    inspection: inspection_ui::Panel,
    focus_spill: bool,
    spill: spill_ui::Panel,
}

impl Studio {
    pub fn new(
        options: &super::Options,
        device: wgpu::Device,
        queue: wgpu::Queue,
        info: wgpu::AdapterInfo,
        egui_renderer: &mut egui_wgpu::Renderer,
        hwnd: isize,
    ) -> Result<Self, String> {
        let mut model = Model {
            storage_root: options.storage.clone(),
            dark_mode: true,
            ..Default::default()
        };
        crate::editor::preferences(&mut model);
        let mut want_real_sea = false;
        let document = match &options.open {
            Some(path) => Document::open(std::path::Path::new(path))?,
            None if options.library_demo => Document::new(library::demo_scene()),
            // Open on a real sea: the last real-world scene, else the cached
            // default real sea, else build it (the template shows meanwhile).
            None => model
                .recent
                .iter()
                .find_map(|p| {
                    Document::open(std::path::Path::new(p))
                        .ok()
                        .filter(|d| real_place(&d.scene))
                })
                .or_else(|| {
                    Scene::read(&crate::editor::storage(&model).join(STARTUP_SEA))
                        .ok()
                        .map(|mut scene| {
                            // A real place opens as it is: no template
                            // vehicle or simulated school (real animals and
                            // ships are drawn instead).
                            scene.vehicle = false;
                            scene.school.count = 0;
                            Document::new(scene)
                        })
                })
                .unwrap_or_else(|| {
                    want_real_sea = true;
                    Document::new(library::demo_scene())
                }),
        };
        let opened_real = real_place(&document.scene);
        let cam = FlyCam::from_scene(&document.scene);
        crate::editor::install(&mut model, document);
        model.notice.clear();
        let adapter = format!("{} ({:?})", info.name, info.backend);
        let discrete = info.device_type == wgpu::DeviceType::DiscreteGpu;
        let graphics = graphics::Graphics::load(&crate::editor::storage(&model), discrete);
        let renderer =
            pelagic_render_gpu::GpuRenderer::with_device(device.clone(), queue, info, 960, 540)?;
        let viewport_tex = Some(egui_renderer.register_native_texture(
            &device,
            &renderer.output_view(),
            wgpu::FilterMode::Linear,
        ));
        let mut studio = Self {
            quit: false,
            dock: default_layout(),
            s: State {
                model,
                device,
                renderer: Some(renderer),
                adapter,
                viewport_tex,
                viewport_px: (960, 540),
                viewport_rect: Rect::NOTHING,
                uploaded: 0,
                static_key: 0,
                landscape: Default::default(),
                landscape_ms: 0.0,
                env_preview: None,
                time_of_day: 10.0,
                live_sea: None,
                startup_build: false,
                auto_live: false,
                world: None,
                world_place: None,
                fauna: Default::default(),
                fauna_rx: None,
                fauna_cell: None,
                fauna_upload: true,
                fauna_clock: Instant::now(),
                fauna_recorded: 0,
                live: None,
                live_rx: None,
                live_on: false,
                live_at: None,
                forecast_ahead: None,
                ships: vec![],
                ships_at: None,
                flight: None,
                velocity: Vec3::ZERO,
                looking: false,
                nearest_land: None,
                land_at: None,
                quick_bottom: 0.0,
                wait_world: false,
                daylight: false,
                trees: vec![],
                trees_at: None,
                imagery_rx: None,
                imagery_checked: None,
                graphics,
                paused_since: None,
                viewport_visible: false,
                view_dirty: true,
                cam,
                selection: Selection::None,
                tool: Tool::Move,
                snap: Snap::Off,
                drag: None,
                flying: false,
                orbiting: false,
                panning: false,
                mouse: Vec2::ZERO,
                last: Instant::now(),
                frame_ms: 16.0,
                build_ms: 0.0,
                thumbs: HashMap::new(),
                search: String::new(),
                content_filter: 0,
                pending: None,
                import: None,
                hwnd,
                reset_layout: false,
                globe: super::globe::Globe::default(),
                error: None,
                unattended: options.capture.is_some(),
                focus_viewport: false,
                focus_experiments: false,
                job: None,
                job_status: String::new(),
                script_queue: Default::default(),
                sensor_camera: false,
                focus_sensors: false,
                campaign: Default::default(),
                focus_trials: false,
                trials: Default::default(),
                focus_seakeeping: false,
                seakeeping: Default::default(),
                focus_sar: false,
                sar: Default::default(),
                focus_survey: false,
                survey: Default::default(),
                focus_ship: false,
                ship: Default::default(),
                focus_inspection: false,
                inspection: Default::default(),
                focus_spill: false,
                spill: Default::default(),
            },
        };
        // Script commands run in order; each waits for the previous
        // background task (batch, export) to finish.
        studio.s.script_queue = options.script.iter().cloned().collect();
        studio.s.run_scripts();
        studio
            .s
            .log(format!("GPU editor ready on {}", studio.s.adapter));
        if want_real_sea {
            studio.s.globe.build_default_sea();
            studio.s.startup_build = true;
        }
        studio.s.auto_live = opened_real && !want_real_sea;
        studio.s.live_on = opened_real;
        Ok(studio)
    }

    pub fn animating(&self) -> bool {
        let s = &self.s;
        s.flying
            || s.orbiting
            || s.panning
            || s.looking
            || s.flight.is_some()
            || s.live_rx.is_some()
            || s.velocity.length() > 1e-3
            || s.drag.is_some()
            || s.import.is_some()
            || s.background_busy()
            || !s.script_queue.is_empty()
            || s.globe.busy()
            || s.model.session.as_ref().is_some_and(|x| x.running)
            || (s.graphics.animate_paused && s.viewport_visible)
    }

    /// The view is being turned with the mouse (pointer hidden).
    pub fn mouse_look(&self) -> bool {
        self.s.flying || self.s.looking
    }

    /// Present without waiting for the display refresh.
    pub fn vsync(&self) -> bool {
        self.s.graphics.vsync
    }

    /// Script commands or a background task are still running.
    pub fn scripted_work_pending(&self) -> bool {
        self.s.wait_world
            || !self.s.script_queue.is_empty()
            || self.s.job.is_some()
            || self.s.globe.sea_loading()
            || self.s.globe.building()
            || self.s.globe.features_busy()
    }

    pub fn raw_mouse(&mut self, delta: (f64, f64)) -> bool {
        let s = &mut self.s;
        if s.flying || s.orbiting || s.panning || s.looking {
            s.mouse += Vec2::new(delta.0 as f32, delta.1 as f32);
            true
        } else {
            false
        }
    }

    pub fn confirm_close(&mut self) -> bool {
        self.s.confirm_discard("closing Pelagic")
    }

    pub fn before_ui(&mut self) {
        let s = &mut self.s;
        s.viewport_visible = false;
        let now = Instant::now();
        let dt = now.duration_since(s.last).as_secs_f64().min(0.25);
        s.last = now;
        s.frame_ms = s.frame_ms * 0.9 + dt * 1000.0 * 0.1;
        if let Some(session) = s.model.session.as_mut()
            && session.running
        {
            session.advance_view(dt, false);
        }
        s.poll_import();
        s.poll_background();
        s.run_scripts();
        s.globe.poll();
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Live conditions load and refresh whichever tab is showing.
        if self.s.model.document.is_some() {
            self.s.poll_live();
            self.s.poll_imagery();
        }
        // Scenes built on the Earth tab (or at startup) open even while
        // another tab is showing.
        if let Some(scene) = self.s.globe.poll_build() {
            if std::mem::take(&mut self.s.startup_build) {
                let _ = crate::project::atomic_json(
                    &crate::editor::storage(&self.s.model).join(STARTUP_SEA),
                    &scene.value(),
                );
            }
            self.s.open_scene(scene);
            self.s.focus_viewport = true;
        }
        if self.s.reset_layout {
            self.dock = default_layout();
            self.s.reset_layout = false;
        }
        if std::mem::take(&mut self.s.globe.focus)
            && let Some(path) = self.dock.find_tab(&Tab::Globe)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_viewport)
            && let Some(path) = self.dock.find_tab(&Tab::Viewport)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_sensors)
            && let Some(path) = self.dock.find_tab(&Tab::Sensors)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_experiments)
            && let Some(path) = self.dock.find_tab(&Tab::Experiments)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_trials)
            && let Some(path) = self.dock.find_tab(&Tab::Trials)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_seakeeping)
            && let Some(path) = self.dock.find_tab(&Tab::Seakeeping)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_sar)
            && let Some(path) = self.dock.find_tab(&Tab::Sar)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_survey)
            && let Some(path) = self.dock.find_tab(&Tab::Survey)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_ship)
            && let Some(path) = self.dock.find_tab(&Tab::Ship)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_inspection)
            && let Some(path) = self.dock.find_tab(&Tab::Inspection)
        {
            let _ = self.dock.set_active_tab(path);
        }
        if std::mem::take(&mut self.s.focus_spill)
            && let Some(path) = self.dock.find_tab(&Tab::Spill)
        {
            let _ = self.dock.set_active_tab(path);
        }
        self.s.globe.spill = self.s.spill.overlay();
        self.s.globe.sar = self.s.sar.overlay();
        self.s.shortcuts(ui);
        self.s.menu_bar(ui);
        self.s.toolbar(ui);
        self.s.status_bar(ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(4))
            .show(ui, |ui| {
                let mut style = egui_dock::Style::from_egui(ui.style().as_ref());
                let top = egui::CornerRadius {
                    nw: 7,
                    ne: 7,
                    sw: 0,
                    se: 0,
                };
                style.tab_bar.bg_fill = theme::HEADER;
                style.tab_bar.height = 32.0;
                style.tab_bar.hline_color = theme::BORDER;
                style.tab_bar.corner_radius = egui::CornerRadius::same(8);
                for (state, fill, text) in [
                    (&mut style.tab.active, theme::PANEL, theme::STRONG),
                    (&mut style.tab.focused, theme::PANEL, theme::ACCENT),
                    (
                        &mut style.tab.active_with_kb_focus,
                        theme::PANEL,
                        theme::ACCENT,
                    ),
                    (
                        &mut style.tab.focused_with_kb_focus,
                        theme::PANEL,
                        theme::ACCENT,
                    ),
                    (&mut style.tab.inactive, theme::HEADER, theme::MUTED),
                    (
                        &mut style.tab.inactive_with_kb_focus,
                        theme::HEADER,
                        theme::TEXT,
                    ),
                    (&mut style.tab.hovered, theme::HOVER, theme::STRONG),
                ] {
                    state.bg_fill = fill;
                    state.text_color = text;
                    state.outline_color = theme::BORDER;
                    state.corner_radius = top;
                }
                style.tab.hline_below_active_tab_name = true;
                style.tab.tab_body.bg_fill = theme::PANEL;
                style.tab.tab_body.stroke = egui::Stroke::new(1.0, theme::BORDER);
                style.tab.tab_body.inner_margin = egui::Margin::same(8);
                style.separator.color_idle = theme::BG;
                style.separator.color_hovered = theme::ACCENT;
                style.separator.color_dragged = theme::ACCENT;
                style.separator.width = 4.0;
                DockArea::new(&mut self.dock)
                    .style(style)
                    .show_close_buttons(false)
                    .show_add_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut self.s);
            });
        self.quit |= std::mem::take(&mut self.s.model.about);
        self.s.model.record_notice();
    }

    /// Renders the 3D view into the texture egui samples. Geometry is rebuilt
    /// only when the session revision changes; camera moves only update uniforms.
    pub fn render_viewport(&mut self, egui_renderer: &mut egui_wgpu::Renderer) {
        let s = &mut self.s;
        let Some(renderer) = s.renderer.as_mut() else {
            return;
        };
        let (w, h) = s.viewport_px;
        if renderer.size() != (w, h) {
            if let Err(e) = renderer.resize(w, h) {
                s.error = Some(e);
                return;
            }
            if let Some(id) = s.viewport_tex {
                egui_renderer.update_egui_texture_from_wgpu_texture(
                    &s.device,
                    &renderer.output_view(),
                    wgpu::FilterMode::Linear,
                    id,
                );
            }
            s.view_dirty = true;
        }
        let Some(session) = s.model.session.as_ref() else {
            return;
        };
        let scene = &s.model.document.as_ref().unwrap().scene;
        let quality = s.graphics.preset;
        let static_key = landscape_key(scene, session.world_revision, quality, s.drag.is_some());
        if static_key != s.static_key {
            // Seabed, backdrop, structures and seabed life: rebuilt only when
            // what they depend on changes, never on ticks or sun/cloud edits.
            let start = Instant::now();
            let geometry = if scene.terrain_material == 0 {
                let look = pelagic::landscape::Landscape {
                    backdrop: scene.backdrop.as_deref(),
                    habitat: scene.habitat.as_deref(),
                    seed: scene.seed,
                    quality,
                    imagery: scene.imagery.as_deref(),
                    backdrop_imagery: scene.backdrop_imagery.as_deref(),
                };
                let (geometry, counts) = pelagic::landscape::build(session.view_ocean(), &look);
                s.landscape = counts;
                geometry
            } else {
                pelagic::render_scene::RenderScene::build_static(
                    session.view_ocean(),
                    quality,
                    scene.terrain_material,
                )
            };
            // The open sea is the projected FFT ocean (`set_ocean` below).
            if let Err(e) = renderer
                .upload_static(&geometry)
                .and_then(|()| renderer.upload_water(&[]))
            {
                s.error = Some(format!("Viewport geometry: {e}"));
            }
            s.landscape_ms = start.elapsed().as_secs_f64() * 1000.0;
            s.static_key = static_key;
            s.uploaded = 0;
        }
        // While paused, the sea keeps moving (visual only) unless switched
        // off; on Play the waves return to the simulation's own time.
        if session.running {
            s.paused_since = None;
        } else if s.paused_since.is_none() {
            s.paused_since = Some(Instant::now());
        }
        // Only while the viewport is on screen: a hidden viewport neither
        // animates nor re-renders.
        let ambient = if !session.running && s.graphics.animate_paused && s.viewport_visible {
            s.paused_since.map_or(0.0, |t| t.elapsed().as_secs_f64())
        } else {
            0.0
        };
        if ambient > 0.0 {
            s.view_dirty = true;
        }
        let snapshot_time = session.view_snapshot().time + ambient;
        // The world beyond the scene: real terrain streamed around the camera.
        let real = real_place(scene);
        let origin = (
            scene.provenance.origin_latitude,
            scene.provenance.origin_longitude,
        );
        let geo = super::world::Geo {
            lat: origin.0,
            lon: origin.1,
        };
        if !real {
            if s.world.take().is_some() {
                let _ = renderer.upload_world(&[]);
            }
            s.world_place = None;
        } else if s.world_place != Some(origin) {
            s.world_place = Some(origin);
            let covered = scene
                .backdrop
                .as_deref()
                .or(scene.grid.as_deref())
                .map(|g| {
                    let w = (g.columns - 1) as f64 * g.spacing;
                    let h = (g.rows - 1) as f64 * g.spacing;
                    [g.origin[0], g.origin[1], g.origin[0] + w, g.origin[1] + h]
                });
            s.world = Some(super::world::World::new(geo, covered, quality));
            s.nearest_land = None;
            s.land_at = None;
            let _ = renderer.upload_world(&[]);
        }
        if let Some(world) = s.world.as_mut()
            && let Some(mesh) = world.update(s.cam.eye)
        {
            if let Err(e) = renderer.upload_world(&mesh) {
                s.error = Some(format!("Distant terrain: {e}"));
            }
            s.land_at = None;
            s.trees_at = None;
            s.view_dirty = true;
        }
        // Marine animals: the species recorded where the camera is (OBIS).
        let (cam_lat, cam_lon) = geo.to_latlon(s.cam.eye.x, s.cam.eye.y);
        let cell = if real {
            (
                (cam_lat * 2.0).round() as i64,
                (cam_lon * 2.0).round() as i64,
            )
        } else {
            (i64::MAX, i64::MAX)
        };
        if s.fauna_cell != Some(cell) {
            s.fauna_cell = Some(cell);
            if real {
                let (send, receive) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let list = super::globe::marine::checklist_here(cam_lat, cam_lon);
                    let chosen = pelagic::fauna::choose(&list, 14);
                    let _ = send.send((list.len(), chosen));
                });
                s.fauna_rx = Some(receive);
            } else {
                s.fauna =
                    pelagic::fauna::Population::new(pelagic::fauna::representative(), scene.seed);
                s.fauna_recorded = 0;
                s.fauna_upload = true;
            }
        }
        if let Some(rx) = &s.fauna_rx
            && let Ok((recorded, mut species)) = rx.try_recv()
        {
            s.fauna_rx = None;
            s.fauna_recorded = recorded;
            // Offline, or nothing recorded: representative species only where
            // they belong (tropical seas).
            if species.is_empty() && cam_lat.abs() < 30.0 {
                species = pelagic::fauna::representative();
            }
            s.fauna = pelagic::fauna::Population::new(species, scene.seed);
            s.fauna_upload = true;
        }
        let species_count = s.fauna.species.len();
        if std::mem::take(&mut s.fauna_upload) {
            let mut meshes = Vec::with_capacity(species_count * 2 + super::world::SHIP_KINDS);
            for sp in &s.fauna.species {
                meshes.push(pelagic::fauna::mesh(sp, 1));
                meshes.push(pelagic::fauna::mesh(sp, 0));
            }
            for k in 0..super::world::SHIP_KINDS {
                meshes.push(super::world::ship_mesh(k));
            }
            for k in 0..super::world::TREE_KINDS {
                meshes.push(super::world::tree_mesh(k));
            }
            s.trees_at = None;
            if let Err(e) = renderer.upload_animal_meshes(&meshes) {
                s.error = Some(format!("Animal models: {e}"));
            }
        }
        let now = Instant::now();
        let dt = now.duration_since(s.fauna_clock).as_secs_f64();
        s.fauna_clock = now;
        let ocean = session.view_ocean();
        let world = s.world.as_ref();
        let elevation = |x: f64, y: f64| ground_at(scene, ocean, world, x, y);
        let habitat = |x: f64, y: f64| scene.habitat.as_deref().map_or(0, |h| h.at(x, y));
        let mut animals: Vec<pelagic_render_gpu::Animal> = vec![];
        if s.cam.eye.z < 400.0 {
            let moving = s.viewport_visible && (ambient > 0.0 || session.running);
            let surroundings = pelagic::fauna::Surroundings {
                elevation: &elevation,
                habitat: &habitat,
                camera: s.cam.eye,
                range: (1.6 / scene.attenuation.max(0.01)).clamp(25.0, 90.0),
                budget: [90, 180, 320, 520][quality.min(3) as usize],
            };
            if moving || s.fauna.is_empty() {
                s.fauna.update(if moving { dt } else { 0.0 }, &surroundings);
            }
            for pose in s.fauna.poses(snapshot_time, &elevation) {
                let far = (pose.position - s.cam.eye).length() > 18.0 + pose.length * 6.0;
                animals.push(pelagic_render_gpu::Animal {
                    mesh: pose.species * 2 + usize::from(far),
                    position: pose.position,
                    heading: pose.heading,
                    length: pose.length,
                    mode: pose.mode,
                    amplitude: pose.amplitude,
                    frequency: pose.frequency,
                    wavenumber: pose.wavenumber,
                    phase: pose.phase,
                    tint: pose.tint,
                    bend: pose.bend,
                });
            }
        } else {
            s.fauna.clear();
        }
        // Live ships (AIS) near the scene, dead-reckoned to now.
        let ships_wanted = s.live_on && real;
        s.globe.marine.viewport_ships = ships_wanted;
        s.globe.marine.run_feeds();
        if !ships_wanted {
            s.ships.clear();
        } else if s.ships_at.is_none_or(|t| t.elapsed().as_secs_f64() > 2.0) {
            s.ships_at = Some(Instant::now());
            s.ships = s
                .globe
                .marine
                .ships_near(origin.0, origin.1, 80.0)
                .into_iter()
                .map(|sh| {
                    let kind = super::world::ship_kind(sh.kind);
                    LiveShip {
                        name: if sh.name.trim().is_empty() {
                            format!("MMSI {}", sh.mmsi)
                        } else {
                            sh.name.trim().to_string()
                        },
                        lat: sh.lat,
                        lon: sh.lon,
                        time: sh.time,
                        sog: sh.sog,
                        cog: sh.cog,
                        heading: sh.heading.filter(|h| *h < 360.0).unwrap_or(sh.cog),
                        length: sh
                            .size
                            .map(|z| z.0)
                            .filter(|l| *l > 3.0 && *l < 460.0)
                            .unwrap_or([180.0, 200.0, 150.0, 25.0, 30.0, 12.0, 40.0][kind]),
                        kind,
                        category: crate::ais::category(sh.kind),
                    }
                })
                .collect();
        }
        let clock = unix_now();
        let mut ship_wakes = vec![];
        for ship in &s.ships {
            let minutes = ((clock - ship.time as f64) / 60.0).clamp(0.0, 10.0);
            let (la, lo) = crate::ais::dead_reckon(ship.lat, ship.lon, ship.sog, ship.cog, minutes);
            let (x, y) = geo.to_xy(la, lo);
            let h = ship.heading.to_radians();
            let heading = Vec3::new(h.sin(), h.cos(), 0.0);
            animals.push(pelagic_render_gpu::Animal {
                mesh: species_count * 2 + ship.kind,
                position: Vec3::new(x, y, 0.0),
                heading,
                length: ship.length,
                tint: [1.0, 1.0, 1.0],
                ..Default::default()
            });
            let v = ship.sog * 0.514_444;
            let c = ship.cog.to_radians();
            if v > 0.5 && (Vec3::new(x, y, 0.0) - s.cam.eye).length() < 6_000.0 {
                ship_wakes.push(pelagic_render_gpu::Wake {
                    x,
                    y,
                    vx: v * c.sin(),
                    vy: v * c.cos(),
                    length: ship.length,
                    beam: ship.length * 0.15,
                });
            }
        }
        // Trees where the satellite image shows vegetation (palms on tropical
        // coasts), re-planted as the camera moves.
        let tree_base = species_count * 2 + super::world::SHIP_KINDS;
        if !real || s.cam.eye.z > 3_000.0 {
            s.trees.clear();
            s.trees_at = None;
        } else if s.trees_at.is_none_or(|p| (p - s.cam.eye).length() > 200.0) {
            s.trees_at = Some(s.cam.eye);
            let cap = [600, 1_200, 2_500, 5_000][quality.min(3) as usize];
            s.trees = plant_trees(scene, ocean, world, s.cam.eye, cam_lat, cap)
                .into_iter()
                .map(|mut t| {
                    t.mesh += tree_base;
                    t
                })
                .collect();
            s.view_dirty = true;
        }
        animals.extend_from_slice(&s.trees);
        // Redraw while animals swim or live ships move.
        if s.viewport_visible
            && ((!animals.is_empty() && (ambient > 0.0 || session.running)) || !s.ships.is_empty())
        {
            s.view_dirty = true;
        }
        renderer.set_animals(&animals);
        renderer.set_visibility(
            s.live
                .as_ref()
                .and_then(|l| {
                    l.metar
                        .as_ref()
                        .and_then(|m| m.visibility_m)
                        .or(l.visibility_m)
                })
                .filter(|_| s.live_on)
                .unwrap_or(40_000.0),
        );
        if session.revision != s.uploaded {
            let start = Instant::now();
            let snapshot = session.view_snapshot();
            let built =
                pelagic::render_scene::RenderScene::build_bodies(session.view_ocean(), &snapshot)
                    .and_then(|geometry| {
                        renderer
                            .upload_dynamic(&geometry)
                            .map_err(|_| "upload failed")
                    });
            if let Err(e) = built {
                s.error = Some(format!("Viewport geometry: {e}"));
            }
            s.build_ms = start.elapsed().as_secs_f64() * 1000.0;
            s.uploaded = session.revision;
            s.view_dirty = true;
        }
        if !s.view_dirty {
            return;
        }
        let ocean = session.view_ocean();
        let (eye, target, up, fov_y) = s
            .sensor_camera
            .then(|| sensors::mounted_camera(&s.model))
            .flatten()
            .unwrap_or((s.cam.eye, s.cam.target(), Vec3::new(0.0, 0.0, 1.0), FOV));
        let view = pelagic_render_gpu::View {
            eye,
            target,
            up,
            fov_y,
            sun_elevation: s
                .env_preview
                .map_or(scene.sun_elevation, |p| p.0)
                .to_radians(),
            sun_azimuth: s
                .env_preview
                .map_or(scene.sun_azimuth, |p| p.1)
                .to_radians(),
            clouds: s.env_preview.map_or(scene.clouds, |p| p.2),
            time: snapshot_time,
            attenuation: ocean.attenuation,
            underwater: eye.z < ocean.surface(eye.x, eye.y, snapshot_time).height,
        };
        let detail = detail_sea(scene);
        let snapshot = session.view_snapshot();
        let wakes: Vec<pelagic_render_gpu::Wake> = snapshot
            .vehicles
            .iter()
            .filter(|p| p.kind == 1 || (p.position.z + p.half.z > -0.5 && p.half.x > 1.0))
            .map(|p| pelagic_render_gpu::Wake {
                x: p.position.x,
                y: p.position.y,
                vx: p.velocity.x,
                vy: p.velocity.y,
                length: p.half.x * 2.0,
                beam: p.half.y * 2.0,
            })
            .chain(ship_wakes)
            .collect();
        renderer.set_wakes(&wakes);
        if let Err(e) = renderer
            .set_ocean(&ocean.waves, &detail, snapshot_time, quality)
            .and_then(|()| renderer.draw(view))
        {
            s.error = Some(e);
        }
        // Reclaim finished uploads every frame (offscreen captures and hidden
        // windows never present, which would otherwise let staging grow).
        let _ = s.device.poll(wgpu::PollType::Poll);
        s.view_dirty = false;
    }
}

/// The wind sea whose short-wave tail the GPU ocean draws.
pub(super) fn detail_sea(scene: &Scene) -> pelagic_render_gpu::DetailSea {
    pelagic_render_gpu::DetailSea {
        hs: scene.wave_height,
        tp: scene.wave_period,
        direction: scene.wave_direction.to_radians(),
        spread: scene.wave_spread.to_radians(),
        depth: scene.bottom_depth.max(1.0),
        wind: scene.wind_speed,
        seed: scene.seed,
    }
}

/// Everything the static landscape layer depends on (not the sun, clouds
/// or sea state, so those edits stay instant).
fn landscape_key(scene: &Scene, revision: u64, quality: u8, dragging: bool) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |v: u64| hash = (hash ^ v).wrapping_mul(0x100_0000_01b3);
    let ptr = |a: Option<usize>| a.unwrap_or(0) as u64;
    mix(ptr(scene
        .grid
        .as_ref()
        .map(|g| std::sync::Arc::as_ptr(g) as usize)));
    mix(ptr(scene
        .backdrop
        .as_ref()
        .map(|g| std::sync::Arc::as_ptr(g) as usize)));
    mix(ptr(scene
        .habitat
        .as_ref()
        .map(|g| std::sync::Arc::as_ptr(g) as usize)));
    for imagery in [&scene.imagery, &scene.backdrop_imagery] {
        mix(ptr(imagery
            .as_ref()
            .map(|g| std::sync::Arc::as_ptr(g) as usize)));
    }
    for v in [
        scene.terrain as u64,
        scene.terrain_material as u64,
        quality as u64,
        scene.seed,
        scene.bottom_depth.to_bits(),
        scene.structure as u64,
        scene.structure_scale.to_bits(),
        scene.structure_position[0].to_bits(),
        scene.structure_position[1].to_bits(),
        scene.structure_position[2].to_bits(),
    ] {
        mix(v);
    }
    // Previews during a drag change the world without a scene edit; the
    // revision covers those.
    mix(if dragging { revision } else { 0 });
    hash
}

fn default_layout() -> DockState<Tab> {
    // Trials is a full workspace (setup, results, charts): it shares the
    // main area with the viewport rather than the short bottom dock.
    let mut dock = DockState::new(vec![
        Tab::Viewport,
        Tab::Globe,
        Tab::Trials,
        Tab::Seakeeping,
        Tab::Sar,
        Tab::Survey,
        Tab::Ship,
        Tab::Inspection,
        Tab::Spill,
    ]);
    let surface = dock.main_surface_mut();
    let [left, right] = surface.split_right(NodeIndex::root(), 0.76, vec![Tab::Outliner]);
    surface.split_below(right, 0.40, vec![Tab::Details]);
    surface.split_below(
        left,
        0.8,
        vec![Tab::Content, Tab::Log, Tab::Experiments, Tab::Sensors],
    );
    dock
}

impl TabViewer for State {
    type Tab = Tab;
    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Viewport => "  Viewport  ",
            Tab::Outliner => "  Outliner  ",
            Tab::Details => "  Details  ",
            Tab::Content => "  Content Browser  ",
            Tab::Log => "  Output Log  ",
            Tab::Globe => "  Earth  ",
            Tab::Experiments => "  Experiments  ",
            Tab::Sensors => "  Sensor Monitor  ",
            Tab::Trials => "  Trials  ",
            Tab::Seakeeping => "  Seakeeping  ",
            Tab::Sar => "  Search & Rescue  ",
            Tab::Survey => "  Survey  ",
            Tab::Ship => "  Ship Handling  ",
            Tab::Inspection => "  Inspection  ",
            Tab::Spill => "  Spill  ",
        }
        .into()
    }
    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::NULL.with(("pelagic-tab", *tab as u8))
    }
    fn is_closeable(&self, _: &Tab) -> bool {
        false
    }
    fn clear_background(&self, tab: &Tab) -> bool {
        !matches!(tab, Tab::Viewport | Tab::Globe)
    }
    fn scroll_bars(&self, tab: &Tab) -> [bool; 2] {
        match tab {
            Tab::Viewport
            | Tab::Globe
            | Tab::Content
            | Tab::Log
            | Tab::Details
            | Tab::Sensors
            | Tab::Trials
            | Tab::Seakeeping
            | Tab::Sar
            | Tab::Survey
            | Tab::Ship
            | Tab::Inspection
            | Tab::Spill => [false, false],
            _ => [false, true],
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Viewport => self.viewport(ui),
            Tab::Outliner => self.outliner(ui),
            Tab::Details => self.details(ui),
            Tab::Content => self.content(ui),
            Tab::Log => self.log_panel(ui),
            Tab::Experiments => self.experiments(ui),
            Tab::Sensors => self.sensor_monitor(ui),
            Tab::Trials => {
                self.tool_guide(ui, Tab::Trials);
                self.trials_panel(ui)
            }
            Tab::Seakeeping => {
                self.tool_guide(ui, Tab::Seakeeping);
                self.seakeeping_panel(ui)
            }
            Tab::Sar => {
                self.tool_guide(ui, Tab::Sar);
                self.sar_panel(ui)
            }
            Tab::Survey => {
                self.tool_guide(ui, Tab::Survey);
                self.survey_panel(ui)
            }
            Tab::Ship => {
                self.tool_guide(ui, Tab::Ship);
                self.ship_panel(ui)
            }
            Tab::Inspection => {
                self.tool_guide(ui, Tab::Inspection);
                self.inspection_panel(ui)
            }
            Tab::Spill => {
                self.tool_guide(ui, Tab::Spill);
                self.spill_panel(ui)
            }
            Tab::Globe => {
                if let Some(scene) = self.globe.ui(ui) {
                    self.open_scene(scene);
                    self.focus_viewport = true;
                }
                if let Some((conditions, source)) = self.globe.apply_sea.take() {
                    self.apply_forecast_sea(&conditions, &source);
                }
                if let Some((lat, lon)) = self.globe.spill_here.take() {
                    self.spill.set_position(lat, lon);
                    self.focus_spill = true;
                    self.log(format!(
                        "Oil spill: release point set to {lat:.4}°, {lon:.4}°."
                    ));
                }
                if let Some((lat, lon)) = self.globe.sar_here.take() {
                    self.sar.set_position(lat, lon);
                    self.focus_sar = true;
                    self.log(format!(
                        "Search and rescue: last known position set to {lat:.4}°, {lon:.4}°."
                    ));
                }
            }
        }
    }
}

impl State {
    fn log(&mut self, text: impl Into<String>) {
        self.model.notice = text.into();
        self.model.record_notice();
    }
    fn scene(&self) -> &Scene {
        &self.model.document.as_ref().expect("document").scene
    }
    fn apply(&mut self, next: Scene, message: &str) -> bool {
        self.view_dirty = true;
        match crate::editor::edit(&mut self.model, next, true) {
            Ok(()) => {
                if !message.is_empty() {
                    self.log(message);
                }
                true
            }
            Err(e) => {
                self.log(format!("Edit refused: {e}"));
                false
            }
        }
    }
    fn open_scene(&mut self, scene: Scene) {
        if !self.confirm_discard("opening another scene") {
            return;
        }
        self.cam = FlyCam::from_scene(&scene);
        self.live_on = real_place(&scene);
        self.live = None;
        self.live_at = None;
        self.forecast_ahead = None;
        self.flight = None;
        self.daylight = false;
        let name = scene.name.clone();
        crate::editor::install(&mut self.model, Document::new(scene));
        self.selection = Selection::None;
        self.view_dirty = true;
        self.log(format!("Opened {name} (unsaved)."));
    }
    fn open_path(&mut self, path: &str) {
        match Document::open(std::path::Path::new(path)) {
            Ok(doc) => {
                if !self.confirm_discard("opening another project") {
                    return;
                }
                self.cam = FlyCam::from_scene(&doc.scene);
                self.live_on = real_place(&doc.scene);
                self.live = None;
                self.live_at = None;
                self.forecast_ahead = None;
                self.flight = None;
                self.daylight = false;
                crate::editor::install(&mut self.model, doc);
                self.selection = Selection::None;
                self.view_dirty = true;
                self.log(format!("Opened {path}"));
            }
            Err(e) => self.log(format!("Could not open project: {e}")),
        }
    }
    fn editable(&self) -> bool {
        self.import.is_none()
            && self.model.playback.is_none()
            && self.model.session.as_ref().is_some_and(|s| {
                !s.running && s.world.tick() == 0 && s.display.is_none() && s.capture.is_none()
            })
    }
    fn selected_actor(&self) -> Option<Actor> {
        match self.selection {
            Selection::Actor(a) => Some(a),
            _ => None,
        }
    }
    fn actor_index(&self) -> Option<Option<usize>> {
        match self.selected_actor()? {
            Actor::Reference => Some(None),
            Actor::Entity(id) => self
                .scene()
                .entities
                .iter()
                .position(|e| e.id == id)
                .map(Some),
            Actor::Structure => None,
        }
    }
    fn select(&mut self, selection: Selection) {
        self.selection = selection;
        self.pending = None;
        // Keep the shared model in step for library/editor helpers.
        self.model.selection_empty = selection == Selection::None;
        self.model.selected_entity = None;
        self.model.inspect_structure = false;
        self.model.inspect_ocean = false;
        match selection {
            Selection::Actor(Actor::Entity(id)) => {
                self.model.inspector_page = 1;
                self.model.selected_entity = self.scene().entities.iter().position(|e| e.id == id);
            }
            Selection::Actor(Actor::Reference) => self.model.inspector_page = 1,
            Selection::Actor(Actor::Structure) => {
                self.model.inspector_page = 0;
                self.model.inspect_structure = true;
            }
            _ => {
                self.model.inspector_page = 0;
                self.model.inspect_ocean = true;
            }
        }
    }
    fn frame_selected(&mut self) {
        let target = match self.selected_actor().and_then(|a| a.position(self.scene())) {
            Some(p) => p,
            None => {
                let t = self.scene().target;
                Vec3::new(t[0], t[1], t[2])
            }
        };
        let size = self
            .selected_actor()
            .and_then(|a| a.entity(self.scene()))
            .map_or(8.0, |e| e.model.half.length() * 6.0)
            .max(3.0);
        self.cam.focus = size;
        self.cam.eye = target - self.cam.forward() * size;
        self.cam.speed = (size * 0.8).clamp(1.0, 60.0);
        self.view_dirty = true;
    }
    fn confirm_discard(&mut self, action: &str) -> bool {
        if self.unattended || !self.model.document.as_ref().is_some_and(|d| d.dirty()) {
            return true;
        }
        match native::ask_save(self.hwnd, action) {
            native::Answer::Save => self.save(false),
            native::Answer::Discard => true,
            native::Answer::Cancel => false,
        }
    }
    fn save(&mut self, save_as: bool) -> bool {
        let mut scene = self.scene().clone();
        let eye = self.cam.eye;
        let target = self.cam.target();
        let camera = [eye.x, eye.y, eye.z];
        let aim = [target.x, target.y, target.z];
        if scene.camera != camera || scene.target != aim {
            scene.camera = camera;
            scene.target = aim;
            let _ = crate::editor::edit(&mut self.model, scene, false);
        }
        let path = match (&self.model.project_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => native::pick_file(
                self.hwnd,
                true,
                "Save Pelagic project",
                "Pelagic project\0*.pelagic\0",
                "pelagic",
            ),
        };
        let Some(path) = path else {
            return false;
        };
        match crate::editor::save(&mut self.model, std::path::Path::new(&path)) {
            Ok(()) => {
                self.log(format!("Saved {path}"));
                true
            }
            Err(e) => {
                self.log(format!("Save failed: {e}"));
                false
            }
        }
    }
    fn history(&mut self, redo: bool) {
        self.pending = None;
        if let Err(e) = crate::editor::history(&mut self.model, redo) {
            self.log(e);
        }
        if let Selection::Actor(Actor::Entity(id)) = self.selection
            && !self.scene().entities.iter().any(|e| e.id == id)
        {
            self.select(Selection::None);
        }
        self.view_dirty = true;
    }
    fn start_import(&mut self) {
        if self.import.is_some() {
            return;
        }
        let Some(path) = native::pick_file(
            self.hwnd,
            false,
            "Import mesh (Wavefront OBJ)",
            "Wavefront OBJ\0*.obj\0",
            "obj",
        ) else {
            return;
        };
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                use std::io::Read;
                let mut text = String::new();
                std::fs::File::open(&path)
                    .map_err(|e| e.to_string())?
                    .take(8_388_609)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                let name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Mesh")
                    .to_string();
                let mesh =
                    pelagic::assets::Mesh::parse_obj(&text, &name).map_err(str::to_string)?;
                Ok((name, mesh))
            })();
            let _ = send.send(result);
        });
        self.import = Some(receive);
        self.log("Importing mesh…");
    }
    fn poll_import(&mut self) {
        let Some(rx) = &self.import else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        self.import = None;
        match result {
            Ok((name, mesh)) => {
                let selected = self.actor_index().flatten();
                match library::import(self.scene(), &name, mesh, selected) {
                    Ok((next, id, index)) => {
                        if self.apply(next, &format!("Imported {name}")) {
                            self.model.selected_library = Some(Item::Mesh(id));
                            if let Some(e) = self.scene().entities.get(index) {
                                let id = e.id;
                                self.select(Selection::Actor(Actor::Entity(id)));
                            }
                        }
                    }
                    Err(e) => self.log(format!("Import failed: {e}")),
                }
            }
            Err(e) => self.log(format!("Import failed: {e}")),
        }
    }
    fn place(&mut self, mesh: u64, material: Option<Item>) {
        let near = self.cam.eye + self.cam.forward() * self.cam.focus.clamp(4.0, 30.0);
        match library::place(self.scene(), mesh, material, [near.x, near.y, near.z]) {
            Ok((next, index)) => {
                if self.apply(next, "Placed mesh instance. Ctrl+Z removes it.") {
                    let id = self.scene().entities[index].id;
                    self.select(Selection::Actor(Actor::Entity(id)));
                }
            }
            Err(e) => self.log(e),
        }
    }
    fn delete_selected(&mut self) {
        let mut next = self.scene().clone();
        match self.selection {
            Selection::Actor(Actor::Entity(id)) => {
                next.entities.retain(|e| e.id != id);
                if next.sensors.entity == id {
                    next.sensors.entity = 0;
                }
            }
            Selection::Actor(Actor::Reference) => next.vehicle = false,
            Selection::Actor(Actor::Structure) => next.structure = false,
            _ => return,
        }
        if self.apply(next, "Deleted actor. Ctrl+Z restores it.") {
            self.select(Selection::None);
        }
    }
    fn duplicate_selected(&mut self) {
        let Some(Some(index)) = self.actor_index() else {
            return;
        };
        let mut next = self.scene().clone();
        let mut copy = next.entities[index].clone();
        copy.id = next.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        copy.position.x += copy.model.half.x * 2.5 + 0.5;
        copy.name = format!("{} copy", copy.name).chars().take(80).collect();
        let id = copy.id;
        next.entities.push(copy);
        if self.apply(next, "Duplicated actor.") {
            self.select(Selection::Actor(Actor::Entity(id)));
        }
    }
    fn run_scripts(&mut self) {
        if self.wait_world && !self.world_busy() {
            self.wait_world = false;
        }
        while self.job.is_none()
            && !self.wait_world
            && !self.globe.sea_loading()
            && !self.globe.building()
            && !self.globe.features_loading()
            && let Some(command) = self.script_queue.pop_front()
        {
            self.script(&command);
        }
    }
    fn script(&mut self, command: &str) {
        // Deterministic UI states for captures and automated checks.
        match command {
            "select-auv" => {
                if let Some(e) = self.scene().entities.iter().find(|e| e.asset.is_some()) {
                    let id = e.id;
                    self.select(Selection::Actor(Actor::Entity(id)));
                }
            }
            "frame" => self.frame_selected(),
            "land" => self.go_to_land(),
            "wait-world" => self.wait_world = true,
            "live:on" => {
                self.live_on = true;
                self.live_at = None;
            }
            "live:off" => self.live_on = false,
            "daylight" => self.daylight = true,
            other if other.starts_with("forecast:") => {
                let v = numbers(&other[9..]);
                if let Some(h) = v.first().map(|h| h.clamp(1.0, 192.0) as u32) {
                    let scene = self.scene();
                    let (lat, lon) = (
                        scene.provenance.origin_latitude,
                        scene.provenance.origin_longitude,
                    );
                    self.forecast_ahead = Some(h);
                    self.live_on = false;
                    let (send, receive) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let result =
                            crate::forecast::point(lat, lon, (h / 24 + 2).min(8)).map(|f| {
                                let unix = unix_now() as i64 + h as i64 * 3600;
                                let i = f.index_at(&pelagic::metocean::utc_hour(unix));
                                (f.hours[i].clone(), f.source.clone())
                            });
                        let _ = send.send(result);
                    });
                    self.live_sea = Some(receive);
                }
            }
            other if other.starts_with("speed:") => {
                if let Some(v) = numbers(&other[6..]).first() {
                    self.cam.speed = v.clamp(0.2, 20_000.0);
                }
            }
            other if other.starts_with("geo:") => {
                // lat, lon, height above sea (m, negative = depth), heading °, pitch °
                let v = numbers(&other[4..]);
                if v.len() == 5 && real_place(self.scene()) {
                    let scene = self.scene();
                    let geo = super::world::Geo {
                        lat: scene.provenance.origin_latitude,
                        lon: scene.provenance.origin_longitude,
                    };
                    let (x, y) = geo.to_xy(v[0], v[1]);
                    self.cam.eye = Vec3::new(x, y, v[2]);
                    self.cam.yaw = (90.0 - v[3]).to_radians();
                    self.cam.pitch = v[4].to_radians().clamp(-1.5, 1.5);
                    self.flight = None;
                    self.view_dirty = true;
                }
            }
            other if other.starts_with("pose:") => {
                // x, y, z (scene metres), heading °, pitch °
                let v = numbers(&other[5..]);
                if v.len() == 5 {
                    self.cam.eye = Vec3::new(v[0], v[1], v[2]);
                    self.cam.yaw = (90.0 - v[3]).to_radians();
                    self.cam.pitch = v[4].to_radians().clamp(-1.5, 1.5);
                    self.flight = None;
                    self.view_dirty = true;
                }
            }
            "select-vessel" => {
                // First surface vessel (body kind 1), for ship handling.
                match self.scene().entities.iter().find(|e| e.kind == 1) {
                    Some(e) => {
                        let id = e.id;
                        self.select(Selection::Actor(Actor::Entity(id)));
                    }
                    None => self.log(
                        "Ship handling needs a surface vessel: place one from the Content Browser, or open the Asset library demo.",
                    ),
                }
            }
            "select-reference" => {
                if !self.scene().vehicle {
                    let mut next = self.scene().clone();
                    next.vehicle = true;
                    self.apply(next, "Added the reference vehicle (an ROV) for this tool.");
                }
                if self.scene().vehicle {
                    self.select(Selection::Actor(Actor::Reference));
                }
            }
            "globe" => self.globe.focus = true,
            "satellite" => self.globe.set_satellite(true),
            "create" => self.globe.create_here(),
            "viewport" => self.focus_viewport = true,
            "experiments" => self.focus_experiments = true,
            "sensors" => self.focus_sensors = true,
            "trials" => self.focus_trials = true,
            "seakeeping" => self.focus_seakeeping = true,
            "sar" => self.focus_sar = true,
            "survey" => self.focus_survey = true,
            "ship" => self.focus_ship = true,
            "inspect" => self.focus_inspection = true,
            "inspect-run" => {
                self.focus_inspection = true;
                self.start_inspection(false);
            }
            "inspect-physics" => {
                self.focus_inspection = true;
                self.start_inspection(true);
            }
            "spill" => self.focus_spill = true,
            "spill-run" => {
                self.focus_spill = true;
                self.start_spill();
            }
            other if other.starts_with("spill-here:") => {
                let v: Vec<f64> = other[11..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if v.len() == 2 {
                    self.spill.set_position(v[0], v[1]);
                }
            }
            other if other.starts_with("spill-steady:") => {
                let v: Vec<f64> = other[13..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if v.len() == 4 {
                    self.spill.set_steady([v[0], v[1]], [v[2], v[3]]);
                }
            }
            other if other.starts_with("spill-hour:") => {
                if let Ok(h) = other[11..].parse::<f64>() {
                    self.spill.show_hour(h);
                }
            }
            "ship-autopilot" => self.ship_autopilot(true),
            "ship-suite" => {
                self.focus_ship = true;
                self.ship_suite();
            }
            other if other.starts_with("ship-start:") => {
                self.focus_ship = true;
                let choice = other[11..].parse::<usize>().ok();
                self.ship_start(choice);
            }
            other if other.starts_with("ship-run:") => {
                if let Ok(seconds) = other[9..].parse::<f64>() {
                    self.ship_advance(seconds);
                }
            }
            "survey-run" => {
                self.focus_survey = true;
                self.start_survey(false);
            }
            "survey-physics" => {
                self.focus_survey = true;
                self.start_survey(true);
            }
            other if other.starts_with("survey-set:") => {
                let what = other[11..].to_string();
                self.survey_script(&what);
            }
            "sar-run" => {
                self.focus_sar = true;
                self.start_sar();
            }
            other if other.starts_with("sar-here:") => {
                let v: Vec<f64> = other[9..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if v.len() == 2 {
                    self.sar.set_position(v[0], v[1]);
                }
            }
            other if other.starts_with("sar-steady:") => {
                let v: Vec<f64> = other[11..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if v.len() == 4 {
                    self.sar.set_steady([v[0], v[1]], [v[2], v[3]]);
                }
            }
            other if other.starts_with("sar-minute:") => {
                if let Ok(m) = other[11..].parse::<f64>() {
                    self.sar.show_search_minute(m);
                }
            }
            "seakeeping-run" => {
                self.focus_seakeeping = true;
                self.start_seakeeping();
            }
            "live-waves" => {
                self.globe.set_live_waves(true);
                self.globe.focus = true;
            }
            other if other.starts_with("sea-forecast:") => {
                let v: Vec<f64> = other[13..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if v.len() == 2 {
                    self.globe.request_forecast(v[0], v[1]);
                }
            }
            "apply-forecast" => {
                if let Some(f) = self.globe.forecast() {
                    let i = f.index_at(&crate::forecast::now_hour());
                    let (c, source) = (f.hours[i].clone(), f.source.clone());
                    self.apply_forecast_sea(&c, &source);
                }
            }
            "trials-run" => {
                self.trials.set_quick();
                self.focus_trials = true;
                self.start_trials();
            }
            other if other.starts_with("trials-chart:") => self.trials.show_chart(&other[13..]),
            other if other.starts_with("select:") => {
                let name = &other[7..];
                if let Some(e) = self.scene().entities.iter().find(|e| e.name == name) {
                    let id = e.id;
                    self.select(Selection::Actor(Actor::Entity(id)));
                }
            }
            other if other.starts_with("import:") => {
                let path = other[7..].to_string();
                self.import_vehicle_from(&path);
            }
            "sensor-camera" => self.sensor_camera = true,
            "record" => self.start_recording(),
            "stop-recording" => self.stop_recording(),
            "replay" => self.replay_recording(),
            other if other.starts_with("campaign:") => {
                if let Ok(runs) = other[9..].parse::<usize>() {
                    self.campaign.set_quick(runs);
                    self.start_campaign();
                }
            }
            other if other.starts_with("scrub:") => {
                if let Ok(index) = other[6..].parse::<usize>() {
                    self.scrub(index);
                }
            }
            other if other.starts_with("batch:") => {
                if let Ok(seconds) = other[6..].parse::<f64>() {
                    self.model.batch_seconds = seconds;
                    self.start_batch();
                }
            }
            other if other.starts_with("goto:") => {
                let v: Vec<f64> = other[5..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if (3..=5).contains(&v.len()) {
                    self.globe.set_view(v[0], v[1], v[2]);
                    self.globe.set_orientation(
                        v.get(3).copied().unwrap_or(0.0),
                        v.get(4).copied().unwrap_or(0.0),
                    );
                    self.globe.focus = true;
                }
            }
            "reefs" | "seagrass" | "names" => self.globe.script(command),
            other
                if other.starts_with("find:")
                    || other.starts_with("layer:")
                    || other.starts_with("vessel:")
                    || other.starts_with("place:") =>
            {
                self.globe.script(other)
            }
            "play" => {
                if let Some(s) = self.model.session.as_mut() {
                    s.running = true;
                }
            }
            other if other.starts_with("view:") => {
                let wanted = other[5..].to_lowercase();
                let scene = self.scene().clone();
                if let Some(v) = pelagic::landscape::viewpoints(
                    scene.grid.as_deref(),
                    scene.habitat.as_deref(),
                    scene.bottom_depth,
                )
                .into_iter()
                .find(|v| v.name.to_lowercase().contains(&wanted))
                {
                    let mut place = scene;
                    place.camera = v.eye;
                    place.target = v.target;
                    self.cam = FlyCam::from_scene(&place);
                    self.view_dirty = true;
                }
            }
            other if other.starts_with("cam:") => {
                let v = numbers(&other[4..]);
                if v.len() == 6 {
                    let mut scene = self.scene().clone();
                    scene.camera = [v[0], v[1], v[2]];
                    scene.target = [v[3], v[4], v[5]];
                    self.cam = FlyCam::from_scene(&scene);
                    self.view_dirty = true;
                }
            }
            other if other.starts_with("sun:") => {
                let v = numbers(&other[4..]);
                if v.len() == 2 {
                    let mut next = self.scene().clone();
                    next.sun_elevation = v[0];
                    next.sun_azimuth = v[1];
                    self.apply(next, "");
                }
            }
            other if other.starts_with("clouds:") => {
                let v = numbers(&other[7..]);
                if v.len() == 1 {
                    let mut next = self.scene().clone();
                    next.clouds = v[0];
                    self.apply(next, "");
                }
            }
            other if other.starts_with("sea:") => {
                // Hs (m), Tp (s)[, wind m/s[, direction °]]
                let v = numbers(&other[4..]);
                if (2..=4).contains(&v.len()) {
                    let mut next = self.scene().clone();
                    next.wave_height = v[0];
                    next.wave_period = v[1];
                    next.wind_speed = v.get(2).copied().unwrap_or(0.0);
                    if let Some(d) = v.get(3) {
                        next.wave_direction = *d;
                        next.wind_direction = *d;
                    }
                    next.bottom_depth = next.bottom_depth.max(next.wave_height * 4.5);
                    self.apply(next, "");
                }
            }
            other if other.starts_with("quality:") => {
                let v = numbers(&other[8..]);
                if v.len() == 1 && (0.0..=2.0).contains(&v[0]) {
                    let mut next = self.scene().clone();
                    next.quality = v[0] as u8;
                    self.apply(next, "");
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- chrome

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        let pressed = |k: Key| ui.input(|i| i.key_pressed(k));
        if ctrl && pressed(Key::S) {
            self.save(shift);
        }
        if ctrl && pressed(Key::Z) && !typing {
            self.history(false);
        }
        if ctrl && pressed(Key::Y) && !typing {
            self.history(true);
        }
        if ctrl && pressed(Key::O) {
            self.open_dialog();
        }
        if ctrl && pressed(Key::D) && !typing {
            self.duplicate_selected();
        }
        if typing || ctrl || self.flying {
            return;
        }
        if pressed(Key::Delete) {
            self.delete_selected();
        }
        if pressed(Key::F) {
            self.frame_selected();
        }
        // W/E/R pick tools for a selected actor; with nothing selected the
        // keys move the camera (camera_input).
        let selected = self.selected_actor().is_some();
        if selected && (pressed(Key::W) || pressed(Key::Q) && self.tool != Tool::Move) {
            self.tool = Tool::Move;
        }
        if selected && pressed(Key::E) {
            self.tool = Tool::Rotate;
        }
        if selected && pressed(Key::R) {
            self.tool = Tool::Size;
        }
        if pressed(Key::Space) {
            self.tool = match self.tool {
                Tool::Move => Tool::Rotate,
                Tool::Rotate => Tool::Size,
                _ => Tool::Move,
            };
        }
        if pressed(Key::Escape) {
            if self.drag.take().is_some() {
                if let Some(s) = self.model.session.as_mut() {
                    s.clear_preview();
                }
                self.view_dirty = true;
            } else {
                self.select(Selection::None);
            }
        }
    }
    fn open_dialog(&mut self) {
        if let Some(path) = native::pick_file(
            self.hwnd,
            false,
            "Open Pelagic project",
            "Pelagic project\0*.pelagic\0",
            "pelagic",
        ) {
            self.open_path(&path);
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.label(RichText::new("◆").color(theme::ACCENT).size(15.0));
                    ui.label(RichText::new("Pelagic").strong().color(theme::STRONG));
                    ui.add_space(10.0);
                    ui.menu_button("File", |ui| {
                        ui.menu_button("New scene", |ui| {
                            for (label, make) in [
                                ("Asset library demo", library::demo_scene as fn() -> Scene),
                                ("Marine robotics lab", Scene::robotics_demo),
                                ("Schooling ecology", Scene::ecology_demo),
                                ("Surface sea state", Scene::surface_demo),
                                ("Sensor experiment", Scene::sensor_demo),
                                ("Empty ocean", Scene::default),
                            ] {
                                if ui.button(label).clicked() {
                                    self.open_scene(make());
                                    ui.close();
                                }
                            }
                        });
                        if ui.button("Open…            Ctrl+O").clicked() {
                            self.open_dialog();
                            ui.close();
                        }
                        let recent = self.model.recent.clone();
                        ui.menu_button("Open recent", |ui| {
                            for path in recent {
                                if ui.button(&path).clicked() {
                                    self.open_path(&path);
                                    ui.close();
                                }
                            }
                        });
                        ui.separator();
                        if ui.button("Save               Ctrl+S").clicked() {
                            self.save(false);
                            ui.close();
                        }
                        if ui.button("Save as…    Ctrl+Shift+S").clicked() {
                            self.save(true);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Import mesh…").clicked() {
                            self.start_import();
                            ui.close();
                        }
                        if ui.button("Import vehicle definition…").clicked() {
                            self.import_vehicle();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Exit").clicked() && self.confirm_discard("closing Pelagic") {
                            self.model.about = true;
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Undo        Ctrl+Z").clicked() {
                            self.history(false);
                            ui.close();
                        }
                        if ui.button("Redo        Ctrl+Y").clicked() {
                            self.history(true);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Duplicate   Ctrl+D").clicked() {
                            self.duplicate_selected();
                            ui.close();
                        }
                        if ui.button("Delete      Del").clicked() {
                            self.delete_selected();
                            ui.close();
                        }
                    });
                    ui.menu_button("Window", |ui| {
                        if ui.button("Reset layout").clicked() {
                            self.reset_layout = true;
                            ui.close();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        ui.label(CONTROLS);
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let doc = self.model.document.as_ref().unwrap();
                        if doc.dirty() {
                            ui.label(RichText::new("● unsaved").small().color(theme::WARN));
                        }
                        ui.label(RichText::new(&doc.scene.name).color(theme::TEXT));
                    });
                });
            });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(10, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    theme::group(ui, |ui| {
                        if ui
                            .button("💾 Save")
                            .on_hover_text("Save (Ctrl+S)")
                            .clicked()
                        {
                            self.save(false);
                        }
                        if ui
                            .button("📂 Open")
                            .on_hover_text("Open a project (Ctrl+O)")
                            .clicked()
                        {
                            self.open_dialog();
                        }
                        if ui
                            .button("⬆ Import")
                            .on_hover_text("Import an OBJ mesh")
                            .clicked()
                        {
                            self.start_import();
                        }
                    });
                    theme::group(ui, |ui| {
                        let (undo, redo) = self
                            .model
                            .document
                            .as_ref()
                            .map_or((false, false), |d| (d.can_undo(), d.can_redo()));
                        let undo =
                            ui.add_enabled(undo, egui::Button::new(RichText::new("↶").size(17.0)));
                        super::a11y::name(&undo, "Undo");
                        if undo.on_hover_text("Undo (Ctrl+Z)").clicked() {
                            self.history(false);
                        }
                        let redo =
                            ui.add_enabled(redo, egui::Button::new(RichText::new("↷").size(17.0)));
                        super::a11y::name(&redo, "Redo");
                        if redo.on_hover_text("Redo (Ctrl+Y)").clicked() {
                            self.history(true);
                        }
                    });
                    theme::group(ui, |ui| {
                        for (tool, label, tip) in [
                            (Tool::Move, "✥ Move", "Translate (W)"),
                            (Tool::Rotate, "⟲ Rotate", "Rotate heading (E)"),
                            (Tool::Size, "⤢ Scale", "Uniform scale (R)"),
                        ] {
                            if ui
                                .selectable_label(self.tool == tool, label)
                                .on_hover_text(tip)
                                .clicked()
                            {
                                self.tool = tool;
                            }
                        }
                        let snap = egui::ComboBox::from_id_salt("snap")
                            .width(96.0)
                            .selected_text(self.snap.label())
                            .show_ui(ui, |ui| {
                                for snap in [Snap::Off, Snap::Fine, Snap::Normal, Snap::Coarse] {
                                    ui.selectable_value(&mut self.snap, snap, snap.label());
                                }
                            });
                        super::a11y::name(&snap.response, "Snap");
                    });
                    ui.add_space(12.0);
                    theme::group(ui, |ui| {
                        let running = self.model.session.as_ref().is_some_and(|s| s.running);
                        let play = if running {
                            egui::Button::new(
                                RichText::new("⏸ Pause").color(theme::ON_ACCENT).strong(),
                            )
                            .fill(theme::WARN)
                        } else {
                            theme::primary("▶ Play")
                        };
                        if ui
                            .add(play)
                            .on_hover_text("Run or pause the simulation")
                            .clicked()
                        {
                            self.pending = None;
                            if let Some(s) = self.model.session.as_mut() {
                                s.toggle();
                            }
                        }
                        if ui
                            .button("⏭ Step")
                            .on_hover_text("Advance one simulation step")
                            .clicked()
                            && let Some(s) = self.model.session.as_mut()
                        {
                            s.step();
                        }
                        if ui
                            .button("⏹ Reset")
                            .on_hover_text("Back to the initial state (keeps any recording)")
                            .clicked()
                        {
                            crate::editor::archive_run(&mut self.model);
                            let scene = self.scene().clone();
                            if let Some(s) = self.model.session.as_mut() {
                                s.reset_scene(&scene);
                            }
                            self.view_dirty = true;
                        }
                    });
                    if ui.available_size_before_wrap().x < 170.0 {
                        ui.end_row();
                    }
                    ui.vertical(|ui| {
                        let before = self.graphics;
                        let fps = 1000.0 / self.frame_ms.max(0.1);
                        let menu = ui.menu_button(self.graphics.label(), |ui| {
                            ui.set_min_width(290.0);
                            ui.label(RichText::new("Quality").small().color(theme::MUTED));
                            for (i, name) in graphics::PRESETS.iter().enumerate() {
                                let hint = [
                                    "Fastest: coarse sea grid, fewer corals",
                                    "Laptops with integrated graphics",
                                    "Fine sea grid, dense reefs",
                                    "Dedicated GPUs: finest sea, most life, up to 4K",
                                ][i];
                                ui.radio_value(&mut self.graphics.preset, i as u8, *name)
                                    .on_hover_text(hint);
                            }
                            ui.separator();
                            ui.checkbox(&mut self.graphics.vsync, "V-Sync")
                                .on_hover_text("Off: no frame-rate cap (as fast as the GPU can draw)");
                            ui.add(
                                egui::Slider::new(&mut self.graphics.scale, 0.5..=2.0)
                                    .text("render scale")
                                    .fixed_decimals(2),
                            )
                            .on_hover_text("Above 1 renders more pixels than the screen (sharper); below 1 is faster");
                            ui.checkbox(&mut self.graphics.animate_paused, "Animate the sea while paused")
                                .on_hover_text("Visual only: waves keep moving, bodies keep their paused pose");
                            ui.separator();
                            ui.label(
                                RichText::new(format!("{} Â· {fps:.0} fps", self.adapter))
                                    .small()
                                    .color(theme::MUTED),
                            );
                        });
                        super::a11y::name(&menu.response, "Graphics settings");
                        if self.graphics != before {
                            self.graphics = self.graphics.sanitized();
                            if let Err(error) = self.graphics.save(&crate::editor::storage(&self.model)) {
                                self.log(format!("Graphics preferences: {error}"));
                            }
                            self.view_dirty = true;
                        }
                    });
                });
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let session = self.model.session.as_ref().unwrap();
                    let (colour, text) = match (&session.display, self.model.playback) {
                        (Some(shown), Some(sample)) => (
                            theme::ACCENT,
                            format!(
                                "Recorded playback · sample {sample} · t = {:.2} s",
                                shown.time
                            ),
                        ),
                        _ => (
                            if session.running {
                                theme::GOOD
                            } else {
                                theme::MUTED
                            },
                            format!(
                                "{} · t = {:.2} s · tick {}",
                                if session.running {
                                    "Simulating"
                                } else {
                                    "Paused"
                                },
                                session.world.time(),
                                session.world.tick()
                            ),
                        ),
                    };
                    let recording = session.capture.as_ref().map(|r| r.frames.len());
                    theme::chip(ui, colour, &text);
                    if let Some(samples) = recording {
                        theme::chip(ui, theme::BAD, &format!("REC {samples}"));
                    }
                    if self.job.is_some() {
                        theme::chip(ui, theme::WARN, &self.job_status.clone());
                    }
                    let notice = self
                        .error
                        .clone()
                        .unwrap_or_else(|| self.model.notice.clone());
                    let refused = notice.contains("refused") || notice.contains("failed");
                    ui.label(RichText::new(notice).small().color(if refused {
                        theme::BAD
                    } else {
                        theme::MUTED
                    }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(&self.adapter).small().color(theme::MUTED));
                        ui.label(
                            RichText::new(format!(
                                "{:.0} fps · {:.1} ms geometry",
                                1000.0 / self.frame_ms.max(0.1),
                                self.build_ms
                            ))
                            .small()
                            .color(theme::MUTED),
                        );
                    });
                });
            });
    }

    // --------------------------------------------------------------- viewport

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        super::a11y::name(&response, "3D viewport");
        let ppp = ui.ctx().pixels_per_point();
        let mut w = (rect.width() * ppp).max(64.0);
        let mut h = (rect.height() * ppp).max(64.0);
        self.viewport_visible = true;
        w *= self.graphics.scale;
        h *= self.graphics.scale;
        let budget = self.graphics.pixel_budget() / (w * h);
        if budget < 1.0 {
            w *= budget.sqrt();
            h *= budget.sqrt();
        }
        let px = ((w as u32).min(8192), (h as u32).min(8192));
        if px != self.viewport_px {
            self.viewport_px = px;
            self.view_dirty = true;
        }
        if self.viewport_rect != rect {
            self.viewport_rect = rect;
            self.view_dirty = true;
        }
        let painter = ui.painter_at(rect);
        if let Some(id) = self.viewport_tex {
            painter.image(
                id,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if self.sensor_camera {
            let painter = ui.painter_at(rect);
            painter.text(
                rect.left_top() + Vec2::new(10.0, 8.0),
                Align2::LEFT_TOP,
                "Sensor camera · view only (click or drag to return to the editor camera)",
                FontId::proportional(12.0),
                Color32::from_rgb(235, 190, 80),
            );
            if response.clicked() || response.drag_started() || response.secondary_clicked() {
                self.sensor_camera = false;
                self.view_dirty = true;
            }
            return;
        }
        self.camera_input(ui, &response);
        let camera = viewport_edit::Camera::new(
            [rect.left(), rect.top(), rect.width(), rect.height()],
            self.cam.eye,
            self.cam.target(),
            Vec3::new(0.0, 0.0, 1.0),
            FOV,
        );
        self.selection_overlay(&painter, camera);
        let gizmo = self.gizmo(camera);
        let hover_handle = response
            .hover_pos()
            .and_then(|p| gizmo.as_ref().and_then(|g| g.hit([p.x, p.y])));
        if let Some(g) = &gizmo {
            draw_gizmo(
                &painter,
                g,
                hover_handle.or(self.drag.as_ref().map(|d| d.0.handle)),
            );
        }
        // Gizmo drag / click selection.
        let alt = ui.input(|i| i.modifiers.alt);
        if response.drag_started_by(PointerButton::Primary)
            && !alt
            && let (Some(handle), Some(pos)) = (hover_handle, response.interact_pointer_pos())
            && let Some(actor) = self.selected_actor()
        {
            if self.editable() {
                if let Some(d) = Drag::begin(
                    self.scene().clone(),
                    actor,
                    handle,
                    camera,
                    [pos.x, pos.y],
                    self.snap,
                ) {
                    self.drag = Some((d, None));
                }
            } else {
                self.log("Press ⏹ Reset to edit initial transforms (the simulation has advanced).");
            }
        }
        if let Some((drag, last)) = self.drag.as_mut()
            && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
            && response.dragged_by(PointerButton::Primary)
        {
            match drag.update([pos.x, pos.y]) {
                Ok(next) => {
                    if let Some(s) = self.model.session.as_mut() {
                        let _ = s.preview_scene(&next);
                    }
                    *last = Some(next);
                    self.view_dirty = true;
                }
                Err(e) => self.model.notice = e,
            }
        }
        // Left-drag on open space looks around.
        if response.dragged_by(PointerButton::Primary)
            && !alt
            && self.drag.is_none()
            && hover_handle.is_none()
        {
            self.looking = true;
        }
        if response.drag_stopped_by(PointerButton::Primary)
            && let Some((drag, last)) = self.drag.take()
        {
            if let Some(s) = self.model.session.as_mut() {
                s.clear_preview();
            }
            if let Some(next) = last
                && let Err(e) = crate::editor::commit_transform(&mut self.model, &drag.base, next)
            {
                self.log(e);
            }
            self.view_dirty = true;
        }
        if response.clicked_by(PointerButton::Primary)
            && !alt
            && hover_handle.is_none()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let session = self.model.session.as_ref().unwrap();
            let picked = viewport_edit::pick(
                camera,
                [pos.x, pos.y],
                &session.view_snapshot(),
                session.view_ocean(),
            );
            self.select(picked.map_or(Selection::None, Selection::Actor));
        }
        self.quick_bar(ui, rect);
        self.hud(ui, rect, &painter, camera);
        painter.text(
            rect.left_bottom() + Vec2::new(10.0, -8.0),
            Align2::LEFT_BOTTOM,
            "RMB + WASD: fly · Wheel: zoom · Alt + drag: orbit",
            FontId::proportional(11.0),
            Color32::from_white_alpha(150),
        );
        axis_triad(&painter, rect, &self.cam);
    }

    /// Real scenes saved with satellite colours from masked tiles (open-water
    /// fill over small islands) get fresh colours once.
    fn poll_imagery(&mut self) {
        let (real, place) = {
            let scene = self.scene();
            (
                real_place(scene),
                (
                    scene.provenance.origin_latitude,
                    scene.provenance.origin_longitude,
                ),
            )
        };
        if !real {
            return;
        }
        if self.imagery_checked != Some(place) {
            self.imagery_checked = Some(place);
            let job = {
                let scene = self.scene();
                scene.grid.clone().and_then(|grid| {
                    // Land nodes whose stored colour looks like open water.
                    let mut land = 0usize;
                    let mut watery = 0usize;
                    for j in (0..grid.rows).step_by(2) {
                        for i in (0..grid.columns).step_by(2) {
                            if grid.elevations[j * grid.columns + i] <= 1.0 {
                                continue;
                            }
                            land += 1;
                            let (x, y) = (
                                grid.origin[0] + i as f64 * grid.spacing,
                                grid.origin[1] + j as f64 * grid.spacing,
                            );
                            let c = scene.imagery.as_deref().and_then(|m| m.linear(x, y));
                            if c.is_none_or(|c| c[2] > c[1] && c[2] > c[0] * 1.5) {
                                watery += 1;
                            }
                        }
                    }
                    (land > 10 && watery * 2 > land).then(|| (grid, scene.backdrop.clone()))
                })
            };
            self.imagery_rx = job.map(|(grid, backdrop)| {
                let (send, receive) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    let _ = send.send(super::globe::scene_imagery(
                        place.0,
                        place.1,
                        &grid,
                        backdrop.as_deref(),
                    ));
                });
                receive
            });
        }
        if let Some(rx) = &self.imagery_rx
            && let Ok((imagery, backdrop)) = rx.try_recv()
        {
            self.imagery_rx = None;
            let mut next = self.scene().clone();
            if let Some(i) = imagery {
                next.imagery = Some(std::sync::Arc::new(i));
            }
            if let Some(b) = backdrop {
                next.backdrop_imagery = Some(std::sync::Arc::new(b));
            }
            if next != *self.scene() {
                self.apply(
                    next,
                    "Satellite colours refreshed: small islands now use the 2024 Sentinel-2 mosaic.",
                );
                self.trees_at = None;
            }
        }
    }

    /// Terrain, species, live conditions or a camera flight still loading.
    fn world_busy(&self) -> bool {
        let real = self.model.document.is_some() && real_place(self.scene());
        (real
            && (self.world.is_none()
                || self.fauna_cell.is_none()
                || (self.live_on && self.live.is_none() && self.live_at.is_none())))
            || self.world.as_ref().is_some_and(|w| w.loading())
            || self.fauna_rx.is_some()
            || self.live_rx.is_some()
            || self.live_sea.is_some()
            || self.imagery_rx.is_some()
            || self.flight.is_some()
    }

    /// Live conditions: applies a finished request and asks again every
    /// 10 minutes while Live is on.
    fn poll_live(&mut self) {
        if let Some(rx) = &self.live_rx
            && let Ok(result) = rx.try_recv()
        {
            self.live_rx = None;
            self.live_at = Some(Instant::now());
            match result {
                Ok(live) => {
                    let scene = self.scene().clone();
                    let mut next = scene.clone();
                    let (conditions, source) = live.conditions();
                    let mut notes = pelagic::metocean::apply(&mut next, &conditions, &source);
                    if next.validate().is_err() {
                        next = scene.clone();
                        notes
                            .push("Live sea does not fit this scene's depth; kept the sea.".into());
                    }
                    if let Some(cover) = live.cloud_cover() {
                        next.clouds = cover.clamp(0.0, 1.0);
                    }
                    // The real sun now, unless daytime was asked for at night.
                    let unix = unix_now() as i64;
                    let (day, mut hours) = super::globe::utc_day_hours(unix);
                    let (lat, lon) = (
                        scene.provenance.origin_latitude,
                        scene.provenance.origin_longitude,
                    );
                    if self.daylight {
                        hours = (10.0 - lon / 15.0).rem_euclid(24.0);
                    }
                    let (e, a) = pelagic_render_gpu::sky::solar_position(lat, lon, day, hours);
                    next.sun_elevation = e.clamp(-6.0, 90.0);
                    next.sun_azimuth = a;
                    self.time_of_day = (hours + lon / 15.0).rem_euclid(24.0);
                    for note in notes {
                        self.log(note);
                    }
                    if next != scene {
                        self.apply(next, &format!("{source}."));
                    }
                    self.live = Some(live);
                }
                Err(e) => self.log(format!("Live conditions unavailable: {e}")),
            }
        }
        let due = self.live_at.is_none_or(|t| t.elapsed().as_secs() >= 600);
        if self.live_on && due && self.live_rx.is_none() && real_place(self.scene()) {
            let scene = self.scene();
            let geo = super::world::Geo {
                lat: scene.provenance.origin_latitude,
                lon: scene.provenance.origin_longitude,
            };
            let (lat, lon) = geo.to_latlon(self.cam.eye.x, self.cam.eye.y);
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = send.send(super::live::fetch(lat, lon));
            });
            self.live_rx = Some(receive);
            self.live_at = Some(Instant::now());
        }
    }

    /// Navigation display over the viewport: position, depth or altitude,
    /// speed, a compass with the nearest land, ship names, the animals
    /// around and controls (speed, go to land, surface or dive).
    fn hud(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        painter: &egui::Painter,
        camera: viewport_edit::Camera,
    ) {
        if rect.width() < 420.0 || self.model.document.is_none() {
            return;
        }
        let scene = self.scene().clone();
        let real = real_place(&scene);
        let geo = super::world::Geo {
            lat: scene.provenance.origin_latitude,
            lon: scene.provenance.origin_longitude,
        };
        let eye = self.cam.eye;
        let forward = self.cam.forward();
        let heading = forward.x.atan2(forward.y).to_degrees().rem_euclid(360.0);
        let pitch = self.cam.pitch.to_degrees();
        // Nearest land, refreshed while moving.
        if self.land_at.is_none_or(|t| t.elapsed().as_secs_f64() > 1.5) {
            self.land_at = Some(Instant::now());
            self.nearest_land = self.find_land(eye);
        }
        let ground = self.ground(eye.x, eye.y);
        let text = Color32::from_white_alpha(225);
        let dim = Color32::from_white_alpha(150);
        let panel = Color32::from_black_alpha(140);
        // Position and motion (top left).
        let mut lines = vec![];
        if real {
            let (lat, lon) = geo.to_latlon(eye.x, eye.y);
            lines.push(format!(
                "{:.4}° {}  {:.4}° {}",
                lat.abs(),
                if lat >= 0.0 { "N" } else { "S" },
                lon.abs(),
                if lon >= 0.0 { "E" } else { "W" }
            ));
        }
        let surface = 0.0;
        if eye.z < surface {
            lines.push(format!(
                "Depth {:.1} m · seabed {:.1} m below",
                -eye.z,
                (eye.z - ground).max(0.0)
            ));
        } else if ground > 0.0 {
            lines.push(format!(
                "Altitude {:.0} m · {:.0} m above the ground",
                eye.z,
                eye.z - ground
            ));
        } else {
            lines.push(format!(
                "Altitude {:.1} m · sea {:.0} m deep below",
                eye.z, -ground
            ));
        }
        let v = if self.flight.is_some() {
            self.cam.speed
        } else {
            self.velocity.length()
        };
        lines.push(format!(
            "Speed {:.1} m/s · {:.0} kn · {:.0} km/h   (set {:.0} m/s)",
            v,
            v * 1.943_844,
            v * 3.6,
            self.cam.speed
        ));
        lines.push(format!(
            "Heading {:.0}° {} · pitch {:+.0}°",
            heading,
            compass_point(heading),
            pitch
        ));
        if let Some(world) = &self.world
            && world.loading()
        {
            lines.push("Loading terrain around you…".to_string());
        }
        let font = FontId::proportional(12.0);
        let galley: Vec<_> = lines
            .iter()
            .map(|l| painter.layout_no_wrap(l.clone(), font.clone(), text))
            .collect();
        let width = galley.iter().map(|g| g.size().x).fold(0.0, f32::max) + 16.0;
        let height = galley.len() as f32 * 16.0 + 10.0;
        // Bottom left, above the animals line and the key hints.
        let top_left = rect.left_bottom() + Vec2::new(64.0, -50.0 - height);
        painter.rect_filled(
            Rect::from_min_size(top_left, Vec2::new(width, height)),
            6.0,
            panel,
        );
        for (i, g) in galley.into_iter().enumerate() {
            painter.galley(top_left + Vec2::new(8.0, 5.0 + i as f32 * 16.0), g, text);
        }
        // Compass tape (top centre, under the quick bar).
        let tape_w = 340.0f32.min(rect.width() - 40.0);
        let tape_top = (rect.top() + 8.0).max(self.quick_bottom + 6.0);
        let tape = Rect::from_center_size(
            Pos2::new(rect.center().x, tape_top + 17.0),
            Vec2::new(tape_w, 34.0),
        );
        painter.rect_filled(tape, 6.0, panel);
        let span = 120.0f32;
        let x_of = |bearing: f64| {
            let d = ((bearing - heading + 540.0).rem_euclid(360.0) - 180.0) as f32;
            (d.abs() <= span * 0.5).then(|| tape.center().x + d / span * tape_w)
        };
        for b in (0..360).step_by(5) {
            let Some(x) = x_of(b as f64) else { continue };
            let major = b % 45 == 0;
            let len = if major {
                10.0
            } else if b % 15 == 0 {
                6.0
            } else {
                3.0
            };
            painter.line_segment(
                [
                    Pos2::new(x, tape.bottom() - 2.0),
                    Pos2::new(x, tape.bottom() - 2.0 - len),
                ],
                Stroke::new(1.0, dim),
            );
            if major {
                painter.text(
                    Pos2::new(x, tape.top() + 9.0),
                    Align2::CENTER_CENTER,
                    compass_point(b as f64),
                    FontId::proportional(if b % 90 == 0 { 12.0 } else { 10.0 }),
                    if b == 0 {
                        Color32::from_rgb(240, 110, 90)
                    } else {
                        text
                    },
                );
            } else if b % 30 == 0 {
                painter.text(
                    Pos2::new(x, tape.top() + 9.0),
                    Align2::CENTER_CENTER,
                    format!("{b}"),
                    FontId::proportional(9.0),
                    dim,
                );
            }
        }
        painter.line_segment(
            [
                Pos2::new(tape.center().x, tape.top()),
                Pos2::new(tape.center().x, tape.bottom()),
            ],
            Stroke::new(2.0, Color32::from_rgb(255, 200, 80)),
        );
        painter.text(
            Pos2::new(tape.center().x, tape.bottom() + 9.0),
            Align2::CENTER_CENTER,
            format!("{heading:03.0}° {}", compass_point(heading)),
            FontId::proportional(11.0),
            Color32::from_rgb(255, 210, 110),
        );
        let land_label = self.nearest_land.map(|(d, x, y)| {
            let bearing = (x - eye.x).atan2(y - eye.y).to_degrees().rem_euclid(360.0);
            (d, bearing)
        });
        if let Some((d, bearing)) = land_label {
            let green = Color32::from_rgb(120, 220, 130);
            let label = format!("Land {:.1} km", d / 1000.0);
            match x_of(bearing) {
                Some(x) => {
                    let y = tape.bottom() - 1.0;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            Pos2::new(x, y - 9.0),
                            Pos2::new(x - 5.0, y),
                            Pos2::new(x + 5.0, y),
                        ],
                        green,
                        Stroke::NONE,
                    ));
                    painter.text(
                        Pos2::new(x, tape.bottom() + 22.0),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(10.0),
                        green,
                    );
                }
                None => {
                    let right = ((bearing - heading).rem_euclid(360.0)) < 180.0;
                    let x = if right {
                        tape.right() + 6.0
                    } else {
                        tape.left() - 6.0
                    };
                    painter.text(
                        Pos2::new(x, tape.center().y),
                        if right {
                            Align2::LEFT_CENTER
                        } else {
                            Align2::RIGHT_CENTER
                        },
                        if right {
                            format!("{label} ▶")
                        } else {
                            format!("◀ {label}")
                        },
                        FontId::proportional(10.0),
                        green,
                    );
                }
            }
        }
        // Night at a live place: say so, and offer daytime light.
        let sun = self.env_preview.map_or(scene.sun_elevation, |p| p.0);
        let mut by_day = None;
        if real && self.live_on && (sun < -1.0 || self.daylight) {
            let local = self.live.as_ref().map_or(String::new(), |l| {
                l.local_time.split(' ').nth(1).unwrap_or("").to_string()
            });
            let text = if self.daylight {
                format!(
                    "It is night here ({local} local). Live sea and weather, shown in daylight."
                )
            } else {
                format!("Night here now ({local} local): the sea is dark, as it really is.")
            };
            egui::Area::new(egui::Id::NULL.with("night-banner"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.right_bottom() + Vec2::new(-8.0, -66.0))
                .pivot(Align2::RIGHT_BOTTOM)
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgba_unmultiplied(20, 30, 60, 200))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let label = if self.daylight {
                                    "Daylight preview · use real sun"
                                } else {
                                    "See it by day"
                                };
                                if ui.button(label).on_hover_text(text).clicked() {
                                    by_day = Some(!self.daylight);
                                }
                            });
                        });
                });
        }
        if let Some(day) = by_day {
            self.daylight = day;
            let lon = scene.provenance.origin_longitude;
            let (d, now) = super::globe::utc_day_hours(unix_now() as i64);
            let hours = if day {
                (10.0 - lon / 15.0).rem_euclid(24.0)
            } else {
                now
            };
            let (e, a) = pelagic_render_gpu::sky::solar_position(
                scene.provenance.origin_latitude,
                lon,
                d,
                hours,
            );
            let mut next = scene.clone();
            next.sun_elevation = e.clamp(-6.0, 90.0);
            next.sun_azimuth = a;
            self.time_of_day = (hours + lon / 15.0).rem_euclid(24.0);
            self.apply(
                next,
                if day {
                    "Daylight view of the live sea."
                } else {
                    "Real sun."
                },
            );
        }
        // Ship names near the camera.
        let clock = unix_now();
        for ship in &self.ships {
            let minutes = ((clock - ship.time as f64) / 60.0).clamp(0.0, 10.0);
            let (la, lo) = crate::ais::dead_reckon(ship.lat, ship.lon, ship.sog, ship.cog, minutes);
            let (x, y) = geo.to_xy(la, lo);
            let p = Vec3::new(x, y, ship.length * 0.12 + 3.0);
            let d = (p - eye).length();
            if d > 12_000.0 {
                continue;
            }
            if let Some([sx, sy]) = camera.project(p)
                && rect.contains(Pos2::new(sx, sy))
            {
                painter.text(
                    Pos2::new(sx, sy - 4.0),
                    Align2::CENTER_BOTTOM,
                    format!(
                        "{} · {} · {:.1} kn · {:.1} km",
                        ship.name,
                        ship.category,
                        ship.sog,
                        d / 1000.0
                    ),
                    FontId::proportional(10.5),
                    Color32::from_rgb(255, 225, 150),
                );
            }
        }
        // Animals around (bottom left, above the hints).
        if rect.width() >= 900.0 && eye.z < 30.0 && !self.fauna.species.is_empty() {
            let near = self.fauna.nearby(eye, 35.0);
            if !near.is_empty() {
                let names: Vec<String> = near
                    .iter()
                    .take(4)
                    .map(|(i, n)| {
                        let sp = &self.fauna.species[*i];
                        if *n > 1 {
                            format!("{} ×{n}", sp.common)
                        } else {
                            sp.common.clone()
                        }
                    })
                    .collect();
                painter.text(
                    rect.left_bottom() + Vec2::new(70.0, -28.0),
                    Align2::LEFT_BOTTOM,
                    format!("Around you: {}", names.join(" · ")),
                    FontId::proportional(12.0),
                    Color32::from_rgb(170, 225, 235),
                );
            }
        }
        // Controls (bottom right).
        let mut speed = self.cam.speed;
        let mut action: Option<&str> = None;
        let land = land_label;
        let far_from_scene = real && eye.x.hypot(eye.y) > 6_000.0;
        egui::Area::new(egui::Id::NULL.with("viewport-nav"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.right_bottom() + Vec2::new(-8.0, -26.0))
            .pivot(Align2::RIGHT_BOTTOM)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(Color32::from_black_alpha(150))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.menu_button("Navigate", |ui| {
                        ui.set_max_width((rect.width() - 24.0).min(360.0));
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Speed");
                            let slider = ui
                                .add(
                                    egui::Slider::new(&mut speed, 0.5..=20_000.0)
                                        .logarithmic(true)
                                        .max_decimals(1)
                                        .suffix(" m/s"),
                                )
                                .on_hover_text("Camera speed. Shift moves 5× faster; + and − double or halve it; the mouse wheel while holding the right button also changes it.");
                            super::a11y::name(&slider, "Camera speed");
                            let current = SPEEDS
                                .iter()
                                .min_by(|a, b| {
                                    (a.1 / speed).ln().abs().total_cmp(&(b.1 / speed).ln().abs())
                                })
                                .map_or("Speed", |p| p.0);
                            egui::ComboBox::from_id_salt("speed-presets")
                                .selected_text(current)
                                .width(96.0)
                                .show_ui(ui, |ui| {
                                    for (name, v) in SPEEDS {
                                        if ui
                                            .selectable_label(name == current, format!("{name} · {v} m/s"))
                                            .clicked()
                                        {
                                            speed = v;
                                        }
                                    }
                                });
                        });
                        ui.horizontal_wrapped(|ui| {
                            let land_text = match land {
                                Some((d, b)) => format!("⛰ Go to land · {:.1} km {}", d / 1000.0, compass_point(b)),
                                None => "⛰ Go to land".to_string(),
                            };
                            let go = ui
                                .add_enabled(land.is_some(), egui::Button::new(land_text))
                                .on_hover_text("Fly to the nearest coast and look at it")
                                .on_disabled_hover_text("No land in the loaded terrain yet (real places load terrain around the camera)");
                            super::a11y::name(&go, "Go to land");
                            if go.clicked() {
                                action = Some("land");
                            }
                            if eye.z < 0.0 {
                                if ui.button("⬆ Surface").on_hover_text("Rise to just above the water").clicked() {
                                    action = Some("surface");
                                }
                            } else if ui
                                .add_enabled(ground < -1.5, egui::Button::new("⬇ Dive"))
                                .on_hover_text("Go under the surface here")
                                .clicked()
                            {
                                action = Some("dive");
                            }
                            if ui.button("⟲ Scene").on_hover_text("Back to the scene's own viewpoint").clicked() {
                                action = Some("home");
                            }
                            if far_from_scene
                                && ui
                                    .button("Build detailed scene here")
                                    .on_hover_text("Rebuild the scene at this spot: 20 m seabed, reefs, seagrass and satellite land (as Explore in 3D on the Earth tab)")
                                    .clicked()
                            {
                                action = Some("build");
                            }
                        });
                        });
                    });
            });
        if (speed - self.cam.speed).abs() > 1e-9 {
            self.cam.speed = speed.clamp(0.2, 20_000.0);
        }
        match action {
            Some("land") => self.go_to_land(),
            Some("surface") => {
                let to = Vec3::new(eye.x, eye.y, 2.5);
                self.fly_to(
                    to,
                    to + Vec3::new(forward.x, forward.y, 0.0) * 50.0 + Vec3::new(0.0, 0.0, -2.0),
                );
            }
            Some("dive") => {
                let depth = (-ground * 0.4).clamp(1.5, 15.0);
                let to = Vec3::new(eye.x, eye.y, -depth);
                self.fly_to(
                    to,
                    to + Vec3::new(forward.x, forward.y, 0.0) * 30.0 - Vec3::new(0.0, 0.0, 2.0),
                );
            }
            Some("home") => {
                let home = FlyCam::from_scene(&scene);
                self.fly_to(home.eye, home.target());
            }
            Some("build") => {
                let (lat, lon) = geo.to_latlon(eye.x, eye.y);
                self.globe.build_here(lat, lon);
                self.log(format!(
                    "Building a detailed scene at {lat:.4}°, {lon:.4}°… (it opens when ready)"
                ));
            }
            _ => {}
        }
    }

    /// Ground elevation under a scene point (scene grid, backdrop, streamed
    /// world).
    fn ground(&self, x: f64, y: f64) -> f64 {
        let Some(session) = self.model.session.as_ref() else {
            return -40.0;
        };
        ground_at(
            self.scene(),
            session.view_ocean(),
            self.world.as_ref(),
            x,
            y,
        )
    }

    /// Flies the camera smoothly to `eye`, looking at `target`.
    fn fly_to(&mut self, eye: Vec3, target: Vec3) {
        let d = target - eye;
        let len = d.length().max(0.1);
        let yaw = d.y.atan2(d.x);
        // Turn the short way round.
        let mut from_yaw = self.cam.yaw;
        while yaw - from_yaw > std::f64::consts::PI {
            from_yaw += std::f64::consts::TAU;
        }
        while from_yaw - yaw > std::f64::consts::PI {
            from_yaw -= std::f64::consts::TAU;
        }
        let distance = (eye - self.cam.eye).length();
        self.flight = Some(Flight {
            from: self.cam.eye,
            to: eye,
            yaw: (from_yaw, yaw),
            pitch: (self.cam.pitch, (d.z / len).clamp(-1.0, 1.0).asin()),
            start: Instant::now(),
            seconds: (1.2 + distance.max(1.0).log10() * 0.8).clamp(1.2, 6.0),
        });
        self.cam.focus = len;
        self.velocity = Vec3::ZERO;
        self.view_dirty = true;
    }

    /// Flies to the nearest land: just off its shore, low, looking at it.
    fn go_to_land(&mut self) {
        let eye = self.cam.eye;
        let Some((d, x, y)) = self.nearest_land.or_else(|| self.find_land(eye)) else {
            self.log("No land found yet within the loaded terrain (about 650 km). Fly or rise higher and try again.");
            return;
        };
        let dir = Vec3::new(x - eye.x, y - eye.y, 0.0);
        let dir = if dir.length() > 1.0 {
            dir.normalized()
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let ground = self.ground(x, y).max(0.0);
        let stand = (d * 0.02).clamp(250.0, 1_500.0);
        let at = Vec3::new(x, y, 0.0) - dir * stand;
        let height = (ground * 0.6 + 25.0).clamp(25.0, 600.0);
        let eye_to = Vec3::new(at.x, at.y, self.ground(at.x, at.y).max(0.0) + height);
        self.fly_to(eye_to, Vec3::new(x, y, ground + 5.0));
        self.cam.speed = self.cam.speed.min(25.0);
        self.log(format!(
            "Flying to the nearest land, {:.1} km {} of you.",
            d / 1000.0,
            compass_point(dir.x.atan2(dir.y).to_degrees())
        ));
    }

    /// Nearest land to a point: the scene grids first, then the streamed world.
    fn find_land(&self, p: Vec3) -> Option<(f64, f64, f64)> {
        let scene = self.scene();
        let mut best: Option<(f64, f64, f64)> = None;
        for g in [scene.grid.as_deref(), scene.backdrop.as_deref()]
            .into_iter()
            .flatten()
        {
            let step = if g.columns > 300 { 2 } else { 1 };
            for j in (0..g.rows).step_by(step) {
                for i in (0..g.columns).step_by(step) {
                    if g.elevations[j * g.columns + i] > 1.0 {
                        let (x, y) = (
                            g.origin[0] + i as f64 * g.spacing,
                            g.origin[1] + j as f64 * g.spacing,
                        );
                        let d = (x - p.x).hypot(y - p.y);
                        if best.is_none_or(|b| d < b.0) {
                            best = Some((d, x, y));
                        }
                    }
                }
            }
        }
        let far = self.world.as_ref().and_then(|w| w.nearest_land(p.x, p.y));
        match (best, far) {
            (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    fn camera_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        let (secondary, middle, primary, alt, shift, scroll, dt) = ui.input(|i| {
            (
                i.pointer.button_down(PointerButton::Secondary),
                i.pointer.button_down(PointerButton::Middle),
                i.pointer.button_down(PointerButton::Primary),
                i.modifiers.alt,
                i.modifiers.shift,
                i.smooth_scroll_delta.y,
                i.stable_dt.min(0.1) as f64,
            )
        });
        let typing = ui.ctx().egui_wants_keyboard_input();
        let hovered =
            response.hovered() || self.flying || self.orbiting || self.panning || self.looking;
        let starting = response.is_pointer_button_down_on();
        self.flying = secondary && (self.flying || starting);
        self.orbiting = primary && alt && (self.orbiting || starting) && self.drag.is_none();
        self.panning = middle && (self.panning || starting);
        self.looking = self.looking && primary && self.drag.is_none();
        // Raw deltas are gathered from device events; fall back to egui motion.
        let mut delta = std::mem::take(&mut self.mouse);
        if delta == Vec2::ZERO && (self.flying || self.orbiting || self.panning || self.looking) {
            delta = ui.input(|i| i.pointer.delta());
        }
        let (dx, dy) = (delta.x as f64, delta.y as f64);
        let mut moved = false;
        // Keys move the camera while flying (right button), or whenever the
        // pointer is over the viewport: arrows and PageUp/PageDown always,
        // WASD/QE when no actor is selected (otherwise W/E/R pick tools).
        let keys = !typing && (self.flying || hovered);
        let key = |k: Key| (keys && ui.input(|i| i.key_down(k))) as i32 as f64;
        let wasd = self.flying || self.selected_actor().is_none();
        let letter = |k: Key| if wasd { key(k) } else { 0.0 };
        let ahead = key(Key::ArrowUp) - key(Key::ArrowDown) + letter(Key::W) - letter(Key::S);
        let side = key(Key::ArrowRight) - key(Key::ArrowLeft) + letter(Key::D) - letter(Key::A);
        let rise = key(Key::PageUp) - key(Key::PageDown) + letter(Key::E) - letter(Key::Q);
        if keys {
            let (faster, slower) = ui.input(|i| {
                (
                    i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals),
                    i.key_pressed(Key::Minus),
                )
            });
            if faster {
                self.cam.speed = (self.cam.speed * 2.0).min(20_000.0);
            }
            if slower {
                self.cam.speed = (self.cam.speed * 0.5).max(0.2);
            }
        }
        let wish = self.cam.forward() * ahead + self.cam.right() * side + Vec3::new(0.0, 0.0, rise);
        let target = if wish.length() > 0.0 {
            wish.normalized() * self.cam.speed * if shift { 5.0 } else { 1.0 }
        } else {
            Vec3::ZERO
        };
        if wish.length() > 0.0 || self.flying || self.looking || self.orbiting || self.panning {
            self.flight = None;
        }
        // Smooth acceleration and braking (no jerky starts and stops).
        let k = 1.0 - (-dt * 9.0).exp();
        self.velocity = self.velocity + (target - self.velocity) * k;
        if self.velocity.length() < 0.01 * self.cam.speed.max(0.1) && target == Vec3::ZERO {
            self.velocity = Vec3::ZERO;
        }
        if self.velocity != Vec3::ZERO {
            self.cam.eye = self.cam.eye + self.velocity * dt;
            moved = true;
        }
        if self.flying || self.looking {
            self.cam.look(dx, dy);
            if self.flying && scroll != 0.0 {
                self.cam.speed =
                    (self.cam.speed * 1.25f64.powf((scroll / 50.0) as f64)).clamp(0.2, 20_000.0);
            }
            moved = true;
        } else if self.orbiting {
            self.cam.orbit(dx, dy);
            moved = true;
        } else if self.panning {
            let k = self.cam.focus.max(self.cam.eye.z.abs() * 0.5) * 0.0016;
            self.cam.eye = self.cam.eye - self.cam.right() * (dx * k) + self.cam.up() * (dy * k);
            moved = true;
        } else if hovered && scroll != 0.0 {
            // Dolly: steps grow with height so high views move usefully.
            let reach = self.cam.focus.max(self.cam.eye.z.abs() * 0.3);
            let step = (reach * 0.12).max(0.3) * (scroll as f64 / 50.0);
            self.cam.eye = self.cam.eye + self.cam.forward() * step;
            self.cam.focus = (self.cam.focus - step).max(1.0);
            self.flight = None;
            moved = true;
        }
        if let Some(f) = self.flight {
            let t = (f.start.elapsed().as_secs_f64() / f.seconds).min(1.0);
            let e = t * t * (3.0 - 2.0 * t);
            // Arc over long distances so the flight shows the way.
            let lift = (f.to - f.from).length() * 0.15 * (std::f64::consts::PI * t).sin();
            self.cam.eye = f.from + (f.to - f.from) * e + Vec3::new(0.0, 0.0, lift);
            self.cam.yaw = f.yaw.0 + (f.yaw.1 - f.yaw.0) * e;
            self.cam.pitch = f.pitch.0 + (f.pitch.1 - f.pitch.0) * e;
            if t >= 1.0 {
                self.flight = None;
            }
            moved = true;
        }
        if moved {
            // Never into the ground or seabed.
            let ground = self.ground(self.cam.eye.x, self.cam.eye.y);
            let clearance = 0.6;
            if self.cam.eye.z < ground + clearance {
                self.cam.eye.z = ground + clearance;
                self.velocity.z = self.velocity.z.max(0.0);
            }
            self.view_dirty = true;
        }
    }

    fn gizmo(&self, camera: viewport_edit::Camera) -> Option<Gizmo> {
        let actor = self.selected_actor()?;
        let scene = self
            .drag
            .as_ref()
            .and_then(|d| d.1.as_ref())
            .unwrap_or(self.scene());
        let pivot = actor.position(scene)?;
        Gizmo::new(camera, actor, pivot, self.tool)
    }

    fn selection_overlay(&self, painter: &egui::Painter, camera: viewport_edit::Camera) {
        let Some(actor) = self.selected_actor() else {
            return;
        };
        let session = self.model.session.as_ref().unwrap();
        let snapshot = session.view_snapshot();
        let (center, half, q) = match actor {
            Actor::Structure => {
                let s = self.scene();
                let p = Vec3::new(
                    s.structure_position[0],
                    s.structure_position[1],
                    s.structure_position[2] - 4.0 * s.structure_scale,
                );
                (
                    p,
                    Vec3::new(15.6, 3.6, 5.0) * s.structure_scale,
                    pelagic::math::Quat::IDENTITY,
                )
            }
            _ => {
                let Some(id) = actor.runtime_id() else {
                    return;
                };
                let Some(body) = snapshot.vehicles.iter().find(|v| v.id == id) else {
                    return;
                };
                (body.position, body.half, body.orientation)
            }
        };
        let corners: Vec<Option<[f32; 2]>> = (0..8)
            .map(|i| {
                let local = Vec3::new(
                    if i & 1 == 0 { -half.x } else { half.x },
                    if i & 2 == 0 { -half.y } else { half.y },
                    if i & 4 == 0 { -half.z } else { half.z },
                );
                camera.project(center + q.rotate(local))
            })
            .collect();
        for (a, b) in [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            if let (Some(p), Some(q)) = (corners[a], corners[b]) {
                painter.line_segment(
                    [Pos2::new(p[0], p[1]), Pos2::new(q[0], q[1])],
                    Stroke::new(1.5, theme::SELECT),
                );
            }
        }
        if let Some(p) = camera.project(center + Vec3::new(0.0, 0.0, half.z * 1.2)) {
            painter.text(
                Pos2::new(p[0], p[1] - 6.0),
                Align2::CENTER_BOTTOM,
                actor.name(self.scene()),
                FontId::proportional(12.0),
                theme::SELECT,
            );
        }
    }

    // --------------------------------------------------------------- outliner

    fn outliner(&mut self, ui: &mut egui::Ui) {
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("🔍 Search actors")
                .desired_width(f32::INFINITY),
        );
        super::a11y::name(&search, "Search actors");
        ui.add_space(2.0);
        let scene = self.scene().clone();
        let mut rows: Vec<(Selection, String, &str, &str)> = vec![
            (
                Selection::Ocean,
                "Ocean environment".into(),
                "🌊",
                "Environment",
            ),
            (Selection::Seabed, "Seabed".into(), "⛰", "Terrain"),
            (
                Selection::Life,
                format!("Marine life ({})", scene.school.count),
                "🐟",
                "School",
            ),
            (Selection::Sensors, "Sensors".into(), "📡", "Sensor rig"),
        ];
        if scene.vehicle {
            rows.push((
                Selection::Actor(Actor::Reference),
                "Reference vehicle".into(),
                "🚀",
                if scene.reference_asset.is_some() {
                    "Static mesh"
                } else {
                    "ROV"
                },
            ));
        }
        for e in &scene.entities {
            rows.push((
                Selection::Actor(Actor::Entity(e.id)),
                e.name.clone(),
                if e.asset.is_some() {
                    "◼"
                } else {
                    ["🚀", "⛵", "🔸"][e.kind.min(2) as usize]
                },
                if e.asset.is_some() {
                    "Static mesh"
                } else {
                    ["ROV", "Vessel", "Drifter"][e.kind.min(2) as usize]
                },
            ));
        }
        if scene.structure {
            rows.push((
                Selection::Actor(Actor::Structure),
                "Harbour structure".into(),
                "🏗",
                "Static geometry",
            ));
        }
        let query = self.search.to_lowercase();
        egui::Grid::new("outliner")
            .num_columns(2)
            .striped(true)
            .min_col_width(60.0)
            .spacing([12.0, 3.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Item Label").small().color(theme::MUTED));
                ui.label(RichText::new("Type").small().color(theme::MUTED));
                ui.end_row();
                for (selection, name, icon, kind) in rows {
                    if !query.is_empty() && !name.to_lowercase().contains(&query) {
                        continue;
                    }
                    let selected = self.selection == selection;
                    let response = ui.selectable_label(selected, format!("{icon}  {name}"));
                    if response.clicked() {
                        self.select(selection);
                    }
                    if response.double_clicked() {
                        self.select(selection);
                        self.frame_selected();
                    }
                    if let Selection::Actor(_) = selection {
                        response.context_menu(|ui| {
                            self.select(selection);
                            if ui.button("Frame (F)").clicked() {
                                self.frame_selected();
                                ui.close();
                            }
                            if ui.button("Duplicate (Ctrl+D)").clicked() {
                                self.duplicate_selected();
                                ui.close();
                            }
                            if ui.button("Delete (Del)").clicked() {
                                self.delete_selected();
                                ui.close();
                            }
                        });
                    }
                    ui.label(RichText::new(kind).color(theme::MUTED));
                    ui.end_row();
                }
            });
        let count =
            4 + usize::from(scene.vehicle) + scene.entities.len() + usize::from(scene.structure);
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("{count} actors"))
                .small()
                .color(theme::MUTED),
        );
    }

    // ---------------------------------------------------------------- details

    fn details(&mut self, ui: &mut egui::Ui) {
        let base = self.pending.clone().unwrap_or_else(|| self.scene().clone());
        let mut scene = base.clone();
        let mut changed = false;
        let mut export: Option<pelagic::entities::Entity> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.selection {
                Selection::None => self.scene_overview(ui),
                Selection::Actor(Actor::Structure) => {
                    header(ui, "🏗", "Harbour structure", "Static geometry");
                    section(ui, "Transform", |ui| {
                        changed |=
                            vec3_row(ui, "Location", &mut scene.structure_position, 0.1, " m");
                        changed |= row(
                            ui,
                            "Uniform scale",
                            &mut scene.structure_scale,
                            0.01,
                            0.1..=5.0,
                            "",
                        );
                    });
                }
                Selection::Actor(actor) => {
                    let mut entity = actor.entity(&scene).expect("selected entity");
                    let before = entity.clone();
                    let kind = if entity.asset.is_some() {
                        "Static mesh body"
                    } else {
                        ["ROV", "Surface vessel", "Drifter"][entity.kind.min(2) as usize]
                    };
                    header(ui, "◼", &entity.name.clone(), kind);
                    if ui
                        .button("Export vehicle definition…")
                        .on_hover_text(
                            "Physics, thrusters, controllers and hull mesh as a shareable file",
                        )
                        .clicked()
                    {
                        export = Some(entity.clone());
                    }
                    if let Actor::Entity(_) = actor {
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            changed |= ui
                                .add(
                                    egui::TextEdit::singleline(&mut entity.name)
                                        .desired_width(f32::INFINITY),
                                )
                                .changed();
                        });
                    }
                    section(ui, "Transform", |ui| {
                        let mut p = [entity.position.x, entity.position.y, entity.position.z];
                        if vec3_row(ui, "Location", &mut p, 0.05, " m") {
                            entity.position = Vec3::new(p[0], p[1], p[2]);
                            changed = true;
                        }
                        changed |= row(
                            ui,
                            "Rotation (yaw)",
                            &mut entity.heading,
                            0.5,
                            -360.0..=360.0,
                            "°",
                        );
                        let mut size = [
                            entity.model.half.x * 2.0,
                            entity.model.half.y * 2.0,
                            entity.model.half.z * 2.0,
                        ];
                        if vec3_row(ui, "Size", &mut size, 0.01, " m") {
                            let h = entity.model.half;
                            let f = Vec3::new(
                                size[0] / (2.0 * h.x),
                                size[1] / (2.0 * h.y),
                                size[2] / (2.0 * h.z),
                            );
                            if f.is_finite() && f.x > 0.0 && f.y > 0.0 && f.z > 0.0 {
                                entity.model.half = h.component_mul(f);
                                entity.model.center_buoyancy =
                                    entity.model.center_buoyancy.component_mul(f);
                                for t in &mut entity.model.thrusters {
                                    t.position = t.position.component_mul(f);
                                }
                                for fin in &mut entity.model.fins {
                                    fin.position = fin.position.component_mul(f);
                                }
                                changed = true;
                            }
                        }
                    });
                    section(ui, "Physics", |ui| {
                        changed |= row(ui, "Mass", &mut entity.mass, 0.1, 0.01..=100000.0, " kg");
                        changed |= row(
                            ui,
                            "Displaced volume",
                            &mut entity.volume,
                            0.001,
                            0.0..=1000.0,
                            " m³",
                        );
                        changed |= row(
                            ui,
                            "Thrust limit",
                            &mut entity.model.thrust_limit,
                            1.0,
                            0.0..=100000.0,
                            " N",
                        );
                        let buoyancy =
                            entity.volume * scene.density * 9.80665 - entity.mass * 9.80665;
                        ui.label("Net buoyancy");
                        ui.label(RichText::new(format!("{buoyancy:+.1} N")).color(
                            if buoyancy.abs() < 1.0 {
                                theme::TEXT
                            } else if buoyancy > 0.0 {
                                Color32::from_rgb(120, 200, 255)
                            } else {
                                Color32::from_rgb(240, 170, 90)
                            },
                        ));
                        ui.end_row();
                    });
                    vehicle_ui::physics_sections(ui, &mut entity, scene.density, &mut changed);
                    section(ui, "Appearance", |ui| {
                        let mesh_name = entity
                            .asset
                            .and_then(|id| scene.library.mesh(id))
                            .map_or("Procedural (built-in)".to_string(), |m| m.name.clone());
                        ui.label("Static mesh");
                        let combo = egui::ComboBox::from_id_salt("mesh")
                            .selected_text(mesh_name)
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                changed |= ui
                                    .selectable_value(
                                        &mut entity.asset,
                                        None,
                                        "Procedural (built-in)",
                                    )
                                    .changed();
                                for m in &scene.library.meshes {
                                    changed |= ui
                                        .selectable_value(&mut entity.asset, Some(m.id), &m.name)
                                        .changed();
                                }
                            });
                        super::a11y::name(&combo.response, "Static mesh");
                        ui.end_row();
                        let items = library::material_items(&scene);
                        let material_name = entity
                            .material
                            .and_then(|id| scene.library.material(id))
                            .map_or("Default look".to_string(), |m| m.name.clone());
                        let mut choice: Option<Option<Item>> = None;
                        ui.label("Material");
                        let combo = egui::ComboBox::from_id_salt("material")
                            .selected_text(material_name)
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(entity.material.is_none(), "Default look")
                                    .clicked()
                                {
                                    choice = Some(None);
                                }
                                for (item, m) in &items {
                                    let label = if matches!(item, Item::Preset(_)) {
                                        format!("{} (preset)", m.name)
                                    } else {
                                        m.name.clone()
                                    };
                                    if ui
                                        .selectable_label(
                                            Some(*item) == entity.material.map(Item::Material),
                                            label,
                                        )
                                        .clicked()
                                    {
                                        choice = Some(Some(*item));
                                    }
                                }
                            });
                        super::a11y::name(&combo.response, "Material");
                        ui.end_row();
                        if let Some(choice) = choice {
                            match choice {
                                None => entity.material = None,
                                Some(Item::Material(id)) => entity.material = Some(id),
                                Some(item) => {
                                    if let Some(m) = library::material(&scene, item)
                                        && let Ok(id) = scene.library.add_material(&m)
                                    {
                                        entity.material = Some(id);
                                    }
                                }
                            }
                            changed = true;
                        }
                        if let Some(id) = entity.material
                            && let Some(m) = scene.library.material_mut(id)
                        {
                            ui.label("Base colour");
                            changed |= ui.color_edit_button_srgb(&mut m.base_color).changed();
                            ui.end_row();
                            changed |= slider_row(ui, "Roughness", &mut m.roughness, 0.02..=1.0);
                            changed |= slider_row(ui, "Metallic", &mut m.metallic, 0.0..=1.0);
                            ui.label("");
                            ui.label(
                                RichText::new("Shared by every actor using this material.")
                                    .small()
                                    .color(theme::MUTED),
                            );
                            ui.end_row();
                        }
                    });
                    if entity != before {
                        match actor {
                            Actor::Reference => scene.set_reference(&entity),
                            Actor::Entity(id) => {
                                if let Some(e) = scene.entities.iter_mut().find(|e| e.id == id) {
                                    *e = entity;
                                }
                            }
                            Actor::Structure => {}
                        }
                    }
                }
                Selection::Ocean | Selection::Seabed => {
                    header(ui, "🌊", "Ocean environment", "World settings");
                    section(ui, "Water", |ui| {
                        changed |= row(
                            ui,
                            "Seabed depth",
                            &mut scene.bottom_depth,
                            0.5,
                            1.0..=11000.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Density",
                            &mut scene.density,
                            0.5,
                            900.0..=1300.0,
                            " kg/m³",
                        );
                        changed |= row(
                            ui,
                            "Visibility loss",
                            &mut scene.attenuation,
                            0.001,
                            0.0..=5.0,
                            " /m",
                        );
                        changed |= row(
                            ui,
                            "Sun elevation",
                            &mut scene.sun_elevation,
                            0.5,
                            5.0..=85.0,
                            "°",
                        );
                    });
                    section(ui, "Waves", |ui| {
                        changed |= row(
                            ui,
                            "Significant height",
                            &mut scene.wave_height,
                            0.05,
                            0.0..=15.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Peak period",
                            &mut scene.wave_period,
                            0.1,
                            2.0..=20.0,
                            " s",
                        );
                        changed |= row(
                            ui,
                            "Direction",
                            &mut scene.wave_direction,
                            1.0,
                            -360.0..=360.0,
                            "°",
                        );
                        changed |= row(ui, "Spread", &mut scene.wave_spread, 1.0, 0.0..=90.0, "°");
                        changed |= row(
                            ui,
                            "Swell height",
                            &mut scene.swell_height,
                            0.05,
                            0.0..=12.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Swell period",
                            &mut scene.swell_period,
                            0.1,
                            4.0..=30.0,
                            " s",
                        );
                        changed |= row(
                            ui,
                            "Swell direction",
                            &mut scene.swell_direction,
                            1.0,
                            -360.0..=360.0,
                            "°",
                        );
                        changed |= row(
                            ui,
                            "Wind speed",
                            &mut scene.wind_speed,
                            0.1,
                            0.0..=60.0,
                            " m/s",
                        );
                        if !scene.sea_source.is_empty() {
                            ui.label("Source");
                            ui.label(RichText::new(&scene.sea_source).small().color(theme::MUTED));
                            ui.end_row();
                        }
                    });
                    section(ui, "Current", |ui| {
                        changed |=
                            vec3_row(ui, "Surface current", &mut scene.current, 0.01, " m/s");
                        changed |= row(
                            ui,
                            "Depth shear",
                            &mut scene.shear,
                            0.0005,
                            -0.05..=0.05,
                            " 1/s",
                        );
                        changed |= row(
                            ui,
                            "Tide period",
                            &mut scene.tide_period,
                            10.0,
                            0.0..=86400.0,
                            " s",
                        );
                    });
                    section(ui, "Water column", |ui| {
                        changed |= row(
                            ui,
                            "Surface temperature",
                            &mut scene.temperature,
                            0.1,
                            -2.0..=35.0,
                            " °C",
                        );
                        changed |= row(
                            ui,
                            "Deep temperature",
                            &mut scene.deep_temperature,
                            0.1,
                            -2.0..=35.0,
                            " °C",
                        );
                        changed |= row(
                            ui,
                            "Salinity",
                            &mut scene.salinity,
                            0.1,
                            0.0..=42.0,
                            " g/kg",
                        );
                        changed |= row(
                            ui,
                            "Thermocline",
                            &mut scene.thermocline,
                            1.0,
                            1.0..=500.0,
                            " m",
                        );
                    });
                    section(ui, "Seabed", |ui| {
                        let mut terrain = scene.terrain;
                        egui::ComboBox::from_label("Shape")
                            .selected_text(
                                ["Flat", "Coastal shelf", "Canyon", "Imported / real data"]
                                    [terrain.min(3) as usize],
                            )
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut terrain, 0, "Flat");
                                ui.selectable_value(&mut terrain, 1, "Coastal shelf");
                                ui.selectable_value(&mut terrain, 2, "Canyon");
                                if scene.grid.is_some() {
                                    ui.selectable_value(&mut terrain, 3, "Imported / real data");
                                }
                            });
                        if terrain != scene.terrain {
                            scene.terrain = terrain;
                            changed = true;
                        }
                        let mut material = scene.terrain_material;
                        egui::ComboBox::from_label("Material")
                            .selected_text(["Automatic", "Sand", "Rock"][material.min(2) as usize])
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut material, 0, "Automatic");
                                ui.selectable_value(&mut material, 1, "Sand");
                                ui.selectable_value(&mut material, 2, "Rock");
                            });
                        if material != scene.terrain_material {
                            scene.terrain_material = material;
                            changed = true;
                        }
                    });
                }
                Selection::Life => {
                    header(ui, "🐟", "Marine life", "Schooling model");
                    section(ui, "School", |ui| {
                        let mut count = scene.school.count as f64;
                        if row(ui, "Fish count", &mut count, 1.0, 0.0..=256.0, "") {
                            scene.school.count = count.round() as usize;
                            changed = true;
                        }
                        changed |= row(
                            ui,
                            "Swim speed",
                            &mut scene.school.speed,
                            0.05,
                            0.1..=5.0,
                            " m/s",
                        );
                        changed |= row(
                            ui,
                            "School radius",
                            &mut scene.school.radius,
                            0.1,
                            1.0..=100.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Fish size",
                            &mut scene.school.size,
                            0.01,
                            0.1..=2.0,
                            " m",
                        );
                        let mut depth = -scene.school.center.z;
                        if row(ui, "School depth", &mut depth, 0.2, 0.0..=5000.0, " m") {
                            scene.school.center.z = -depth;
                            changed = true;
                        }
                        changed |= row(
                            ui,
                            "Separation",
                            &mut scene.school.separation,
                            0.05,
                            0.2..=10.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Obstacle avoidance",
                            &mut scene.school.avoidance,
                            0.1,
                            0.0..=20.0,
                            " m",
                        );
                        let mut tracers = scene.particles as f64;
                        if row(ui, "Passive tracers", &mut tracers, 1.0, 0.0..=512.0, "") {
                            scene.particles = tracers.round() as usize;
                            changed = true;
                        }
                    });
                }
                Selection::Sensors => {
                    header(ui, "📡", "Sensors", "Sonar / navigation");
                    section(ui, "Sonar", |ui| {
                        changed |= row(
                            ui,
                            "Max range",
                            &mut scene.sensors.range,
                            0.5,
                            1.0..=500.0,
                            " m",
                        );
                        let mut beams = scene.sensors.beams as f64;
                        if row(ui, "Beams", &mut beams, 1.0, 1.0..=128.0, "") {
                            scene.sensors.beams = beams.round() as usize;
                            changed = true;
                        }
                        changed |= row(
                            ui,
                            "Fan angle",
                            &mut scene.sensors.fan_degrees,
                            0.5,
                            1.0..=150.0,
                            "°",
                        );
                        changed |= row(
                            ui,
                            "Sample rate",
                            &mut scene.sensors.rate_hz,
                            0.5,
                            0.5..=30.0,
                            " Hz",
                        );
                        changed |= row(
                            ui,
                            "Range noise (σ)",
                            &mut scene.sensors.range_noise,
                            0.01,
                            0.0..=5.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Latency",
                            &mut scene.sensors.latency_seconds,
                            0.01,
                            0.0..=10.0,
                            " s",
                        );
                    });
                    section(ui, "Mount (body frame)", |ui| {
                        changed |= row(
                            ui,
                            "Forward (x)",
                            &mut scene.sensors.mount.x,
                            0.01,
                            -20.0..=20.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Left (y)",
                            &mut scene.sensors.mount.y,
                            0.01,
                            -20.0..=20.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Up (z)",
                            &mut scene.sensors.mount.z,
                            0.01,
                            -20.0..=20.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "Down tilt",
                            &mut scene.sensors.tilt_degrees,
                            0.5,
                            0.0..=90.0,
                            "°",
                        );
                        changed |= row(
                            ui,
                            "Camera field of view",
                            &mut scene.sensors.camera_fov,
                            0.5,
                            20.0..=120.0,
                            "°",
                        );
                    });
                    section(ui, "Navigation noise", |ui| {
                        changed |= row(
                            ui,
                            "Position (σ)",
                            &mut scene.sensors.position_noise,
                            0.05,
                            0.0..=100.0,
                            " m",
                        );
                        changed |= row(
                            ui,
                            "DVL velocity (σ)",
                            &mut scene.sensors.velocity_sigma,
                            0.001,
                            0.0..=10.0,
                            " m/s",
                        );
                        changed |= row(
                            ui,
                            "Gyro (σ)",
                            &mut scene.sensors.gyro_sigma,
                            0.0005,
                            0.0..=1.0,
                            " rad/s",
                        );
                        changed |= row(
                            ui,
                            "Accelerometer (σ)",
                            &mut scene.sensors.acceleration_sigma,
                            0.001,
                            0.0..=10.0,
                            " m/s²",
                        );
                    });
                }
            });
        if let Some(entity) = export {
            self.export_vehicle(&entity);
        }
        if changed {
            let dragging = ui.input(|i| i.pointer.any_down());
            if dragging {
                if let Some(s) = self.model.session.as_mut()
                    && s.preview_scene(&scene).is_ok()
                {
                    self.view_dirty = true;
                }
                self.pending = Some(scene);
            } else {
                self.pending = None;
                if let Some(s) = self.model.session.as_mut() {
                    s.clear_preview();
                }
                self.apply(scene, "");
            }
        } else if self.pending.is_some() && !ui.input(|i| i.pointer.any_down()) {
            let scene = self.pending.take().unwrap();
            if let Some(s) = self.model.session.as_mut() {
                s.clear_preview();
            }
            self.apply(scene, "");
        }
    }

    // --------------------------------------------------------- content browser

    fn thumbnail(
        &mut self,
        ctx: &egui::Context,
        key: String,
        mesh: Option<&pelagic::assets::Mesh>,
        stretch: [f64; 3],
        material: &pelagic::assets::Material,
    ) -> egui::TextureId {
        if self.thumbs.len() > 200 {
            self.thumbs.clear();
        }
        self.thumbs
            .entry(key.clone())
            .or_insert_with(|| {
                let frame = pelagic::thumbnail::render(mesh, stretch, material, 192, 144);
                ctx.load_texture(
                    key,
                    egui::ColorImage::from_rgb([frame.width, frame.height], &frame.rgb),
                    egui::TextureOptions::LINEAR,
                )
            })
            .id()
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("⬆ Import").clicked() {
                self.start_import();
            }
            let selected = self.model.selected_library;
            if ui
                .add_enabled(
                    matches!(selected, Some(Item::Mesh(_))),
                    egui::Button::new("➕ Place"),
                )
                .on_hover_text("Add an instance in front of the camera (or double-click a mesh)")
                .clicked()
                && let Some(Item::Mesh(id)) = selected
            {
                self.place(id, None);
            }
            if ui
                .add_enabled(
                    selected.is_some() && self.actor_index().is_some(),
                    egui::Button::new("🎨 Assign"),
                )
                .on_hover_text("Apply to the selected actor")
                .clicked()
                && let (Some(item), Some(actor)) = (selected, self.actor_index())
            {
                match library::assign(self.scene(), actor, item) {
                    Ok(next) => {
                        self.apply(next, "Assigned.");
                    }
                    Err(e) => self.log(e),
                }
            }
            if ui
                .add_enabled(selected.is_some(), egui::Button::new("🗑 Remove"))
                .clicked()
                && let Some(item) = selected
            {
                match library::remove(self.scene(), item) {
                    Ok(next) => {
                        if self.apply(next, "Removed from library.") {
                            self.model.selected_library = None;
                        }
                    }
                    Err(e) => self.log(e),
                }
            }
            ui.separator();
            for (i, label) in ["All", "Meshes", "Materials"].iter().enumerate() {
                if ui
                    .selectable_label(self.content_filter == i as u8, *label)
                    .clicked()
                {
                    self.content_filter = i as u8;
                }
            }
        });
        ui.separator();
        let scene = self.scene().clone();
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                    if self.content_filter != 2 {
                        for asset in &scene.library.meshes {
                            let key = format!(
                                "mesh-{}-{:p}",
                                asset.id,
                                std::sync::Arc::as_ptr(&asset.mesh)
                            );
                            let tex = self.thumbnail(
                                &ctx,
                                key,
                                Some(&asset.mesh),
                                asset.proportions,
                                &super::super::view::default_material(),
                            );
                            let item = Item::Mesh(asset.id);
                            let uses = library::usage(&scene, item);
                            let response = tile(
                                ui,
                                tex,
                                &asset.name,
                                &format!("Static mesh · {uses} use(s)"),
                                self.model.selected_library == Some(item),
                            );
                            if response.clicked() {
                                self.model.selected_library = Some(item);
                            }
                            if response.double_clicked() {
                                self.place(asset.id, None);
                            }
                        }
                    }
                    if self.content_filter != 1 {
                        for (item, material) in library::material_items(&scene) {
                            if self.content_filter == 0 && matches!(item, Item::Preset(_)) {
                                continue;
                            }
                            let key = format!(
                                "mat-{:?}-{:?}-{}-{}",
                                item, material.base_color, material.roughness, material.metallic
                            );
                            let tex = self.thumbnail(&ctx, key, None, [1.0; 3], &material);
                            let kind = if matches!(item, Item::Preset(_)) {
                                "Material preset".to_string()
                            } else {
                                format!("Material · {} use(s)", library::usage(&scene, item))
                            };
                            let response = tile(
                                ui,
                                tex,
                                &material.name,
                                &kind,
                                self.model.selected_library == Some(item),
                            );
                            if response.clicked() {
                                self.model.selected_library = Some(item);
                            }
                            if response.double_clicked()
                                && let Some(actor) = self.actor_index()
                                && let Ok(next) = library::assign(self.scene(), actor, item)
                            {
                                self.apply(next, "Assigned material.");
                            }
                        }
                    }
                });
                if scene.library.meshes.is_empty() && self.content_filter == 1 {
                    ui.label(
                        RichText::new("No meshes yet — use ⬆ Import to add an OBJ.")
                            .color(theme::MUTED),
                    );
                }
            });
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("LogRender: {}", self.adapter))
                        .monospace()
                        .color(theme::MUTED),
                );
                for line in &self.model.messages {
                    let color = if line.contains("refused") || line.contains("failed") {
                        Color32::from_rgb(240, 110, 90)
                    } else {
                        theme::TEXT
                    };
                    ui.label(
                        RichText::new(format!("LogPelagic: {line}"))
                            .monospace()
                            .color(color),
                    );
                }
            });
    }
}

const CONTROLS: &str = "Viewport\n  RMB + WASD/QE  fly (wheel = speed, Shift = fast)\n  Alt + LMB      orbit\n  MMB            pan\n  Wheel          dolly\n  LMB            select / drag gizmo\n  W / E / R      move / rotate / scale\n  F              frame selection\n  Del, Ctrl+D    delete, duplicate\n  Ctrl+Z / Y     undo / redo\n  Ctrl+S         save";

fn header(ui: &mut egui::Ui, icon: &str, name: &str, kind: &str) {
    egui::Frame::new()
        .fill(theme::CARD)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(18, 52, 60))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(9, 5))
                    .show(ui, |ui| {
                        ui.label(RichText::new(icon).size(18.0).color(theme::ACCENT));
                    });
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.add(
                        egui::Label::new(
                            RichText::new(name).strong().size(15.0).color(theme::STRONG),
                        )
                        .wrap(),
                    );
                    ui.add(
                        egui::Label::new(RichText::new(kind).small().color(theme::MUTED)).wrap(),
                    );
                });
            });
        });
    ui.add_space(6.0);
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::CARD)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: 6,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(
                RichText::new(title.to_uppercase())
                    .small()
                    .strong()
                    .color(theme::MUTED),
            )
            .id_salt(title)
            .default_open(true)
            .show(ui, |ui| {
                let column_width = ((ui.available_width() - 12.0) * 0.5).max(40.0);
                egui::Grid::new(title)
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .min_col_width(0.0)
                    .max_col_width(column_width)
                    .show(ui, |ui| add(ui));
            });
        });
    ui.add_space(6.0);
}

fn row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    speed: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
) -> bool {
    ui.label(label);
    let response = ui.add(
        egui::DragValue::new(value)
            .speed(speed)
            .range(range)
            .suffix(suffix)
            .max_decimals(4),
    );
    super::a11y::name(&response, label);
    ui.end_row();
    response.changed()
}

fn slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    ui.label(label);
    let changed = ui
        .add(egui::Slider::new(value, range).max_decimals(2))
        .changed();
    ui.end_row();
    changed
}

fn vec3_row(ui: &mut egui::Ui, label: &str, v: &mut [f64; 3], speed: f64, suffix: &str) -> bool {
    ui.label(label);
    let mut changed = false;
    ui.horizontal(|ui| {
        for (i, color) in [theme::X_AXIS, theme::Y_AXIS, theme::Z_AXIS]
            .into_iter()
            .enumerate()
        {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 18.0), Sense::hover());
            ui.painter().rect_filled(rect, 1.0, color);
            changed |= ui
                .add(
                    egui::DragValue::new(&mut v[i])
                        .speed(speed)
                        .suffix(suffix)
                        .max_decimals(2),
                )
                .changed();
        }
    });
    ui.end_row();
    changed
}

fn tile(
    ui: &mut egui::Ui,
    tex: egui::TextureId,
    name: &str,
    kind: &str,
    selected: bool,
) -> egui::Response {
    let size = Vec2::new(112.0, 128.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    let bg = if selected {
        Color32::from_rgb(20, 60, 104)
    } else if response.hovered() {
        theme::HOVER
    } else {
        Color32::from_rgb(28, 28, 28)
    };
    painter.rect_filled(rect, 4.0, bg);
    let image = Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::new(104.0, 78.0));
    painter.image(
        tex,
        image,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(rect.left(), image.bottom() + 2.0),
            Vec2::new(size.x, 2.0),
        ),
        0.0,
        if kind.starts_with("Material") {
            Color32::from_rgb(80, 160, 90)
        } else {
            Color32::from_rgb(60, 140, 230)
        },
    );
    painter.text(
        Pos2::new(rect.left() + 6.0, image.bottom() + 8.0),
        Align2::LEFT_TOP,
        truncate(name, 16),
        FontId::proportional(12.0),
        Color32::WHITE,
    );
    painter.text(
        Pos2::new(rect.left() + 6.0, image.bottom() + 25.0),
        Align2::LEFT_TOP,
        truncate(kind, 20),
        FontId::proportional(10.0),
        theme::MUTED,
    );
    if selected {
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.5, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(format!("{name}\n{kind}"))
}

fn truncate(text: &str, n: usize) -> String {
    if text.chars().count() <= n {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(n - 1).collect::<String>())
    }
}

fn draw_gizmo(painter: &egui::Painter, gizmo: &Gizmo, active: Option<Handle>) {
    for h in &gizmo.handles {
        let base = match h.handle {
            Handle::X => theme::X_AXIS,
            Handle::Y => theme::Y_AXIS,
            Handle::Z => theme::Z_AXIS,
            Handle::Heading => theme::Z_AXIS,
            Handle::Uniform => Color32::from_rgb(230, 230, 230),
        };
        let color = if active == Some(h.handle) {
            Color32::from_rgb(255, 230, 0)
        } else {
            base
        };
        let points: Vec<Pos2> = h.points.iter().map(|p| Pos2::new(p[0], p[1])).collect();
        painter.add(egui::Shape::line(points.clone(), Stroke::new(3.0, color)));
        if let (Some(a), Some(b)) = (points.first(), points.last())
            && matches!(h.handle, Handle::X | Handle::Y | Handle::Z)
        {
            let d = (*b - *a).normalized();
            let n = Vec2::new(-d.y, d.x);
            painter.add(egui::Shape::convex_polygon(
                vec![*b + d * 10.0, *b + n * 5.0, *b - n * 5.0],
                color,
                Stroke::NONE,
            ));
        } else if let Some(b) = points.last()
            && h.handle == Handle::Uniform
        {
            painter.rect_filled(Rect::from_center_size(*b, Vec2::splat(10.0)), 1.0, color);
        }
    }
}

fn axis_triad(painter: &egui::Painter, rect: Rect, cam: &FlyCam) {
    let origin = rect.left_bottom() + Vec2::new(36.0, -44.0);
    let (r, u) = (cam.right(), cam.up());
    for (axis, color, label) in [
        (Vec3::new(1.0, 0.0, 0.0), theme::X_AXIS, "X"),
        (Vec3::new(0.0, 1.0, 0.0), theme::Y_AXIS, "Y"),
        (Vec3::new(0.0, 0.0, 1.0), theme::Z_AXIS, "Z"),
    ] {
        let tip = origin + Vec2::new(axis.dot(r) as f32, -axis.dot(u) as f32) * 22.0;
        painter.line_segment([origin, tip], Stroke::new(2.0, color));
        painter.text(
            tip,
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(10.0),
            color,
        );
    }
}

/// Native Windows dialogs (file pickers, unsaved-changes prompt).
mod native {
    pub enum Answer {
        Save,
        Discard,
        Cancel,
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    #[cfg(windows)]
    pub fn ask_save(hwnd: isize, action: &str) -> Answer {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let text = wide(&format!(
            "Save changes to the current scene before {action}?"
        ));
        let title = wide("Pelagic Studio");
        match unsafe {
            MessageBoxW(
                hwnd as _,
                text.as_ptr(),
                title.as_ptr(),
                MB_YESNOCANCEL | MB_ICONWARNING,
            )
        } {
            IDYES => Answer::Save,
            IDNO => Answer::Discard,
            _ => Answer::Cancel,
        }
    }
    #[cfg(not(windows))]
    pub fn ask_save(_: isize, _: &str) -> Answer {
        Answer::Cancel
    }
    #[cfg(windows)]
    pub fn pick_file(
        hwnd: isize,
        save: bool,
        title: &str,
        filter: &str,
        extension: &str,
    ) -> Option<String> {
        use windows_sys::Win32::UI::Controls::Dialogs::*;
        let mut path = vec![0u16; 32768];
        let filter = wide(filter);
        let title = wide(title);
        let ext = wide(extension);
        let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = hwnd as _;
        dialog.lpstrFile = path.as_mut_ptr();
        dialog.nMaxFile = path.len() as u32;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrTitle = title.as_ptr();
        dialog.lpstrDefExt = ext.as_ptr();
        dialog.Flags = OFN_EXPLORER
            | OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST
            | if save {
                OFN_OVERWRITEPROMPT
            } else {
                OFN_FILEMUSTEXIST
            };
        let ok = unsafe {
            if save {
                GetSaveFileNameW(&mut dialog)
            } else {
                GetOpenFileNameW(&mut dialog)
            }
        };
        (ok != 0).then(|| {
            let n = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            String::from_utf16_lossy(&path[..n])
        })
    }
    #[cfg(not(windows))]
    pub fn pick_file(_: isize, _: bool, _: &str, _: &str, _: &str) -> Option<String> {
        None
    }
    #[cfg(windows)]
    pub fn pick_folder(hwnd: isize, title: &str) -> Option<String> {
        use windows_sys::Win32::{System::Com::CoTaskMemFree, UI::Shell::*};
        let title = wide(title);
        let browse = BROWSEINFOW {
            hwndOwner: hwnd as _,
            lpszTitle: title.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            ..unsafe { std::mem::zeroed() }
        };
        let item = unsafe { SHBrowseForFolderW(&browse) };
        if item.is_null() {
            return None;
        }
        let mut path = vec![0_u16; 32768];
        let ok = unsafe { SHGetPathFromIDListEx(item, path.as_mut_ptr(), path.len() as u32, 0) };
        unsafe { CoTaskMemFree(item.cast()) };
        (ok != 0).then(|| {
            let n = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            String::from_utf16_lossy(&path[..n])
        })
    }
    #[cfg(not(windows))]
    pub fn pick_folder(_: isize, _: &str) -> Option<String> {
        None
    }
}

/// Comma-separated numbers from a script argument.
fn numbers(text: &str) -> Vec<f64> {
    text.split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

/// Sea-state presets (WMO/Douglas sea state with typical periods and winds).
const SEA_PRESETS: [(&str, f64, f64, f64); 9] = [
    ("Glassy calm", 0.0, 4.0, 0.5),
    ("Calm, rippled", 0.1, 3.0, 3.0),
    ("Smooth", 0.3, 4.0, 5.0),
    ("Slight", 0.9, 5.5, 8.0),
    ("Moderate", 1.9, 7.0, 11.0),
    ("Rough", 3.2, 8.5, 15.0),
    ("Very rough", 5.0, 10.0, 19.0),
    ("High", 7.5, 12.0, 24.0),
    ("Very high (storm)", 11.0, 14.0, 30.0),
];
/// Water clarity presets: optical attenuation (1/m).
const CLARITY_PRESETS: [(&str, f64); 4] = [
    ("Crystal clear (tropical)", 0.02),
    ("Clear ocean", 0.035),
    ("Coastal", 0.08),
    ("Turbid harbour", 0.25),
];

impl State {
    /// Quick environment controls over the 3D viewport: camera views, sea
    /// state, time of day, clouds and water clarity.
    fn quick_bar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        self.quick_bottom = rect.top();
        if rect.width() < 420.0 || self.model.document.is_none() {
            return;
        }
        if let Some(rx) = &self.live_sea
            && let Ok(result) = rx.try_recv()
        {
            self.live_sea = None;
            match result {
                Ok((conditions, source)) => {
                    let mut next = self.scene().clone();
                    for note in pelagic::metocean::apply(&mut next, &conditions, &source) {
                        self.log(note);
                    }
                    // The sun where it will be at that hour.
                    if let Some(unix) = pelagic::metocean::parse_utc(&conditions.time) {
                        let (day, hours) = super::globe::utc_day_hours(unix);
                        let (e, a) = pelagic_render_gpu::sky::solar_position(
                            next.provenance.origin_latitude,
                            next.provenance.origin_longitude,
                            day,
                            hours,
                        );
                        next.sun_elevation = e.clamp(-6.0, 90.0);
                        next.sun_azimuth = a;
                    }
                    let ahead = self.forecast_ahead.unwrap_or(0);
                    self.apply(
                        next,
                        &format!(
                            "Forecast for {} UTC (+{ahead} h) applied.",
                            conditions.time.replace('T', " ")
                        ),
                    );
                }
                Err(e) => self.log(format!("Forecast unavailable: {e}")),
            }
        }
        if self.live_sea.is_some() || self.live_rx.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        if std::mem::take(&mut self.auto_live) {
            self.live_on = true;
            self.live_at = None;
        }
        let scene = self.scene().clone();
        let (lat, lon) = if scene.provenance.origin_latitude != 0.0
            || scene.provenance.origin_longitude != 0.0
        {
            (
                scene.provenance.origin_latitude,
                scene.provenance.origin_longitude,
            )
        } else {
            (20.0, 0.0)
        };
        let bar = egui::Area::new(egui::Id::NULL.with("viewport-quick-bar"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.right_top() + Vec2::new(-8.0, 6.0))
            .pivot(Align2::RIGHT_TOP)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(Color32::from_black_alpha(150))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.menu_button("Environment", |ui| {
                        ui.set_max_width((rect.width() - 24.0).min(540.0));
                        ui.horizontal_wrapped(|ui| {
                            ui.menu_button("Views", |ui| {
                                let views = pelagic::landscape::viewpoints(
                                    scene.grid.as_deref(),
                                    scene.habitat.as_deref(),
                                    scene.bottom_depth,
                                );
                                for v in views {
                                    if ui.button(v.name).clicked() {
                                        let mut place = scene.clone();
                                        place.camera = v.eye;
                                        place.target = v.target;
                                        self.cam = FlyCam::from_scene(&place);
                                        self.view_dirty = true;
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Jump the camera to a viewpoint of this scene");
                            ui.separator();
                            let shown = self.fauna.species.len();
                            let life_label = if self.fauna_recorded > 0 {
                                format!("Life here · {} species recorded", self.fauna_recorded)
                            } else {
                                "Life here".to_string()
                            };
                            ui.menu_button(life_label, |ui| {
                                ui.set_min_width(360.0);
                                if self.fauna_rx.is_some() {
                                    ui.horizontal(|ui| {
                                        ui.spinner();
                                        ui.label("Looking up the species recorded here (OBIS)…");
                                    });
                                } else if self.fauna_recorded == 0 && shown > 0 {
                                    ui.label("No occurrence records found here (or a template scene): representative tropical species are shown.");
                                } else if shown == 0 {
                                    ui.label("No modelled species recorded here.");
                                } else {
                                    ui.label(format!(
                                        "{} species have records within ~50 km; the {shown} most recorded that Pelagic has models for swim around you:",
                                        self.fauna_recorded
                                    ));
                                }
                                egui::Grid::new("life-grid").striped(true).show(ui, |ui| {
                                    for sp in &self.fauna.species {
                                        ui.label(RichText::new(&sp.common).strong());
                                        ui.label(RichText::new(&sp.scientific).italics());
                                        ui.label(RichText::new(&sp.family).small().color(theme::MUTED));
                                        ui.label(
                                            RichText::new(if sp.records > 0 {
                                                format!("{} records", sp.records)
                                            } else {
                                                String::new()
                                            })
                                            .small(),
                                        );
                                        ui.end_row();
                                    }
                                });
                                ui.separator();
                                ui.label(
                                    RichText::new("Which species live here comes from OBIS occurrence records (surveys and sightings over many years). Each model has its family's body shape and the species' known colouring where catalogued (else its genus' or family's); size is a typical adult. Where each animal swims is simulated, not tracked.")
                                        .small()
                                        .color(theme::MUTED),
                                );
                            })
                            .response
                            .on_hover_text("Marine species recorded around the camera");
                            ui.separator();
                            let current = SEA_PRESETS
                                .iter()
                                .min_by(|a, b| {
                                    (a.1 - scene.wave_height)
                                        .abs()
                                        .total_cmp(&(b.1 - scene.wave_height).abs())
                                })
                                .map_or("Sea", |p| p.0);
                            let sea = egui::ComboBox::from_id_salt("quick-sea")
                                .selected_text(format!("Sea: {current} · Hs {:.1} m", scene.wave_height))
                                .width(210.0)
                                .show_ui(ui, |ui| {
                                    for (label, hs, tp, wind) in SEA_PRESETS {
                                        let text = format!("{label} · Hs {hs} m · {tp} s · wind {wind} m/s");
                                        if ui
                                            .selectable_label(label == current, text)
                                            .clicked()
                                        {
                                            let mut next = scene.clone();
                                            next.wave_height = hs;
                                            next.wave_period = tp;
                                            next.wind_speed = wind;
                                            next.sea_source = String::new();
                                            if hs.hypot(next.swell_height) > next.bottom_depth * 0.25 {
                                                self.log(format!(
                                                    "{label} needs water at least {:.0} m deep for the linear wave model; this scene is {:.0} m.",
                                                    hs * 4.0,
                                                    next.bottom_depth
                                                ));
                                            } else {
                                                self.apply(next, &format!("Sea state: {label}."));
                                            }
                                        }
                                    }
                                });
                            super::a11y::name(&sea.response, "Sea state");
                            let real = real_place(&scene);
                            let live_text = if self.live_on {
                                RichText::new("● Live").color(Color32::from_rgb(255, 95, 80)).strong()
                            } else {
                                RichText::new("○ Live")
                            };
                            let live = ui
                                .add_enabled(real, egui::Button::new(live_text).selected(self.live_on))
                                .on_hover_text("Live: the sea and weather here now (nearest buoy and airport observations first, else the models' value for this moment), the sun's real position, and live ships (AIS). Refreshes every 10 minutes.")
                                .on_disabled_hover_text("Only for scenes created from a real place on the Earth tab");
                            super::a11y::name(&live, "Live conditions");
                            if live.clicked() {
                                self.live_on = !self.live_on;
                                self.live_at = None;
                                if self.live_on {
                                    self.forecast_ahead = None;
                                }
                            }
                            if self.live_rx.is_some() || self.live_sea.is_some() {
                                ui.spinner();
                            }
                            let forecast_label = match self.forecast_ahead {
                                Some(h) => format!("Forecast +{h} h"),
                                None => "Forecast".to_string(),
                            };
                            let forecast = ui
                                .add_enabled_ui(real, |ui| {
                                    ui.menu_button(forecast_label, |ui| {
                                        ui.label(RichText::new("Show the predicted sea and weather (Open-Meteo models). Live switches off.").small().color(theme::MUTED));
                                        for h in [1u32, 3, 6, 12, 24, 48, 72, 120] {
                                            let text = if h < 24 { format!("In {h} hours") } else { format!("In {} days", h / 24) };
                                            if ui.selectable_label(self.forecast_ahead == Some(h), text).clicked() {
                                                self.forecast_ahead = Some(h);
                                                self.live_on = false;
                                                let (send, receive) = std::sync::mpsc::channel();
                                                std::thread::spawn(move || {
                                                    let days = (h / 24 + 2).min(8);
                                                    let result = crate::forecast::point(lat, lon, days).map(|f| {
                                                        let unix = std::time::SystemTime::now()
                                                            .duration_since(std::time::UNIX_EPOCH)
                                                            .map(|d| d.as_secs() as i64)
                                                            .unwrap_or(0)
                                                            + h as i64 * 3600;
                                                        let i = f.index_at(&pelagic::metocean::utc_hour(unix));
                                                        (f.hours[i].clone(), f.source.clone())
                                                    });
                                                    let _ = send.send(result);
                                                });
                                                self.live_sea = Some(receive);
                                                ui.close();
                                            }
                                        }
                                    })
                                })
                                .inner;
                            forecast.response.on_hover_text("Predicted conditions for a later hour or day");
                            ui.separator();
                            ui.label("Time");
                            let time = ui
                                .add(
                                    egui::Slider::new(&mut self.time_of_day, 0.0..=24.0)
                                        .step_by(0.25)
                                        .show_value(false),
                                )
                                .on_hover_text("Local solar time: the sun follows its real path at this scene's latitude today");
                            ui.label(format!(
                                "{:02}:{:02}",
                                self.time_of_day as u32 % 24,
                                ((self.time_of_day.fract()) * 60.0).round() as u32 % 60
                            ));
                            let clouds_now = self.env_preview.map_or(scene.clouds, |p| p.2);
                            let mut clouds = clouds_now;
                            ui.label("Clouds");
                            let cloud = ui
                                .add(egui::Slider::new(&mut clouds, 0.0..=1.0).show_value(false))
                                .on_hover_text("Cloud cover");
                            if time.changed() || cloud.changed() {
                                let unix = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs() as i64)
                                    .unwrap_or(0);
                                let (day, _) = super::globe::utc_day_hours(unix);
                                let (e, a) = if time.changed() {
                                    let utc = (self.time_of_day - lon / 15.0).rem_euclid(24.0);
                                    pelagic_render_gpu::sky::solar_position(lat, lon, day, utc)
                                } else {
                                    self.env_preview
                                        .map_or((scene.sun_elevation, scene.sun_azimuth), |p| (p.0, p.1))
                                };
                                self.env_preview = Some((e.clamp(-6.0, 90.0), a, clouds));
                                self.view_dirty = true;
                            }
                            let released = (time.drag_stopped() || cloud.drag_stopped())
                                || ((time.changed() || cloud.changed())
                                    && !time.dragged()
                                    && !cloud.dragged());
                            if released && let Some((e, a, c)) = self.env_preview.take() {
                                let mut next = scene.clone();
                                next.sun_elevation = e;
                                next.sun_azimuth = a;
                                next.clouds = c;
                                if let Err(err) = crate::editor::edit(&mut self.model, next, false) {
                                    self.log(format!("Edit refused: {err}"));
                                }
                                self.view_dirty = true;
                            }
                            ui.separator();
                            let water = CLARITY_PRESETS
                                .iter()
                                .min_by(|a, b| {
                                    (a.1.ln() - scene.attenuation.max(1e-4).ln())
                                        .abs()
                                        .total_cmp(&(b.1.ln() - scene.attenuation.max(1e-4).ln()).abs())
                                })
                                .map_or("Water", |p| p.0);
                            let clarity = egui::ComboBox::from_id_salt("quick-water")
                                .selected_text(format!("Water: {water}"))
                                .width(190.0)
                                .show_ui(ui, |ui| {
                                    for (label, attenuation) in CLARITY_PRESETS {
                                        if ui.selectable_label(label == water, label).clicked() {
                                            let mut next = scene.clone();
                                            next.attenuation = attenuation;
                                            self.apply(next, &format!("Water clarity: {label}."));
                                        }
                                    }
                                });
                            super::a11y::name(&clarity.response, "Water clarity");
                        });
                        });
                    });
            });
        self.quick_bottom = bar.response.rect.bottom();
    }
}

/// Cached default real sea opened at startup.
const STARTUP_SEA: &str = "startup-sea.pelagic";

/// The scene was built from a real place on Earth.
fn real_place(scene: &Scene) -> bool {
    scene.terrain == 3
        && (scene.provenance.origin_latitude != 0.0 || scene.provenance.origin_longitude != 0.0)
}
/// Plain-language guide for a tool tab: what it answers, three steps and a
/// one-click example (runs the tool with its defaults).
struct Guide {
    title: &'static str,
    question: &'static str,
    steps: [&'static str; 3],
    who: &'static str,
    example: &'static [&'static str],
}

fn guide(tab: Tab) -> Option<Guide> {
    Some(match tab {
        Tab::Trials => Guide {
            title: "Ship trials",
            question: "How does this vessel handle? The tests a shipyard runs before delivery, done in simulation.",
            steps: [
                "Pick the vessel in the Outliner (or keep the reference vehicle).",
                "Tick the trials you want: speed, turning circle, zig-zag, stopping, roll decay, waves.",
                "Press Run. Charts and a pass/fail table against the IMO manoeuvring standards appear below.",
            ],
            who: "Naval architects, vehicle designers, test engineers.",
            example: &["select-reference", "trials-run"],
        },
        Tab::Seakeeping => Guide {
            title: "Seakeeping",
            question: "Is it safe and comfortable to go out in these waves? Motions, sickness, slamming and deck wetness.",
            steps: [
                "Pick the vessel and its speed.",
                "Set the waves by hand, or use the live forecast for the scene's place.",
                "Press Run. See the wave heights where each limit is exceeded and a go/no-go outlook for the coming days.",
            ],
            who: "Operators planning a voyage or a launch and recovery.",
            example: &["select-reference", "seakeeping-run"],
        },
        Tab::Sar => Guide {
            title: "Search and rescue",
            question: "Someone went overboard or a boat is adrift. Where has it drifted, and where should we search first?",
            steps: [
                "Set the last known position (Earth tab, right-click: Search and rescue here).",
                "Choose what is missing (person in water, life raft, dinghy) and how long ago.",
                "Press Run. A drift cloud from live wind and current and ranked search patterns appear, ready to export to GPS.",
            ],
            who: "Rescue coordinators and trainers.",
            example: &["sar-run"],
        },
        Tab::Survey => Guide {
            title: "Seabed survey",
            question: "How do we map this seabed to the international (IHO) standard, and did the survey meet it?",
            steps: [
                "Choose the area, the sonar (multibeam or side-scan) and the IHO order.",
                "Press Plan for survey lines sized to the depth.",
                "Press Run to fly it with vehicle physics: coverage map, gaps and depth accuracy.",
            ],
            who: "Hydrographic surveyors, AUV and USV teams.",
            example: &["survey-run"],
        },
        Tab::Ship => Guide {
            title: "Ship handling (rules of the road)",
            question: "Can you avoid a collision the way the COLREGs require? Practise at the helm or watch the autopilot.",
            steps: [
                "Pick an encounter: head-on, crossing, overtaking, several ships.",
                "Steer with the helm controls, or switch on the autopilot.",
                "Get a rule-by-rule debrief: closest approach, early and substantial action.",
            ],
            who: "Watch officers, cadets, autonomy developers.",
            example: &[
                "select-vessel",
                "ship-start:0",
                "ship-autopilot",
                "ship-run:600",
            ],
        },
        Tab::Inspection => Guide {
            title: "Underwater inspection",
            question: "How should a camera ROV fly to photograph harbour pilings or a hull sharply enough?",
            steps: [
                "Choose the target (harbour pilings or a vessel's hull) and the image sharpness needed.",
                "Press Plan for passes at the right distance for the water clarity.",
                "Press Run to fly it with the ROV: coverage and closest approach to the structure.",
            ],
            who: "Port and hull inspection teams, ROV pilots.",
            example: &["inspect-run"],
        },
        Tab::Spill => Guide {
            title: "Oil spill drift",
            question: "Where will spilled oil go, how much evaporates and when does it reach the shore?",
            steps: [
                "Set the release point (Earth tab, right-click: Oil spill here).",
                "Choose the oil type and amount.",
                "Press Run. See the slick drift on live wind and current, the oil budget over time and the first landfall.",
            ],
            who: "Spill responders and planners.",
            example: &["spill-run"],
        },
        _ => return None,
    })
}

impl State {
    /// The guide card at the top of a tool tab (collapsible, remembered).
    fn tool_guide(&mut self, ui: &mut egui::Ui, tab: Tab) {
        let Some(g) = guide(tab) else {
            return;
        };
        let id = egui::Id::NULL.with(("tool-guide", tab as u8));
        let roomy = ui.available_height() > 620.0;
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, roomy));
        egui::Frame::new()
            .fill(theme::ACCENT.gamma_multiply(0.12))
            .stroke(egui::Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.5)))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(g.title).strong().size(16.0).color(theme::STRONG));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(if open { "Hide guide" } else { "What is this?" })
                            .clicked()
                        {
                            open = !open;
                        }
                        {
                            let busy = self.job.is_some() || !self.script_queue.is_empty();
                            let example = ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(RichText::new("▶ Show me an example").strong())
                                        .fill(theme::ACCENT.gamma_multiply(0.5)),
                                )
                                .on_hover_text("Runs this tool now with sensible defaults on the current scene");
                            super::a11y::name(&example, "Show me an example");
                            if example.clicked() {
                                self.script_queue.extend(g.example.iter().map(|s| s.to_string()));
                            }
                        }
                    });
                });
                if !open {
                    ui.label(RichText::new(g.question).small().color(theme::MUTED));
                }
                if open {
                    ui.label(RichText::new(g.question).color(theme::STRONG));
                    for (i, step) in g.steps.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(format!("{}", i + 1)).strong().color(theme::ACCENT));
                            ui.label(*step);
                        });
                    }
                    ui.label(RichText::new(format!("Who uses it: {}", g.who)).small().color(theme::MUTED));
                }
            });
        ui.data_mut(|d| d.insert_persisted(id, open));
        ui.add_space(6.0);
    }
}

/// A live AIS ship near the scene: last report and how to draw it.
#[derive(Clone, Debug)]
struct LiveShip {
    name: String,
    lat: f64,
    lon: f64,
    /// Unix time of the report.
    time: i64,
    sog: f64,
    cog: f64,
    heading: f64,
    length: f64,
    kind: usize,
    category: &'static str,
}

/// Fresh satellite colours for a scene and its backdrop.
type SceneImagery = (
    Option<pelagic::habitat::Imagery>,
    Option<pelagic::habitat::Imagery>,
);

/// A smooth camera flight between two views.
#[derive(Clone, Copy, Debug)]
struct Flight {
    from: Vec3,
    to: Vec3,
    yaw: (f64, f64),
    pitch: (f64, f64),
    start: Instant,
    seconds: f64,
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// Ground (terrain or seabed) elevation at a scene point: the scene grid,
/// its backdrop, then the streamed world, else the scene's own seabed.
fn ground_at(
    scene: &Scene,
    ocean: &pelagic::ocean::Ocean,
    world: Option<&super::world::World>,
    x: f64,
    y: f64,
) -> f64 {
    let inside = |g: &pelagic::terrain::HeightGrid| {
        let w = (g.columns - 1) as f64 * g.spacing;
        let h = (g.rows - 1) as f64 * g.spacing;
        x >= g.origin[0] && x <= g.origin[0] + w && y >= g.origin[1] && y <= g.origin[1] + h
    };
    if !real_place(scene) {
        return ocean.bottom.elevation(x, y);
    }
    for g in [scene.grid.as_deref(), scene.backdrop.as_deref()]
        .into_iter()
        .flatten()
    {
        if inside(g) {
            return pelagic::ocean::Bathymetry::elevation(g, x, y);
        }
    }
    world
        .and_then(|w| w.elevation(x, y))
        .unwrap_or(-scene.bottom_depth)
}

/// Trees on vegetated land around `eye`: where the ground is above the sea
/// and the satellite colour is vegetation green. Coconut palms on low
/// tropical land, conifers at high latitude or altitude, broadleaf trees
/// elsewhere. Mesh indices 0-2 (`world::tree_mesh`), nearest first.
fn plant_trees(
    scene: &Scene,
    ocean: &pelagic::ocean::Ocean,
    world: Option<&super::world::World>,
    eye: Vec3,
    lat: f64,
    cap: usize,
) -> Vec<pelagic_render_gpu::Animal> {
    let hash = |a: i64, b: i64, salt: u64| {
        let mut z = (a as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ (b as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
            ^ salt;
        z ^= z >> 31;
        z = z.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z ^= z >> 29;
        (z >> 11) as f64 / (1u64 << 53) as f64
    };
    let colour = |x: f64, y: f64| {
        scene
            .imagery
            .as_deref()
            .and_then(|i| i.linear(x, y))
            .or_else(|| {
                scene
                    .backdrop_imagery
                    .as_deref()
                    .and_then(|i| i.linear(x, y))
            })
            .or_else(|| world.and_then(|w| w.imagery(x, y)))
    };
    let mut out: Vec<(f64, pelagic_render_gpu::Animal)> = vec![];
    // Finer near the camera, coarser (each cell more likely planted) out to
    // 5 km, so distant islands show their tree lines.
    for (ring, (step, inner, outer, density)) in
        [(14.0, 0.0, 1_200.0, 0.35), (40.0, 1_200.0, 5_000.0, 0.9)]
            .into_iter()
            .enumerate()
    {
        let n = (outer / step) as i64;
        let (cx, cy) = ((eye.x / step).floor() as i64, (eye.y / step).floor() as i64);
        let salt = ring as u64 * 101;
        for j in -n..=n {
            for i in -n..=n {
                let (gx, gy) = (cx + i, cy + j);
                let x = (gx as f64 + hash(gx, gy, 2 + salt)) * step;
                let y = (gy as f64 + hash(gx, gy, 3 + salt)) * step;
                let d = (x - eye.x).hypot(y - eye.y);
                if d < inner || d > outer || hash(gx, gy, 1 + salt) > density {
                    continue;
                }
                let z = ground_at(scene, ocean, world, x, y);
                if !(0.8..2_500.0).contains(&z) {
                    continue;
                }
                let Some(c) = colour(x, y) else { continue };
                let green = c[1] > c[0] * 1.08 && c[1] > c[2] * 1.1 && c[1] < 0.45;
                if !green {
                    continue;
                }
                let r = hash(gx, gy, 4 + salt);
                let (kind, height) = if lat.abs() < 26.0 && z < 25.0 {
                    (0, 14.0 + 10.0 * r)
                } else if lat.abs() > 48.0 || z > 1_500.0 {
                    (2, 10.0 + 15.0 * r)
                } else {
                    (1, 8.0 + 10.0 * r)
                };
                let yaw = hash(gx, gy, 5 + salt) * std::f64::consts::TAU;
                let tint = (0.85 + 0.25 * hash(gx, gy, 6 + salt)) as f32;
                out.push((
                    d,
                    pelagic_render_gpu::Animal {
                        mesh: kind,
                        position: Vec3::new(x, y, z - 0.2),
                        heading: Vec3::new(yaw.cos(), yaw.sin(), 0.0),
                        length: height,
                        tint: [tint, tint, tint],
                        ..Default::default()
                    },
                ));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.truncate(cap);
    out.into_iter().map(|(_, a)| a).collect()
}

/// Compass point for a bearing (degrees).
fn compass_point(bearing: f64) -> &'static str {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    POINTS[((bearing.rem_euclid(360.0) / 22.5).round() as usize) % 16]
}

/// Camera speed presets (m/s).
const SPEEDS: [(&str, f64); 7] = [
    ("Swim", 1.5),
    ("Boat", 10.0),
    ("Fast boat", 25.0),
    ("Helicopter", 70.0),
    ("Plane", 250.0),
    ("Jet", 1_000.0),
    ("Rocket", 8_000.0),
];
