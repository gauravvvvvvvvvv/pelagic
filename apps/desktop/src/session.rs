//! Desktop session state uses the same simulation and renderer as headless runs.
use pelagic::{
    World,
    math::Vec3,
    render::{Frame, Renderer, SoftwareRenderer},
};

const DT: f64 = 1.0 / 60.0;
/// When false (GPU editor shell), sessions skip CPU frame rendering; the shell
/// renders the shared scene on the GPU from `view_snapshot`/`view_ocean`.
pub static CPU_FRAMES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static REVISIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn next_revision() -> u64 {
    REVISIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
fn cpu_frames() -> bool {
    CPU_FRAMES.load(std::sync::atomic::Ordering::Relaxed)
}
pub struct Session {
    /// Changes whenever the displayed world state changes (step, reset,
    /// replay, preview). Renderers rebuild geometry only when it changes.
    pub revision: u64,
    /// Changes only when the ocean description shown (seabed, structures,
    /// materials) may have changed, not on simulation ticks. Renderers cache
    /// static geometry against it.
    pub world_revision: u64,
    pub capture: Option<pelagic::experiment::Recording>,
    pub observations: Vec<pelagic::instruments::Observation>,
    pub recording_full: bool,
    pub display: Option<pelagic::Snapshot>,
    pub sensor_view: bool,
    pub selected: Option<u64>,
    sensors: pelagic::instruments::SensorConfig,
    suite: pelagic::instruments::Suite,
    pub world: World,
    pub frame: Frame,
    pub running: bool,
    pub render_ms: f64,
    pub wall_time_lost: f64,
    pub backend_notice: String,
    gpu: Option<Box<pelagic_render_gpu::GpuRenderer>>,
    shared_preview: bool,
    preview_world: Option<Box<World>>,
    renderer: SoftwareRenderer,
    remainder: f64,
}
impl Session {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::from_scene(&crate::project::Scene::default())
    }
    pub fn from_scene(scene: &crate::project::Scene) -> Self {
        let mut renderer = SoftwareRenderer::new(512, 320).expect("reference viewport dimensions");
        renderer.quality = scene.quality;
        renderer.terrain_material = scene.terrain_material;
        renderer.sun_elevation = scene.sun_elevation.to_radians();
        renderer.camera = vec(scene.camera);
        renderer.target = vec(scene.target);
        let world = initial_world(scene);
        let start = std::time::Instant::now();
        let frame = if cpu_frames() {
            renderer.render(&world.ocean, &world.snapshot())
        } else {
            Frame {
                width: 1,
                height: 1,
                rgb: vec![0; 3],
            }
        };
        let render_ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut suite = pelagic::instruments::Suite::new(scene.sensors.clone());
        suite.update(&world, DT);
        let observations = suite.latest.clone();
        Self {
            revision: next_revision(),
            world_revision: next_revision(),
            capture: None,
            observations,
            suite,
            recording_full: false,
            display: None,
            sensor_view: false,
            selected: None,
            sensors: scene.sensors.clone(),
            world,
            frame,
            renderer,
            running: false,
            render_ms,
            wall_time_lost: 0.0,
            backend_notice: String::new(),
            gpu: None,
            shared_preview: false,
            preview_world: None,
            remainder: 0.0,
        }
    }
    #[cfg(test)]
    pub fn advance(&mut self, seconds: f64) {
        self.advance_view(seconds, true);
    }
    pub fn advance_view(&mut self, seconds: f64, render: bool) {
        if !self.running || !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        // Bound catch-up after a stalled/modal UI. Never change the physics dt.
        self.remainder += seconds.min(0.25);
        self.wall_time_lost += (seconds - 0.25).max(0.0);
        let mut stepped = false;
        while self.remainder + 1e-12 >= DT {
            self.tick();
            self.remainder = (self.remainder - DT).max(0.0);
            stepped = true;
        }
        if stepped && render {
            self.refresh();
        }
    }
    pub fn toggle(&mut self) {
        self.running = !self.running;
    }
    pub fn step(&mut self) {
        self.running = false;
        self.remainder = 0.0;
        self.tick();
        self.refresh();
    }
    pub fn reset_scene(&mut self, scene: &crate::project::Scene) {
        let dimensions = self.renderer.dimensions();
        let wireframe = self.renderer.wireframe;
        let gpu = self.gpu.take();
        let shared = self.shared_preview;
        *self = Self::from_scene(scene);
        self.gpu = gpu;
        self.shared_preview = shared;
        self.renderer
            .resize(dimensions[0], dimensions[1])
            .expect("previous viewport dimensions");
        self.renderer.wireframe = wireframe;
        if shared || dimensions != [512, 320] || wireframe {
            self.refresh();
        }
    }
    pub fn set_camera(&mut self, scene: &crate::project::Scene) {
        self.renderer.terrain_material = scene.terrain_material;
        self.renderer.quality = scene.quality;
        self.renderer.sun_elevation = scene.sun_elevation.to_radians();
        self.renderer.camera = vec(scene.camera);
        self.renderer.target = vec(scene.target);
        self.refresh();
    }
    pub fn resize(&mut self, width: usize, height: usize) {
        if self.renderer.dimensions() != [width, height] {
            let _ = self.renderer.resize(width, height);
            self.refresh();
        }
    }
    pub fn wireframe(&mut self, enabled: bool) {
        if enabled && self.gpu.take().is_some() {
            self.backend_notice = "Wireframe uses the CPU shared-scene preview.".into();
        }
        self.renderer.wireframe = enabled;
        self.refresh();
    }
    fn refresh(&mut self) {
        self.revision = next_revision();
        if !cpu_frames() {
            return;
        }
        let start = std::time::Instant::now();
        let snapshot = self.view_snapshot();
        let ocean = &self.preview_world.as_deref().unwrap_or(&self.world).ocean;
        let saved = (
            self.renderer.camera,
            self.renderer.target,
            self.renderer.vertical_fov,
            self.renderer.camera_up,
        );
        if self.sensor_view
            && let Some(body) = snapshot
                .vehicles
                .iter()
                .find(|p| p.id == self.sensors.entity)
                .or_else(|| snapshot.vehicles.first())
        {
            self.renderer.camera = body.position + body.orientation.rotate(self.sensors.mount);
            self.renderer.target =
                self.renderer.camera + body.orientation.rotate(Vec3::new(1.0, 0.0, -0.15));
            self.renderer.vertical_fov = self.sensors.camera_fov;
            self.renderer.camera_up = body.orientation.rotate(Vec3::new(0.0, 0.0, 1.0));
        }
        // Selection and editing handles belong to the studio overlay, including GPU.
        self.renderer.selected = None;
        self.frame = if self.shared_preview {
            match pelagic::render_scene::RenderScene::build(
                ocean,
                &snapshot,
                self.renderer.quality,
                self.renderer.terrain_material,
            ) {
                Ok(scene) => {
                    let result = self.gpu.as_mut().map(|gpu| {
                        let [w, h] = self.renderer.dimensions();
                        gpu.resize(w as u32, h as u32)?;
                        gpu.render(
                            &scene,
                            pelagic_render_gpu::View {
                                eye: self.renderer.camera,
                                target: self.renderer.target,
                                up: self.renderer.camera_up,
                                fov_y: self.renderer.vertical_fov,
                                sun_elevation: self.renderer.sun_elevation,
                                sun_azimuth: pelagic_render_gpu::LEGACY_SUN_AZIMUTH,
                                clouds: 0.0,
                                time: snapshot.time,
                                attenuation: ocean.attenuation,
                                underwater: self.renderer.camera.z
                                    < self
                                        .world
                                        .ocean
                                        .surface(
                                            self.renderer.camera.x,
                                            self.renderer.camera.y,
                                            snapshot.time,
                                        )
                                        .height,
                            },
                        )
                    });
                    match result {
                        Some(Ok(frame)) => frame,
                        Some(Err(error)) => {
                            self.gpu = None;
                            self.backend_notice =
                                format!("GPU preview failed; CPU fallback active: {error}");
                            self.renderer.render_shared(ocean, &snapshot, &scene)
                        }
                        None => self.renderer.render_shared(ocean, &snapshot, &scene),
                    }
                }
                Err(error) => {
                    self.gpu = None;
                    self.shared_preview = false;
                    self.backend_notice =
                        format!("Shared preview unavailable; legacy CPU active: {error}");
                    self.renderer.render(ocean, &snapshot)
                }
            }
        } else {
            self.renderer.render(ocean, &snapshot)
        };
        self.renderer.camera = saved.0;
        self.renderer.target = saved.1;
        self.renderer.vertical_fov = saved.2;
        self.renderer.camera_up = saved.3;
        self.render_ms = start.elapsed().as_secs_f64() * 1000.0;
    }
    pub fn backend_name(&self) -> &str {
        if self.gpu.is_some() {
            "GPU preview"
        } else if self.shared_preview {
            "CPU shared"
        } else {
            "CPU"
        }
    }
    pub fn view_snapshot(&self) -> pelagic::Snapshot {
        self.preview_world.as_ref().map_or_else(
            || {
                self.display
                    .clone()
                    .unwrap_or_else(|| self.world.snapshot())
            },
            |w| w.snapshot(),
        )
    }
    pub fn preview_scene(&mut self, scene: &crate::project::Scene) -> Result<(), String> {
        self.preview_world = Some(Box::new(pelagic::scene::world(scene)?));
        self.world_revision = next_revision();
        self.refresh();
        Ok(())
    }
    pub fn clear_preview(&mut self) {
        if self.preview_world.take().is_some() {
            self.world_revision = next_revision();
            self.refresh();
        }
    }
    pub fn editor_camera(&self, rect: [f32; 4]) -> crate::viewport_edit::Camera {
        let (mut eye, mut target, mut up, mut fov) = (
            self.renderer.camera,
            self.renderer.target,
            self.renderer.camera_up,
            self.renderer.vertical_fov,
        );
        let state = self.view_snapshot();
        if self.sensor_view
            && let Some(body) = state
                .vehicles
                .iter()
                .find(|p| p.id == self.sensors.entity)
                .or_else(|| state.vehicles.first())
        {
            eye = body.position + body.orientation.rotate(self.sensors.mount);
            target = eye + body.orientation.rotate(Vec3::new(1., 0., -0.15));
            up = body.orientation.rotate(Vec3::new(0., 0., 1.));
            fov = self.sensors.camera_fov;
        }
        crate::viewport_edit::Camera::new(rect, eye, target, up, fov)
    }
    pub fn view_ocean(&self) -> &pelagic::ocean::Ocean {
        &self.preview_world.as_deref().unwrap_or(&self.world).ocean
    }
    pub fn select_cpu(&mut self, shared: bool) {
        self.gpu = None;
        self.shared_preview = shared;
        self.backend_notice.clear();
        self.refresh();
    }
    pub fn install_gpu(&mut self, gpu: Box<pelagic_render_gpu::GpuRenderer>) {
        self.gpu = Some(gpu);
        self.shared_preview = true;
        self.renderer.wireframe = false;
        self.backend_notice.clear();
        self.refresh();
    }
    pub fn view(&mut self, sensor_view: bool, selected: Option<u64>) {
        if self.sensor_view != sensor_view || self.selected != selected {
            self.sensor_view = sensor_view;
            self.selected = selected;
            self.refresh();
        }
    }
    fn tick(&mut self) {
        self.revision = next_revision();
        if let Some(run) = &mut self.capture {
            run.capture_commands(&self.world);
        }
        self.display = None;
        self.world.step();
        self.suite.update(&self.world, DT);
        self.observations = self.suite.latest.clone();
        if self.world.tick().is_multiple_of(self.sensors.ticks(DT)) {
            let truth = self.world.snapshot();
            let velocities = self
                .world
                .vehicles()
                .iter()
                .map(|v| v.velocity)
                .collect::<Vec<_>>();
            if let Some(run) = &mut self.capture
                && run
                    .append(pelagic::experiment::RunFrame {
                        truth,
                        velocities,
                        observations: self.observations.clone(),
                    })
                    .is_err()
            {
                self.recording_full = true;
            }
        }
    }
    pub fn start_recording(&mut self, scene: &crate::project::Scene) {
        self.capture = Some(pelagic::experiment::Recording::new(
            scene.value().to_string(),
            DT,
        ));
        self.recording_full = false;
        if let Some(run) = &mut self.capture {
            run.checkpoint = Some(self.world.checkpoint());
            run.sensor_checkpoint = Some(self.suite.clone());
            run.capture_commands(&self.world);
        }
        let frame = pelagic::experiment::RunFrame {
            truth: self.world.snapshot(),
            velocities: self.world.vehicles().iter().map(|v| v.velocity).collect(),
            observations: self.observations.clone(),
        };
        self.capture
            .as_mut()
            .unwrap()
            .append(frame)
            .expect("empty run");
    }
    /// Leaves recorded-truth playback and shows the live world again.
    pub fn show_live(&mut self) {
        if self.display.take().is_some() {
            self.refresh();
        }
    }
    pub fn present(&mut self, snapshot: pelagic::Snapshot) {
        self.running = false;
        self.display = Some(snapshot);
        self.refresh();
    }
    pub fn camera(&self, scene: &crate::project::Scene) -> Frame {
        let snapshot = self
            .display
            .clone()
            .unwrap_or_else(|| self.world.snapshot());
        let mut renderer = SoftwareRenderer::new(320, 200).expect("sensor dimensions");
        if let Some(body) = snapshot
            .vehicles
            .iter()
            .find(|p| p.id == scene.sensors.entity)
            .or_else(|| snapshot.vehicles.first())
        {
            renderer.camera = body.position + body.orientation.rotate(scene.sensors.mount);
            renderer.target = renderer.camera + body.orientation.rotate(Vec3::new(1.0, 0.0, -0.15));
            renderer.camera_up = body.orientation.rotate(Vec3::new(0.0, 0.0, 1.0));
        } else {
            renderer.camera = vec(scene.camera);
            renderer.target = vec(scene.target);
        }
        renderer.quality = 0;
        renderer.vertical_fov = scene.sensors.camera_fov;
        renderer.sun_elevation = scene.sun_elevation.to_radians();
        renderer.terrain_material = scene.terrain_material;
        renderer.render(&self.world.ocean, &snapshot)
    }
}
fn vec(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}
fn initial_world(scene: &crate::project::Scene) -> World {
    pelagic::scene::world(scene).expect("validated scene")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_cpu_backend_switch_and_reset_preserve_their_contracts() {
        let scene = crate::project::Scene::robotics_demo();
        let mut session = Session::from_scene(&scene);
        session.resize(400, 250);
        let before = session.world.checkpoint();
        session.select_cpu(true);
        assert_eq!(session.backend_name(), "CPU shared");
        assert_eq!(session.world.checkpoint(), before);
        session.step();
        session.reset_scene(&scene);
        assert_eq!(session.backend_name(), "CPU shared");
        assert_eq!(session.world.tick(), 0);
        assert_eq!([session.frame.width, session.frame.height], [400, 250]);
        session.select_cpu(false);
        assert_eq!(session.backend_name(), "CPU");
        assert_eq!(session.world.checkpoint(), before);
    }
    #[test]
    fn live_recording_started_mid_run_reproduces_through_shared_sdk() {
        let scene = crate::project::Scene::robotics_demo();
        let mut session = Session::from_scene(&scene);
        session.running = true;
        session.advance_view(0.2, false);
        session.start_recording(&scene);
        session.advance_view(0.2, false);
        session.running = false;
        let run = session.capture.take().unwrap();
        assert!(run.frames.len() >= 3);
        pelagic::experiment::verify(&run, &std::sync::atomic::AtomicBool::new(false)).unwrap();
        let before = session.world.snapshot();
        session.present(run.frames[1].truth.clone());
        assert_eq!(session.world.snapshot(), before);
        assert_eq!(session.display.as_ref(), Some(&run.frames[1].truth));
    }
    #[test]
    fn display_changes_preserve_running_truth() {
        let mut scene = crate::project::Scene::default();
        let mut session = Session::from_scene(&scene);
        session.running = true;
        session.advance_view(DT * 10.0, false);
        let truth = session.world.snapshot();
        scene.quality = 1;
        scene.sun_elevation = 60.0;
        scene.terrain_material = 2;
        scene.camera[0] -= 2.0;
        session.set_camera(&scene);
        session.wireframe(true);
        session.resize(320, 200);
        assert_eq!(session.world.snapshot(), truth);
        assert!(session.running);
    }
    #[test]
    fn paused_step_play_and_reset_use_engine_state() {
        let mut session = Session::new();
        let initial = session.world.snapshot();
        session.advance(1.0);
        assert_eq!(session.world.tick(), 0);
        session.step();
        assert_eq!(session.world.tick(), 1);
        assert!(!session.running);
        session.toggle();
        session.advance(DT * 4.0);
        assert_eq!(session.world.tick(), 5);
        assert!(session.world.vehicles()[0].position.x > 0.0);
        session.reset_scene(&crate::project::Scene::default());
        assert_eq!(session.world.snapshot(), initial);
        assert!(!session.running);
        assert_eq!(session.frame.rgb.len(), 512 * 320 * 3);
    }
}
