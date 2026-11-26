use ab_glyph::{Font, FontArc, FontVec, PxScale, ScaleFont, VariableFont, point};
use std::{
    fs::File,
    io::{BufWriter, Cursor},
};
#[path = "editor_ui.rs"]
mod editor_ui;
#[path = "viewport_ui.rs"]
mod viewport_ui;
pub use crate::layout::EditorLayout;

pub const INTRO_SECONDS: f64 = crate::sound::DURATION + 0.15;
pub const BG: u32 = 0x000000;
pub const TEXT: u32 = 0xffffff;
const MUTED: u32 = 0xa1a1a1;
const ACCENT: u32 = 0xffffff;

#[derive(Default)]
pub struct Model {
    pub transform_tool: crate::viewport_edit::Tool,
    pub transform_snap: crate::viewport_edit::Snap,
    pub transform_preview: Option<crate::project::Scene>,
    pub selection_empty: bool,
    pub layout: crate::layout::LayoutSettings,
    pub viewport_maximized: bool,
    pub content_tab: usize,
    pub content_filter: usize,
    pub content_page: usize,
    pub content_collapsed: bool,
    pub outliner_scroll: usize,
    pub outliner_filter: String,
    pub messages: std::collections::VecDeque<String>,
    pub(crate) last_logged_notice: String,
    pub hover: Option<usize>,
    pub focus: Option<usize>,
    pub pressed: Option<usize>,
    pub about: bool,
    pub project: Option<String>,
    pub project_path: Option<String>,
    pub muted: bool,
    pub workspace: bool,
    pub inspect_ocean: bool,
    pub session: Option<crate::session::Session>,
    pub dark_mode: bool,
    pub document: Option<crate::project::Document>,
    pub new_project: bool,
    pub new_name: String,
    pub new_folder: String,
    pub new_vehicle: bool,
    pub editing: Option<(usize, String)>,
    pub notice: String,
    pub recovery_available: bool,
    pub recent: Vec<String>,
    pub input_cursor: usize,
    pub input_selected: bool,
    pub storage_root: Option<std::path::PathBuf>,
    pub environment_tab: usize,
    pub viewport_mode: u8,
    pub wireframe: bool,
    pub importing: bool,
    pub inspect_structure: bool,
    pub reduced_motion: bool,
    pub inspector_page: usize,
    pub selected_entity: Option<usize>,
    /// Content Browser library selection used by Place / Assign / Remove.
    pub selected_library: Option<crate::library::Item>,
    pub run: Option<std::sync::Arc<pelagic::experiment::Recording>>,
    pub playback: Option<usize>,
    pub job_running: bool,
    pub job_progress: String,
    pub tool_tab: usize,
    pub batch_seconds: f64,
    pub frame_update_ms: f64,
    pub last_frame_ms: f64,
    pub backend_generation: u64,
    pub pending_saves: Vec<std::sync::mpsc::Receiver<crate::backups::Reply>>,
    pub failed_runs: Vec<std::sync::Arc<pelagic::experiment::Recording>>,
}

impl Model {
    pub fn record_notice(&mut self) {
        if self.notice == self.last_logged_notice {
            return;
        }
        if !self.notice.is_empty() {
            if self.messages.len() == 128 {
                self.messages.pop_front();
            }
            self.messages
                .push_back(self.notice.chars().take(512).collect());
        }
        self.last_logged_notice.clone_from(&self.notice);
    }
}

#[derive(Clone, Copy)]
pub struct Hit {
    pub id: usize,
    pub rect: [f32; 4],
}

pub struct AccessibleControl {
    pub id: usize,
    pub rect: [f32; 4],
    pub label: String,
    pub value: Option<String>,
    pub disabled: bool,
}

pub struct Painter {
    pub pixels: Vec<u32>,
    pub width: usize,
    pub height: usize,
    pub hits: Vec<Hit>,
    pub accessible: Vec<AccessibleControl>,
    pub dpi_scale: f32,
    scale: f32,
    font: FontArc,
    logo: Vec<u8>,
    logo_w: usize,
    logo_h: usize,
    dark: bool,
    hover_amount: [f32; 512],
    press_amount: [f32; 512],
    viewport_area: Option<[f32; 4]>,
    outliner_area: Option<[f32; 4]>,
    pub editor_layout: Option<EditorLayout>,
    pub viewport_camera: Option<crate::viewport_edit::Camera>,
    pub gizmo: Option<crate::viewport_edit::Gizmo>,
    pub viewport_actors: Vec<(usize, crate::viewport_edit::Actor)>,
    sample: Option<pelagic::render::Frame>,
    thumbs: std::collections::HashMap<ThumbKey, std::sync::Arc<pelagic::render::Frame>>,
}

/// Identifies a rendered thumbnail: mesh identity/shape, surface values and
/// physical pixel size. Cached in memory only; rebuilt after restart.
type ThumbKey = (usize, usize, usize, u64, [u8; 3], u64, u64, usize, usize);

/// Look used when an actor has no assigned material.
pub fn default_material() -> pelagic::assets::Material {
    pelagic::assets::Material::new(0, "Default", [157, 173, 184], 0.42, 0.4)
}

impl Painter {
    pub fn new() -> Self {
        let mut face = FontVec::try_from_vec(include_bytes!("../assets/Geist.ttf").to_vec())
            .expect("bundled font");
        face.set_variation(b"wght", 400.0);
        let font = FontArc::new(face);
        let mut decoder =
            png::Decoder::new(Cursor::new(include_bytes!("../assets/pelagic-logo.png")));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().expect("bundled logo");
        let mut data = vec![0; reader.output_buffer_size().expect("logo size")];
        let info = reader.next_frame(&mut data).expect("logo pixels");
        let channels = info.color_type.samples();
        let mut bounds = [info.width as usize, info.height as usize, 0, 0];
        for y in 0..info.height as usize {
            for x in 0..info.width as usize {
                let i = (y * info.width as usize + x) * channels;
                if data[i] > 100 {
                    bounds[0] = bounds[0].min(x);
                    bounds[1] = bounds[1].min(y);
                    bounds[2] = bounds[2].max(x);
                    bounds[3] = bounds[3].max(y);
                }
            }
        }
        let logo_w = bounds[2] - bounds[0] + 1;
        let logo_h = bounds[3] - bounds[1] + 1;
        let mut logo = vec![0; logo_w * logo_h];
        for y in 0..logo_h {
            for x in 0..logo_w {
                logo[y * logo_w + x] =
                    data[((y + bounds[1]) * info.width as usize + x + bounds[0]) * channels];
            }
        }
        Self {
            pixels: Vec::new(),
            width: 0,
            height: 0,
            hits: Vec::new(),
            accessible: Vec::new(),
            dpi_scale: 1.0,
            scale: 1.0,
            font,
            logo,
            logo_w,
            logo_h,
            dark: false,
            hover_amount: [0.0; 512],
            press_amount: [0.0; 512],
            viewport_area: None,
            outliner_area: None,
            editor_layout: None,
            viewport_camera: None,
            gizmo: None,
            viewport_actors: vec![],
            sample: None,
            thumbs: Default::default(),
        }
    }
    /// Draws a cached software thumbnail of a mesh (or material sphere).
    fn thumbnail(
        &mut self,
        mesh: Option<&std::sync::Arc<pelagic::assets::Mesh>>,
        stretch: [f64; 3],
        material: &pelagic::assets::Material,
        rect: [f32; 4],
    ) {
        let w = ((rect[2] * self.scale).round() as usize).clamp(8, pelagic::thumbnail::MAX_SIZE);
        let h = ((rect[3] * self.scale).round() as usize).clamp(8, pelagic::thumbnail::MAX_SIZE);
        let key = (
            mesh.map_or(0, |m| std::sync::Arc::as_ptr(m) as usize),
            mesh.map_or(0, |m| m.vertices.len()),
            mesh.map_or(0, |m| m.triangles.len()),
            mesh.and_then(|m| m.vertices.first()).map_or(0, |v| {
                v.x.to_bits() ^ v.y.to_bits().rotate_left(21) ^ v.z.to_bits().rotate_left(42)
            }),
            material.base_color,
            material.roughness.to_bits()
                ^ stretch[0].to_bits().rotate_left(7)
                ^ stretch[1].to_bits().rotate_left(19)
                ^ stretch[2].to_bits().rotate_left(37),
            material.metallic.to_bits(),
            w,
            h,
        );
        if !self.thumbs.contains_key(&key) {
            if self.thumbs.len() >= 96 {
                self.thumbs.clear();
            }
            let frame = pelagic::thumbnail::render(mesh.map(|m| &**m), stretch, material, w, h);
            self.thumbs.insert(key, std::sync::Arc::new(frame));
        }
        let frame = self.thumbs[&key].clone();
        self.blit_frame(&frame, rect);
    }
    pub fn paint(&mut self, width: usize, height: usize, model: &Model, intro: Option<f64>) {
        self.width = width;
        self.height = height;
        self.scale = width as f32 / 1100.0;
        if model.workspace && !model.new_project && intro.is_none() {
            self.scale = self.dpi_scale.max(0.5);
        }
        self.viewport_area = None;
        self.outliner_area = None;
        self.editor_layout = None;
        self.viewport_camera = None;
        self.gizmo = None;
        self.viewport_actors.clear();
        self.dark = model.dark_mode || intro.is_some();
        let background = self.theme_color(BG);
        self.pixels.resize(width * height, background);
        self.pixels.fill(background);
        self.hits.clear();
        self.accessible.clear();
        let bottom = height as f32 / self.scale;
        if let Some(t) = intro {
            let t = if model.reduced_motion { 1.5 } else { t };
            self.pixels.fill(BG);
            let fade =
                smooth((t / 0.85).min(1.0)) * (1.0 - smooth((t - 3.5) / (INTRO_SECONDS - 3.5)));
            let size = 400.0 + (smooth((t / 1.9).min(1.0)) * 10.0) as f32;
            self.logo(
                550.0 - size / 2.0,
                bottom / 2.0 - 160.0,
                size,
                fade as f32,
                if model.reduced_motion {
                    None
                } else {
                    Some(t as f32)
                },
            );
            self.text(
                "O C E A N   S I M U L A T I O N   E N G I N E",
                550.0,
                bottom / 2.0 + 142.0,
                10.0,
                blend(BG, MUTED, fade as f32),
                true,
            );
            self.text("Esc to skip", 550.0, bottom - 42.0, 11.0, 0x737373, true);
            return;
        }
        if model.new_project {
            self.new_project_form(model, bottom);
            return;
        }
        if model.workspace && model.session.is_some() {
            self.workspace(model, bottom);
            return;
        }
        self.rect(0.0, 0.0, 210.0, bottom, BG);
        self.rect(209.0, 0.0, 1.0, bottom, 0x262626);
        self.logo(38.0, 30.0, 128.0, 0.95, None);
        self.text("OCEAN ENGINE", 38.0, 126.0, 9.0, MUTED, false);
        self.button(
            0,
            [20.0, 168.0, 170.0, 34.0],
            "Projects",
            !model.about,
            model,
        );
        self.button(
            1,
            [20.0, 212.0, 170.0, 34.0],
            "About Pelagic",
            model.about,
            model,
        );
        self.text("LOCAL / NATIVE", 30.0, bottom - 76.0, 9.0, MUTED, false);
        self.text(
            concat!("Pelagic  ", env!("CARGO_PKG_VERSION")),
            30.0,
            bottom - 51.0,
            11.0,
            0x737373,
            false,
        );
        self.text(
            if model.about { "About" } else { "Projects" },
            254.0,
            39.0,
            13.0,
            TEXT,
            false,
        );
        self.button(
            5,
            [758.0, 25.0, 158.0, 30.0],
            if model.muted {
                "Sound off"
            } else {
                "Startup sound on"
            },
            false,
            model,
        );
        self.button(
            7,
            [928.0, 25.0, 124.0, 30.0],
            if model.dark_mode {
                "Light mode"
            } else {
                "Dark mode"
            },
            false,
            model,
        );
        self.rect(210.0, 75.0, 890.0, 1.0, 0x262626);
        if model.about {
            self.text(
                "A world beneath the surface.",
                254.0,
                158.0,
                29.0,
                TEXT,
                false,
            );
            self.text("Pelagic", 254.0, 221.0, 17.0, ACCENT, false);
            self.text(
                "An independent ocean simulation engine, written in Rust.",
                254.0,
                262.0,
                14.0,
                MUTED,
                false,
            );
            self.text(
                "Native interface. CPU rendering. No browser runtime.",
                254.0,
                295.0,
                14.0,
                MUTED,
                false,
            );
            self.text(
                "Phase 4 / Marine life, vehicles, sensors and experiments.",
                254.0,
                359.0,
                13.0,
                MUTED,
                false,
            );
            self.text(
                "Open a reference lab or create a local project to begin.",
                254.0,
                388.0,
                13.0,
                MUTED,
                false,
            );
            self.button(
                4,
                [254.0, 446.0, 148.0, 34.0],
                "Replay startup",
                false,
                model,
            );
        } else {
            self.project_hub(model, bottom);
        }
        self.rect(210.0, bottom - 44.0, 890.0, 1.0, 0x262626);
        self.text("RUST NATIVE", 254.0, bottom - 27.0, 9.0, ACCENT, false);
        self.text(
            "Local projects / CPU rendering",
            828.0,
            bottom - 27.0,
            10.0,
            MUTED,
            false,
        );
    }
    fn project_hub(&mut self, model: &Model, bottom: f32) {
        self.text("Your ocean workspace", 254.0, 104.0, 25.0, TEXT, false);
        self.text(
            "Create a project, open a study, or try the reference scene.",
            254.0,
            146.0,
            13.0,
            MUTED,
            false,
        );
        let width = self.button(
            2,
            [254.0, 184.0, 180.0, 34.0],
            "+  New project",
            true,
            model,
        );
        self.button(
            3,
            [254.0 + width + 12.0 / self.scale, 184.0, 180.0, 34.0],
            "Open project",
            false,
            model,
        );
        if !model.notice.is_empty() {
            let notice: String = model.notice.chars().take(95).collect();
            self.text(&notice, 254.0, 222.0, 11.0, TEXT, false);
        }
        let panel_top = 244.0;
        let panel_height = (bottom - 314.0).max(260.0);
        let edge = 1.0 / self.scale;
        for rect in [
            [254.0, panel_top, 482.0, panel_height],
            [756.0, panel_top, 296.0, panel_height],
        ] {
            self.rounded_rect(rect, 8.0, 0x303030);
            self.rounded_rect(
                [
                    rect[0] + edge,
                    rect[1] + edge,
                    rect[2] - 2.0 * edge,
                    rect[3] - 2.0 * edge,
                ],
                8.0 - edge,
                BG,
            );
        }
        self.text("Marine robotics lab", 274.0, 264.0, 17.0, TEXT, false);
        self.text(
            "Explore vehicles, schooling fish and mounted sensors.",
            274.0,
            296.0,
            12.0,
            MUTED,
            false,
        );
        if self.sample.is_none() {
            self.sample = Some(
                crate::session::Session::from_scene(&crate::project::Scene::robotics_demo()).frame,
            );
        }
        let preview_height = (panel_height - 185.0).max(80.0);
        let preview = self.sample.take().expect("sample frame");
        self.blit_frame(&preview, [274.0, 330.0, 442.0, preview_height]);
        self.sample = Some(preview);
        let details_y = 330.0 + preview_height + 16.0;
        self.text(
            "30 m seabed    /    Three bodies    /    24 fish",
            274.0,
            details_y,
            11.0,
            MUTED,
            false,
        );
        self.button(
            8,
            [274.0, details_y + 30.0, 180.0, 34.0],
            "Try reference scene",
            false,
            model,
        );
        self.text("Current session", 776.0, 264.0, 17.0, TEXT, false);
        self.rect(776.0, 306.0, 256.0, edge, 0x262626);
        if let Some(name) = &model.project {
            let name: String = name.chars().take(29).collect();
            self.text(&name, 776.0, 330.0, 14.0, TEXT, false);
            self.text("Paused workspace", 776.0, 359.0, 12.0, MUTED, false);
            self.button(6, [776.0, 398.0, 230.0, 34.0], "Continue", false, model);
        } else {
            self.text("Nothing open yet", 776.0, 330.0, 14.0, TEXT, false);
            self.text(
                "Open a project to continue",
                776.0,
                366.0,
                12.0,
                MUTED,
                false,
            );
            self.text(
                "or start with the reference ocean.",
                776.0,
                388.0,
                12.0,
                MUTED,
                false,
            );
        }
        if model.recovery_available {
            self.button(
                27,
                [776.0, 438.0, 240.0, 34.0],
                "Recover unsaved work",
                false,
                model,
            );
        } else if !model.recent.is_empty() {
            self.text("RECENT PROJECTS", 776.0, 424.0, 9.0, MUTED, false);
            for (i, path) in model.recent.iter().take(4).enumerate() {
                let y = 448.0 + i as f32 * 42.0;
                if y + 32.0 > panel_top + panel_height - 12.0 {
                    break;
                }
                let name = std::path::Path::new(path)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy();
                let label: String = name.chars().take(24).collect();
                self.button(60 + i, [776.0, y, 240.0, 34.0], &label, false, model);
            }
        }
        if model.recent.is_empty() && !model.recovery_available {
            self.text(
                "Projects stay on your computer.",
                776.0,
                panel_top + panel_height - 25.0,
                10.0,
                MUTED,
                false,
            );
        }
    }
    fn new_project_form(&mut self, model: &Model, bottom: f32) {
        self.text("Create a project", 180.0, 52.0, 26.0, TEXT, false);
        self.text(
            "Choose a starting scene and where to save your work.",
            180.0,
            98.0,
            13.0,
            MUTED,
            false,
        );
        self.field(
            20,
            [180.0, 161.0, 600.0, 36.0],
            "Project name",
            &model.new_name,
            model,
        );
        self.field(
            21,
            [180.0, 245.0, 600.0, 36.0],
            "Location (parent folder)",
            &model.new_folder,
            model,
        );
        self.button(22, [796.0, 245.0, 140.0, 36.0], "Browse...", false, model);
        self.text("Starting scene", 180.0, 321.0, 13.0, TEXT, false);
        self.button(
            25,
            [180.0, 353.0, 190.0, 36.0],
            "Empty ocean",
            !model.new_vehicle,
            model,
        );
        self.button(
            26,
            [386.0, 353.0, 220.0, 36.0],
            "Ocean + vehicle",
            model.new_vehicle,
            model,
        );
        self.text(
            if model.new_vehicle {
                "Coastal terrain, waves, harbour and reference vehicle."
            } else {
                "Flat seabed and current. Add a vehicle from the inspector."
            },
            180.0,
            406.0,
            12.0,
            MUTED,
            false,
        );
        self.text(
            "The project gets its own folder. Scene and camera settings are saved.",
            180.0,
            441.0,
            12.0,
            MUTED,
            false,
        );
        let notice: String = model.notice.chars().take(95).collect();
        self.text(&notice, 180.0, 484.0, 12.0, TEXT, false);
        self.button(
            23,
            [180.0, bottom - 58.0, 200.0, 36.0],
            "Create project",
            true,
            model,
        );
        self.button(
            24,
            [390.0, bottom - 58.0, 150.0, 36.0],
            "Cancel",
            false,
            model,
        );
    }
    fn field(&mut self, id: usize, rect: [f32; 4], label: &str, value: &str, model: &Model) {
        self.accessible.push(AccessibleControl {
            id,
            rect,
            label: label.into(),
            value: Some(value.into()),
            disabled: false,
        });
        let [x, y, w, h] = rect;
        self.text(label, x, y - 23.0, 11.0, MUTED, false);
        let focused = model.editing.as_ref().is_some_and(|edit| edit.0 == id);
        let edge = 1.0 / self.scale;
        self.rounded_rect(rect, 5.0, if focused { 0x525252 } else { 0x303030 });
        self.rounded_rect(
            [x + edge, y + edge, w - 2.0 * edge, h - 2.0 * edge],
            5.0 - edge,
            BG,
        );
        let content = if let Some((field, buffer)) = &model.editing {
            if *field == id { buffer.as_str() } else { value }
        } else {
            value
        };
        let shown = self.fit_text(content, w - 26.0, 13.0, true);
        if focused && model.input_selected {
            self.rounded_rect([x + 8.0, y + 5.0, w - 16.0, h - 10.0], 2.0, 0x202020);
        }
        self.text(&shown, x + 10.0, y + 9.0, 13.0, TEXT, false);
        if focused && !model.input_selected {
            let offset = content
                .len()
                .saturating_sub(shown.trim_start_matches('…').len());
            let cursor = model.input_cursor.min(content.len());
            let prefix = content.get(offset..cursor).unwrap_or("");
            let font = self.font.as_scaled(PxScale::from(13.0 * self.scale * 1.16));
            let advance = prefix
                .chars()
                .map(|c| font.h_advance(self.font.glyph_id(c)))
                .sum::<f32>()
                / self.scale;
            self.rect(
                (x + 10.0 + advance).min(x + w - 8.0),
                y + 8.0,
                edge,
                h - 16.0,
                TEXT,
            );
        }
        self.hits.push(Hit { id, rect });
    }
    fn blit_frame(&mut self, frame: &pelagic::render::Frame, rect: [f32; 4]) {
        let [left, top, w, h] = rect;
        let width = w * self.scale;
        let height = h * self.scale;
        let fit = (width / frame.width as f32).max(height / frame.height as f32);
        let crop_x = (frame.width as f32 - width / fit) * 0.5;
        let crop_y = (frame.height as f32 - height / fit) * 0.5;
        for y in 0..(h * self.scale) as usize {
            for x in 0..(w * self.scale) as usize {
                let sx = (crop_x + x as f32 / fit) as usize;
                let sy = (crop_y + y as f32 / fit) as usize;
                let p = (sy.min(frame.height - 1) * frame.width + sx.min(frame.width - 1)) * 3;
                let px = (left * self.scale) as usize + x;
                let py = (top * self.scale) as usize + y;
                if px < self.width && py < self.height {
                    self.pixels[py * self.width + px] = ((frame.rgb[p] as u32) << 16)
                        | ((frame.rgb[p + 1] as u32) << 8)
                        | frame.rgb[p + 2] as u32;
                }
            }
        }
    }
    fn section(&mut self, model: &Model, area: [f32; 4]) {
        let session = model.session.as_ref().expect("section session");
        let scene = model
            .document
            .as_ref()
            .map(|d| d.scene.clone())
            .unwrap_or_default();
        let [x, y, w, h] = area;
        let top = y + 25.0;
        let height = h - 55.0;
        let left = x + 42.0;
        let width = w - 52.0;
        let depth = (scene.bottom_depth * 1.8 + 8.0).max(20.0);
        let bounds = session
            .world
            .ocean
            .bottom
            .grid()
            .map_or([-256.0, -256.0, 256.0, 256.0], |g| g.bounds());
        let project = |z: f64| top + (4.0 - z as f32) / depth as f32 * height;
        self.rect(x, y, w, h, BG);
        for row in 0..height as usize {
            let z = 4.0 - row as f64 / height as f64 * depth;
            let t = session
                .world
                .ocean
                .profile
                .sample(-z, scene.density)
                .temperature;
            let color = if z > 0.0 {
                [180.0, 208.0, 220.0]
            } else {
                [8.0 + t * 0.7, 45.0 + t * 2.0, 81.0 + t * 2.0]
            };
            let color = ((color[0] as u32) << 16) | ((color[1] as u32) << 8) | color[2] as u32;
            // Contiguous fills must not use rounded-rectangle edge coverage.
            // Antialiasing every one-unit strip produces visible scanlines.
            self.rect(left, top + row as f32, width, 1.0, color);
        }
        for i in 0..width as usize {
            let world_x = bounds[0] + i as f64 / width as f64 * (bounds[2] - bounds[0]);
            let bed = session.world.ocean.bottom.elevation(world_x, 0.0);
            let sy = project(bed).clamp(top, top + height);
            self.rect(left + i as f32, sy, 1.0, top + height - sy, 0x515655);
            let wave = session
                .world
                .ocean
                .surface(world_x, 0.0, session.world.time());
            let wy = project(wave.height);
            self.rect(left + i as f32, wy, 1.0, 1.5, 0xbbe9e0);
        }
        for i in 0..5 {
            let z = depth * i as f64 / 4.0;
            let yy = project(-z).min(top + height);
            self.text(&format!("{z:.0}"), x, yy - 5.0, 10.0, MUTED, false);
            self.rect(left, yy, width, 0.5, 0x303030);
        }
        let state = session
            .display
            .clone()
            .unwrap_or_else(|| session.world.snapshot());
        for body in &state.vehicles {
            let xx =
                left + ((body.position.x - bounds[0]) / (bounds[2] - bounds[0])) as f32 * width;
            let yy = project(body.position.z);
            if xx < left || xx > left + width || yy < top || yy > top + height {
                continue;
            }
            self.rect(xx - 3.0, yy - 3.0, 6.0, 6.0, 0xf3cf7a);
            let label = scene
                .entities
                .iter()
                .find(|e| e.id == body.id)
                .map_or("Reference vehicle", |e| e.name.as_str());
            self.text(
                &self.fit_text(label, (left + width - xx - 10.0).max(10.0), 10.0, false),
                xx + 7.0,
                yy - 12.0,
                10.0,
                0xffffff,
                false,
            );
        }
        self.text("Depth (m)", x, y, 10.0, MUTED, false);
        self.text(
            "East–depth projection · bodies from all northings",
            left + 85.0,
            y,
            10.0,
            MUTED,
            false,
        );
        self.text(
            &format!("{:.0} m", bounds[0]),
            left,
            top + height + 12.0,
            10.0,
            MUTED,
            false,
        );
        self.text(
            &format!("{:.0}", (bounds[0] + bounds[2]) * 0.5),
            left + width * 0.5,
            top + height + 12.0,
            10.0,
            MUTED,
            false,
        );
        self.text(
            &format!("{:.0} m", bounds[2]),
            left + width - 38.0,
            top + height + 12.0,
            10.0,
            MUTED,
            false,
        );
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: u32) {
        let color = self.theme_color(color);
        let x0 = (x * self.scale).max(0.0) as usize;
        let y0 = (y * self.scale).max(0.0) as usize;
        let x1 = ((x + w) * self.scale).max(0.0) as usize;
        let y1 = ((y + h) * self.scale).max(0.0) as usize;
        for yy in y0.min(self.height)..y1.min(self.height) {
            for xx in x0.min(self.width)..x1.min(self.width) {
                self.pixels[yy * self.width + xx] = color;
            }
        }
    }
    fn button(
        &mut self,
        id: usize,
        rect: [f32; 4],
        label: &str,
        active: bool,
        model: &Model,
    ) -> f32 {
        let [x, y, max_width, _] = rect;
        // Window width is not a font/DPI setting. Keep controls compact even
        // when the app is maximised, rather than stretching them with the scene.
        let control_scale = self.scale.min(1.25 * self.dpi_scale);
        let fitted_label = self.fit_text(
            label,
            (max_width - 28.0 * control_scale / self.scale).max(1.0),
            13.0 * control_scale / self.scale,
            false,
        );
        let label = fitted_label.as_str();
        let label_scale = self
            .font
            .as_scaled(PxScale::from(13.0 * 1.16 * control_scale));
        let text_width = label
            .chars()
            .map(|ch| label_scale.h_advance(self.font.glyph_id(ch)))
            .sum::<f32>();
        let navigation = matches!(id, 0 | 1 | 14 | 15);
        let w = if navigation {
            max_width
        } else {
            ((text_width + 28.0 * control_scale) / self.scale).min(max_width)
        };
        let h = 32.0 * control_scale / self.scale;
        let rect = [x, y, w, h];
        let focus = model.focus == Some(id);
        let disabled = match id {
            17 => !model.document.as_ref().is_some_and(|d| d.can_undo()),
            18 => !model.document.as_ref().is_some_and(|d| d.can_redo()),
            _ => false,
        };
        self.accessible.push(AccessibleControl {
            id,
            rect,
            label: label.into(),
            value: None,
            disabled,
        });
        let primary = matches!(id, 2 | 11 | 23);
        let hover = if disabled { 0.0 } else { self.hover_amount[id] };
        let press = self.press_amount[id];
        let normal = if primary {
            TEXT
        } else if active {
            0x171717
        } else {
            BG
        };
        let over = if primary { 0xe5e5e5 } else { 0x202020 };
        let down = if primary { 0xcfcfcf } else { 0x303030 };
        let fill = blend(
            blend(self.theme_color(normal), self.theme_color(over), hover),
            self.theme_color(down),
            press,
        );
        let radius = 6.0 * control_scale / self.scale;
        if !primary && !navigation {
            // A single physical pixel: never thicken the outline on maximise.
            let edge = 1.0 / self.scale;
            let border = blend(
                if self.dark { 0x484848 } else { 0xc4c4c4 },
                if self.dark { 0x777777 } else { 0x8c8c8c },
                hover.max(if focus { 1.0 } else { 0.0 }),
            );
            self.rounded_rect_raw(
                [x, y + edge, w, h],
                radius,
                blend(self.theme_color(BG), self.theme_color(TEXT), 0.045),
            );
            self.rounded_rect_raw([x, y, w, h], radius, border);
            self.rounded_rect_raw(
                [x + edge, y + edge, w - 2.0 * edge, h - 2.0 * edge],
                (radius - edge).max(0.0),
                fill,
            );
        } else {
            self.rounded_rect_raw([x, y, w, h], radius, fill);
        }
        let padding = 14.0 * control_scale / self.scale;
        let size = 13.0 * control_scale / self.scale;
        self.text(
            label,
            x + padding,
            y + (h - size * 1.16) / 2.0 + press * control_scale / self.scale,
            size,
            if disabled {
                MUTED
            } else if primary {
                BG
            } else {
                TEXT
            },
            false,
        );
        if !disabled {
            self.hits.push(Hit { id, rect });
        }
        w
    }
    fn rounded_rect(&mut self, rect: [f32; 4], radius: f32, color: u32) {
        let color = self.theme_color(color);
        self.rounded_rect_raw(rect, radius, color);
    }
    fn filtered_frame(
        &mut self,
        frame: &pelagic::render::Frame,
        left: usize,
        top: usize,
        width: usize,
        height: usize,
        scale: f32,
    ) {
        // Pack once and precompute the horizontal mapping. Integer bilinear
        // interpolation avoids repeated float/channel work for every UI pixel.
        let source: Vec<u32> = frame
            .rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)
            .collect();
        let width = width.min(self.width.saturating_sub(left));
        let height = height.min(self.height.saturating_sub(top));
        let columns: Vec<_> = (0..width)
            .map(|x| {
                let f = ((x as f32 + 0.5) / scale - 0.5).clamp(0.0, (frame.width - 1) as f32);
                let a = f as usize;
                (
                    a,
                    (a + 1).min(frame.width - 1),
                    ((f - a as f32) * 256.0) as u32,
                )
            })
            .collect();
        for y in 0..height {
            let f = ((y as f32 + 0.5) / scale - 0.5).clamp(0.0, (frame.height - 1) as f32);
            let a = f as usize;
            let b = (a + 1).min(frame.height - 1);
            let t = ((f - a as f32) * 256.0) as u32;
            let row = &mut self.pixels
                [(top + y) * self.width + left..(top + y) * self.width + left + width];
            for (pixel, &(x0, x1, s)) in row.iter_mut().zip(&columns) {
                *pixel = packed_mix(
                    packed_mix(
                        source[a * frame.width + x0],
                        source[a * frame.width + x1],
                        s,
                    ),
                    packed_mix(
                        source[b * frame.width + x0],
                        source[b * frame.width + x1],
                        s,
                    ),
                    t,
                );
            }
        }
    }
    fn rounded_rect_raw(&mut self, rect: [f32; 4], radius: f32, color: u32) {
        let [x, y, w, h] = rect.map(|v| v * self.scale);
        let r = radius * self.scale;
        for py in (y.max(0.0) as usize)..((y + h).max(0.0) as usize).min(self.height) {
            for px in (x.max(0.0) as usize)..((x + w).max(0.0) as usize).min(self.width) {
                let dx = (px as f32 + 0.5 - (x + w / 2.0)).abs() - (w / 2.0 - r);
                let dy = (py as f32 + 0.5 - (y + h / 2.0)).abs() - (h / 2.0 - r);
                let distance = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
                let coverage = (0.5 - distance).clamp(0.0, 1.0);
                let i = py * self.width + px;
                self.pixels[i] = blend(self.pixels[i], color, coverage);
            }
        }
    }
    fn fit_text(&self, text: &str, width: f32, size: f32, tail: bool) -> String {
        let font = self.font.as_scaled(PxScale::from(size * self.scale * 1.16));
        let advance = |c: char| font.h_advance(self.font.glyph_id(c)) / self.scale;
        if text.chars().map(advance).sum::<f32>() <= width {
            return text.into();
        }
        let mut remaining = (width - advance('…')).max(0.0);
        let mut out = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let iterator: Box<dyn Iterator<Item = char>> = if tail {
            Box::new(chars.into_iter().rev())
        } else {
            Box::new(chars.into_iter())
        };
        for c in iterator {
            let a = advance(c);
            if a > remaining {
                break;
            }
            remaining -= a;
            out.push(c);
        }
        if tail {
            out.reverse();
            format!("…{}", out.into_iter().collect::<String>())
        } else {
            format!("{}…", out.into_iter().collect::<String>())
        }
    }
    fn text(&mut self, text: &str, x: f32, y: f32, size: f32, color: u32, center: bool) {
        let color = self.theme_color(color);
        let font = self.font.clone();
        let scaled = font.as_scaled(PxScale::from(size * self.scale * 1.16));
        let width = text
            .chars()
            .map(|c| scaled.h_advance(font.glyph_id(c)))
            .sum::<f32>();
        let mut cursor = x * self.scale - if center { width / 2.0 } else { 0.0 };
        let baseline = y * self.scale + scaled.ascent();
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            let glyph = id.with_scale_and_position(scaled.scale(), point(cursor, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, alpha| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if px >= 0
                        && py >= 0
                        && (px as usize) < self.width
                        && (py as usize) < self.height
                    {
                        let i = py as usize * self.width + px as usize;
                        self.pixels[i] = blend(self.pixels[i], color, alpha);
                    }
                });
            }
            cursor += scaled.h_advance(id);
        }
    }
    /// Premultiplied BGRA icon pixels. Use the emblem without the wordmark so it
    /// stays recognisable at caption sizes. Supersampling preserves the fine arc.
    pub fn icon_pixels(&self, side: usize) -> Vec<u32> {
        let mut pixels = vec![0; side * side];
        let crop_x = self.logo_w as f32 * 0.10;
        let crop_w = self.logo_w as f32 * 0.80;
        let crop_h = self.logo_h as f32 * 0.77;
        // Caption icons need a safe inset at their actual 16 px display size.
        // Keep the emblem clear of the upper frame rather than filling the slot.
        let inset_scale = if side <= 32 { 0.76 } else { 0.9 };
        let scale = side as f32 * inset_scale / crop_w.max(crop_h);
        let left = (side as f32 - crop_w * scale) * 0.5;
        let top = (side as f32 - crop_h * scale) * 0.5 + if side <= 32 { 1.0 } else { 0.0 };
        for y in 0..side {
            for x in 0..side {
                let mut coverage = 0_u32;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let u = (x as f32 + (sx as f32 + 0.5) / 4.0 - left) / scale;
                        let v = (y as f32 + (sy as f32 + 0.5) / 4.0 - top) / scale;
                        if u >= 0.0 && u < crop_w && v >= 0.0 && v < crop_h {
                            coverage +=
                                self.logo[v as usize * self.logo_w + (crop_x + u) as usize] as u32;
                        }
                    }
                }
                let a = coverage / 16;
                pixels[y * side + x] = (a << 24) | (a << 16) | (a << 8) | a;
            }
        }
        pixels
    }

    fn logo(&mut self, x: f32, y: f32, width: f32, opacity: f32, sweep: Option<f32>) {
        let w = (width * self.scale) as usize;
        let h = (w as f32 * self.logo_h as f32 / self.logo_w as f32) as usize;
        for yy in 0..h {
            for xx in 0..w {
                let sx = xx as f32 / w as f32 * (self.logo_w - 1) as f32;
                let sy = yy as f32 / h as f32 * (self.logo_h - 1) as f32;
                let ix = sx as usize;
                let iy = sy as usize;
                let fx = sx.fract();
                let fy = sy.fract();
                let at = |a: usize, b: usize| {
                    self.logo[b.min(self.logo_h - 1) * self.logo_w + a.min(self.logo_w - 1)] as f32
                        / 255.0
                };
                let alpha = (at(ix, iy) * (1.0 - fx) * (1.0 - fy)
                    + at(ix + 1, iy) * fx * (1.0 - fy)
                    + at(ix, iy + 1) * (1.0 - fx) * fy
                    + at(ix + 1, iy + 1) * fx * fy)
                    * opacity;
                let shine = sweep.map_or(0.0, |t| {
                    let d = xx as f32 / w as f32 - (t - 0.5) * 0.8;
                    (-d * d / 0.006).exp() * 0.22
                });
                let px = (x * self.scale) as usize + xx;
                let py = (y * self.scale) as usize + yy;
                if px < self.width && py < self.height {
                    let i = py * self.width + px;
                    let ink = if self.dark {
                        blend(0xe5e5e5, 0xffffff, shine)
                    } else {
                        0x171717
                    };
                    self.pixels[i] = blend(self.pixels[i], ink, alpha);
                }
            }
        }
    }
    /// Returns true only while a button is moving toward its target appearance.
    pub fn animate_buttons(&mut self, model: &Model, seconds: f32) -> bool {
        if model.reduced_motion {
            for i in 0..self.hover_amount.len() {
                self.hover_amount[i] = if model.hover == Some(i) || model.focus == Some(i) {
                    1.0
                } else {
                    0.0
                };
                self.press_amount[i] = if model.pressed == Some(i) && model.hover == Some(i) {
                    1.0
                } else {
                    0.0
                };
            }
            return false;
        }
        let mut moving = false;
        let hover_speed = 1.0 - (-seconds.clamp(0.0, 0.1) / 0.055).exp();
        let press_speed = 1.0 - (-seconds.clamp(0.0, 0.1) / 0.025).exp();
        for i in 0..self.hover_amount.len() {
            let target = if model.hover == Some(i) || model.focus == Some(i) {
                1.0
            } else {
                0.0
            };
            let pressed = if model.pressed == Some(i) && model.hover == Some(i) {
                1.0
            } else {
                0.0
            };
            for (value, to, speed) in [
                (&mut self.hover_amount[i], target, hover_speed),
                (&mut self.press_amount[i], pressed, press_speed),
            ] {
                *value += (to - *value) * speed;
                if (to - *value).abs() < 0.002 {
                    *value = to;
                } else {
                    moving = true;
                }
            }
        }
        moving
    }
    fn theme_color(&self, color: u32) -> u32 {
        if self.dark {
            return color;
        }
        match color {
            BG => 0xffffff,
            TEXT => 0x171717,
            MUTED => 0x737373,
            0x262626 | 0x303030 => 0xe5e5e5,
            0x171717 => 0xfafafa,
            0x202020 => 0xf5f5f5,
            0x525252 => 0xa3a3a3,
            0x101012 => 0xe9e9ed,
            0x19191c => 0xf0f0f3,
            0x202024 => 0xf8f8fa,
            0x29292d => 0xe6e6eb,
            0x343438 => 0xcdcdd4,
            0x38383e => 0xc4c4cc,
            0x213b54 => 0xcde4fa,
            0x73b5f4 => 0x1667aa,
            0xcfcfcf => 0x404040,
            0xe5e5e5 => 0x262626,
            _ => color,
        }
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let x = x / self.scale;
        let y = y / self.scale;
        self.hits
            .iter()
            .find(|h| {
                x >= h.rect[0]
                    && y >= h.rect[1]
                    && x < h.rect[0] + h.rect[2]
                    && y < h.rect[1] + h.rect[3]
            })
            .map(|h| h.id)
            .or_else(|| self.gizmo.as_ref()?.hit([x, y]).map(|h| h.id()))
    }
    pub fn interactive_id(&self, id: usize) -> bool {
        self.hits.iter().any(|h| h.id == id)
            || self
                .accessible
                .iter()
                .any(|c| c.id == id && (290..=294).contains(&id))
            || self.viewport_actors.iter().any(|a| a.0 == id)
    }
    pub fn focus_ids(&self) -> Vec<usize> {
        self.accessible
            .iter()
            .filter(|c| !c.disabled)
            .map(|c| c.id)
            .collect()
    }
    pub fn viewport_contains(&self, x: f32, y: f32) -> bool {
        self.area_contains(self.viewport_area, x, y)
    }
    pub fn outliner_contains(&self, x: f32, y: f32) -> bool {
        self.area_contains(self.outliner_area, x, y)
    }
    fn area_contains(&self, area: Option<[f32; 4]>, x: f32, y: f32) -> bool {
        area.is_some_and(|[left, top, width, height]| {
            let (x, y) = (x / self.scale, y / self.scale);
            x >= left && x < left + width && y >= top && y < top + height
        })
    }
    pub fn ui_scale(&self) -> f32 {
        self.scale
    }
    pub fn splitter_at(&self, x: f32, y: f32) -> Option<crate::layout::Splitter> {
        self.editor_layout?
            .splitters()
            .into_iter()
            .find(|(_, area)| self.area_contains(Some(*area), x, y))
            .map(|(splitter, _)| splitter)
    }
    pub fn save_png(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let writer = BufWriter::new(File::create(path)?);
        let mut encoder = png::Encoder::new(writer, self.width as u32, self.height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        let bytes: Vec<u8> = self
            .pixels
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect();
        writer.write_image_data(&bytes)?;
        Ok(())
    }
}
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |s: u32| {
        let av = ((a >> s) & 255) as f32;
        let bv = ((b >> s) & 255) as f32;
        (av + (bv - av) * t).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}
pub fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Painter {
    fn segment(&mut self, a: [f32; 2], b: [f32; 2], color: u32) {
        let steps = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()).ceil() as usize).clamp(1, 2048);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            self.rect(
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                1.0 / self.scale,
                1.0 / self.scale,
                color,
            );
        }
    }
    fn results(&mut self, model: &Model, area: [f32; 4]) {
        let [x, y, w, h] = area;
        self.rect(x, y, w, h, BG);
        self.text(
            "RECORDED TRUTH / DEPTH AND SPEED",
            x + 20.0,
            y + 22.0,
            11.0,
            TEXT,
            false,
        );
        let Some(run) = model.run.as_ref().filter(|r| !r.frames.is_empty()) else {
            self.text(
                "Record a live run or run a batch experiment to see results.",
                x + 20.0,
                y + 75.0,
                12.0,
                MUTED,
                false,
            );
            return;
        };
        let start = run.frames[0].truth.time;
        let end = run.frames.last().unwrap().truth.time.max(start + 0.001);
        for (panel, label, speed, color) in [
            (0, "Depth (m)", false, 0x89b8c2),
            (1, "Speed (m/s)", true, 0xd4ba81),
        ] {
            let top = y + 50.0 + panel as f32 * (h - 90.0) * 0.5;
            let height = (h - 130.0) * 0.5;
            self.text(label, x + 20.0, top, 11.0, MUTED, false);
            let value = |f: &pelagic::experiment::RunFrame| {
                if speed {
                    f.velocities.first().map_or(0.0, |v| v.length())
                } else {
                    f.truth.vehicles.first().map_or(0.0, |p| -p.position.z)
                }
            };
            let min = run.frames.iter().map(&value).fold(f64::INFINITY, f64::min);
            let max = run
                .frames
                .iter()
                .map(&value)
                .fold(f64::NEG_INFINITY, f64::max)
                .max(min + 0.1);
            self.text(
                &format!("{min:.2} – {max:.2}"),
                x + w - 135.0,
                top,
                10.0,
                MUTED,
                false,
            );
            for i in 1..4 {
                self.rect(
                    x + 20.0,
                    top + 20.0 + height * i as f32 / 4.0,
                    w - 40.0,
                    1.0 / self.scale,
                    0x262626,
                );
            }
            let mut previous = None;
            for f in run.frames.iter().step_by((run.frames.len() / 500).max(1)) {
                let p = [
                    x + 20.0 + (w - 40.0) * ((f.truth.time - start) / (end - start)) as f32,
                    top + 20.0 + height * (1.0 - ((value(f) - min) / (max - min)) as f32),
                ];
                if let Some(old) = previous {
                    self.segment(old, p, color);
                }
                previous = Some(p);
            }
            if let Some(index) = model.playback.and_then(|i| run.frames.get(i)) {
                let px =
                    x + 20.0 + (w - 40.0) * ((index.truth.time - start) / (end - start)) as f32;
                self.rect(px, top + 20.0, 1.0 / self.scale, height, 0xffffff);
            }
        }
        self.text(
            &format!("{start:.2} s"),
            x + 20.0,
            y + h - 20.0,
            11.0,
            MUTED,
            false,
        );
        self.text(
            &format!("{end:.2} s / {} samples", run.frames.len()),
            x + w - 175.0,
            y + h - 20.0,
            11.0,
            MUTED,
            false,
        );
    }
    fn sonar(&mut self, model: &Model, area: [f32; 4]) {
        let [x, y, w, h] = area;
        self.rect(x, y, w, h, BG);
        self.text(
            "SONAR / STRAIGHT-RAY RANGE RETURNS",
            x + 20.0,
            y + 22.0,
            11.0,
            TEXT,
            false,
        );
        let observation = model
            .playback
            .and_then(|i| model.run.as_ref()?.frames.get(i)?.observations.first())
            .or_else(|| model.session.as_ref()?.observations.first());
        let Some(o) = observation else {
            self.text(
                "Play or step the simulation to sample the mounted sonar.",
                x + 20.0,
                y + 70.0,
                12.0,
                MUTED,
                false,
            );
            return;
        };
        let max = model
            .document
            .as_ref()
            .map_or(80.0, |d| d.scene.sensors.range);
        let origin = [x + w * 0.5, y + 65.0];
        let radius = (h - 110.0).min(w * 0.46);
        for fraction in [0.25, 0.5, 0.75, 1.0] {
            let mut old = None;
            for i in 0..61 {
                let angle = (-75.0 + i as f64 * 2.5).to_radians();
                let p = [
                    origin[0] + angle.sin() as f32 * radius * fraction,
                    origin[1] + angle.cos() as f32 * radius * fraction,
                ];
                if let Some(a) = old {
                    self.segment(a, p, 0x262626);
                }
                old = Some(p);
            }
            self.text(
                &format!("{:.0} m", max * fraction as f64),
                origin[0] + 4.0,
                origin[1] + radius * fraction,
                10.0,
                MUTED,
                false,
            );
        }
        for b in &o.beams {
            if let Some(range) = b.range {
                let p = [
                    origin[0] + b.angle.sin() as f32 * radius * (range / max) as f32,
                    origin[1] + b.angle.cos() as f32 * radius * (range / max) as f32,
                ];
                self.segment(origin, p, 0x23383d);
                self.rect(p[0] - 2.0, p[1] - 2.0, 4.0, 4.0, 0x9dd9cf);
            }
        }
        self.text(
            &format!(
                "Sample {:.2} s / delivered {:.2} s / depth {:.2} m",
                o.time, o.available_time, o.depth
            ),
            x + 20.0,
            y + h - 20.0,
            11.0,
            MUTED,
            false,
        );
    }
    fn atlas(&mut self, model: &Model, area: [f32; 4]) {
        let [x, y, w, h] = area;
        self.rect(x, y, w, h, BG);
        self.text(
            "NOAA ETOPO1 / 2° OVERVIEW",
            x + 15.0,
            y + 20.0,
            11.0,
            TEXT,
            false,
        );
        let top = y + 45.0;
        let map_height = (h - 85.0).min(w * 0.5);
        let map_width = map_height * 2.0;
        let left = x + (w - map_width) * 0.5;
        for line in include_str!("../../../examples/data/world-etopo1.csv")
            .lines()
            .skip(2)
        {
            let mut values = line.split(',').filter_map(|s| s.parse::<f64>().ok());
            if let (Some(lat), Some(lon), Some(z)) = (values.next(), values.next(), values.next()) {
                let color = if z < 0.0 {
                    let level = (1.0 - (-z / 8000.0).clamp(0.0, 1.0)) * 50.0;
                    ((8u32) << 16) | (((34.0 + level) as u32) << 8) | (60.0 + level) as u32
                } else {
                    let level = (z / 5000.0).clamp(0.0, 1.0) * 80.0;
                    (((90.0 + level) as u32) << 16)
                        | (((104.0 + level) as u32) << 8)
                        | (75.0 + level) as u32
                };
                self.rect(
                    left + (lon as f32 + 180.0) / 360.0 * map_width,
                    top + (90.0 - lat as f32) / 180.0 * map_height,
                    map_width / 180.0 + 1.0 / self.scale,
                    map_height / 90.0 + 1.0 / self.scale,
                    color,
                );
            }
        }
        if let Some(doc) = &model.document {
            let p = &doc.scene.provenance;
            let px = left + (p.origin_longitude as f32 + 180.0) / 360.0 * map_width;
            let py = top + (90.0 - p.origin_latitude as f32) / 180.0 * map_height;
            self.rect(px - 5.0, py, 11.0, 1.0, 0xffffff);
            self.rect(px, py - 5.0, 1.0, 11.0, 0xffffff);
        }
        self.text(
            "Overview is coarse data. Local simulation uses a declared regional grid.",
            x + 15.0,
            y + h - 20.0,
            11.0,
            MUTED,
            false,
        );
    }
}
/// Bilinear reconstruction keeps quality-tier upscaling from producing block edges.
fn packed_mix(a: u32, b: u32, t: u32) -> u32 {
    ((((a & 0xff00ff) * (256 - t) + (b & 0xff00ff) * t) >> 8) & 0xff00ff)
        | ((((a & 0xff00) * (256 - t) + (b & 0xff00) * t) >> 8) & 0xff00)
}
#[cfg(test)]
fn filtered_pixel(frame: &pelagic::render::Frame, x: f32, y: f32) -> u32 {
    let x = x.clamp(0.0, (frame.width - 1) as f32);
    let y = y.clamp(0.0, (frame.height - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = (
        (x0 + 1).min(frame.width - 1),
        (y0 + 1).min(frame.height - 1),
    );
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let mut color = 0;
    for c in 0..3 {
        let at = |px, py| frame.rgb[(py * frame.width + px) * 3 + c] as f32;
        let a = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
        let b = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
        color = (color << 8) | ((a * (1.0 - fy) + b * fy).round() as u32);
    }
    color
}
#[cfg(test)]
mod interaction_tests {
    use super::*;
    #[test]
    fn resized_panels_keep_controls_in_bounds_without_overlapping_dividers() {
        let scene = crate::project::Scene::robotics_demo();
        let mut model = Model {
            workspace: true,
            document: Some(crate::project::Document::new(scene.clone())),
            session: Some(crate::session::Session::from_scene(&scene)),
            ..Default::default()
        };
        let mut painter = Painter::new();
        for (width, height) in [(900, 620), (1100, 720), (1920, 990)] {
            for requested in [100.0, 2000.0] {
                model.layout = crate::layout::LayoutSettings {
                    sidebar_width: Some(requested),
                    outliner_height: Some(requested),
                    content_height: Some(requested),
                };
                for page in 0..6 {
                    model.inspector_page = page;
                    painter.paint(width, height, &model, None);
                    for hit in &painter.hits {
                        assert!(
                            hit.rect[0] >= 0.0
                                && hit.rect[1] >= 0.0
                                && hit.rect[0] + hit.rect[2] <= width as f32
                                && hit.rect[1] + hit.rect[3] <= height as f32,
                            "control {} is outside resized layout",
                            hit.id
                        );
                        for (splitter, area) in painter.editor_layout.unwrap().splitters() {
                            if hit.id == splitter.id() {
                                continue;
                            }
                            let overlap = hit.rect[0] < area[0] + area[2]
                                && hit.rect[0] + hit.rect[2] > area[0]
                                && hit.rect[1] < area[1] + area[3]
                                && hit.rect[1] + hit.rect[3] > area[1];
                            assert!(!overlap, "control {} overlaps {:?}", hit.id, splitter);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn editor_density_and_camera_hit_area_follow_dpi_and_view_mode() {
        let scene = crate::project::Scene::robotics_demo();
        let mut model = Model {
            workspace: true,
            dark_mode: true,
            document: Some(crate::project::Document::new(scene.clone())),
            session: Some(crate::session::Session::from_scene(&scene)),
            ..Default::default()
        };
        let mut painter = Painter::new();
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            painter.dpi_scale = dpi;
            painter.paint(
                (1100.0 * dpi) as usize,
                (720.0 * dpi) as usize,
                &model,
                None,
            );
            let play = painter.hits.iter().find(|h| h.id == 11).unwrap();
            assert_eq!(play.rect[3], 26.0);
            assert!((painter.ui_scale() - dpi).abs() < 0.001);
            let area = painter.viewport_area.unwrap();
            assert!(painter.viewport_contains((area[0] + 10.0) * dpi, (area[1] + 10.0) * dpi));
            assert!(!painter.viewport_contains(1090.0 * dpi, 150.0 * dpi));
            model.viewport_mode = 2;
            painter.paint(
                (1100.0 * dpi) as usize,
                (720.0 * dpi) as usize,
                &model,
                None,
            );
            assert!(!painter.viewport_contains((area[0] + 10.0) * dpi, (area[1] + 10.0) * dpi));
            assert!(!painter.hits.iter().any(|h| h.id == 57));
            model.viewport_mode = 0;
        }
    }
    #[test]
    fn section_fill_is_opaque_without_scanlines_at_fractional_dpi() {
        let scene = crate::project::Scene::default();
        let model = Model {
            document: Some(crate::project::Document::new(scene.clone())),
            session: Some(crate::session::Session::from_scene(&scene)),
            ..Default::default()
        };
        let mut painter = Painter::new();
        for scale in [1.0, 1.25, 1.5, 1.745] {
            painter.width = 1000;
            painter.height = 900;
            painter.scale = scale;
            painter.dark = true;
            painter.pixels = vec![0; painter.width * painter.height];
            painter.section(&model, [0.0, 0.0, 500.0, 400.0]);
            let px = (100.0 * scale) as usize;
            for row in (310.0 * scale) as usize..(330.0 * scale) as usize {
                assert_eq!(
                    painter.pixels[row * painter.width + px],
                    0x515655,
                    "seabed strip at scale {scale}, row {row}"
                );
            }
            for row in (145.0 * scale) as usize..(165.0 * scale) as usize {
                assert!(
                    painter.pixels[row * painter.width + px] & 255 > 80,
                    "water strip at scale {scale}, row {row}"
                );
            }
        }
    }
    #[test]
    fn actor_search_and_asset_paging_preserve_truth_and_reach_the_full_inventory() {
        let mut scene = crate::project::Scene::robotics_demo();
        scene.entities = (1..=15)
            .map(|id| {
                pelagic::entities::Entity::rov(
                    id,
                    pelagic::math::Vec3::new(id as f64 * 2.0, 0.0, -12.0),
                )
            })
            .collect();
        let mut model = Model {
            workspace: true,
            document: Some(crate::project::Document::new(scene.clone())),
            session: Some(crate::session::Session::from_scene(&scene)),
            ..Default::default()
        };
        let before = model.session.as_ref().unwrap().world.checkpoint();
        let original_scene = model.document.as_ref().unwrap().scene.clone();
        let original_dirty = model.document.as_ref().unwrap().dirty();
        let mut painter = Painter::new();
        model.editing = Some((254, "ROV 15".into()));
        crate::editor::commit(&mut model).unwrap();
        painter.paint(1100, 720, &model, None);
        assert!(painter.hits.iter().any(|h| h.id == 315));
        assert!(!painter.hits.iter().any(|h| h.id == 301));
        model.outliner_filter.clear();
        model.content_filter = 1;
        let mut reached = std::collections::BTreeSet::new();
        let mut actors = std::collections::BTreeSet::new();
        for page in 0..20 {
            model.content_page = page;
            model.outliner_scroll = page;
            painter.paint(900, 620, &model, None);
            for hit in &painter.hits {
                if (400..=415).contains(&hit.id) {
                    reached.insert(hit.id);
                }
                if (300..=315).contains(&hit.id) {
                    actors.insert(hit.id);
                }
            }
        }
        assert_eq!(reached.len(), 16);
        assert_eq!(actors.len(), 16);
        assert_eq!(before, model.session.as_ref().unwrap().world.checkpoint());
        assert_eq!(original_scene, model.document.as_ref().unwrap().scene);
        assert_eq!(original_dirty, model.document.as_ref().unwrap().dirty());
    }
    #[test]
    fn fast_bilinear_scaling_agrees_with_float_reconstruction() {
        let mut painter = Painter::new();
        painter.width = 29;
        painter.height = 19;
        painter.pixels = vec![0; 29 * 19];
        let frame = pelagic::render::Frame {
            width: 7,
            height: 5,
            rgb: (0..7 * 5 * 3).map(|i| (i * 17 % 256) as u8).collect(),
        };
        painter.filtered_frame(&frame, 0, 0, 28, 18, 4.0);
        for y in 0..18 {
            for x in 0..28 {
                let expected = filtered_pixel(
                    &frame,
                    (x as f32 + 0.5) / 4.0 - 0.5,
                    (y as f32 + 0.5) / 4.0 - 0.5,
                );
                let actual = painter.pixels[y * 29 + x];
                for shift in [0, 8, 16] {
                    assert!(
                        (((expected >> shift) & 255) as i32 - ((actual >> shift) & 255) as i32)
                            .abs()
                            <= 2
                    );
                }
            }
        }
    }
    #[test]
    fn high_dpi_controls_keep_their_logical_size() {
        let mut painter = Painter::new();
        let model = Model::default();
        for dpi in [1.0, 1.5, 2.0] {
            painter.dpi_scale = dpi;
            painter.paint(
                (1100.0 * dpi) as usize,
                (720.0 * dpi) as usize,
                &model,
                None,
            );
            let button = painter.hits.iter().find(|h| h.id == 2).unwrap();
            let physical = button.rect[3] * painter.scale;
            assert!((physical / dpi - 32.0).abs() < 0.01);
        }
    }
    #[test]
    fn reduced_motion_updates_feedback_without_animation() {
        let mut painter = Painter::new();
        let model = Model {
            reduced_motion: true,
            hover: Some(3),
            ..Default::default()
        };
        assert!(!painter.animate_buttons(&model, 0.001));
        assert_eq!(painter.hover_amount[3], 1.0);
    }
    #[test]
    fn creation_and_editor_hit_targets_fit_supported_window_sizes() {
        let mut painter = Painter::new();
        let scene = crate::project::Scene::default();
        let mut model = Model {
            new_project: true,
            new_name: "Test ocean".into(),
            new_folder: "C:\\Projects".into(),
            document: Some(crate::project::Document::new(scene.clone())),
            session: Some(crate::session::Session::from_scene(&scene)),
            ..Default::default()
        };
        for (workspace, tab, mode, page) in [
            (false, 0, 0, 0),
            (true, 0, 0, 0),
            (true, 1, 0, 0),
            (true, 2, 0, 0),
            (true, 3, 0, 0),
            (true, 4, 2, 0),
            (true, 0, 1, 1),
            (true, 1, 1, 1),
            (true, 2, 1, 1),
            (true, 0, 1, 2),
            (true, 1, 1, 2),
            (true, 0, 4, 3),
            (true, 1, 1, 3),
            (true, 2, 6, 3),
            (true, 3, 1, 3),
            (true, 0, 3, 4),
            (true, 0, 5, 5),
        ] {
            model.inspector_page = page;
            model.tool_tab = tab.min(3);
            model.inspect_ocean = true;
            model.environment_tab = tab;
            model.viewport_mode = mode;
            model.new_project = !workspace;
            model.workspace = workspace;
            for (width, height) in [(900, 620), (1100, 720), (1920, 990)] {
                painter.paint(width, height, &model, None);
                let right = width as f32 / painter.scale;
                let bottom = height as f32 / painter.scale;
                for hit in &painter.hits {
                    let [x, y, w, h] = hit.rect;
                    assert!(
                        x >= 0.0 && y >= 0.0 && x + w <= right && y + h <= bottom,
                        "control {} is outside {width}x{height}",
                        hit.id
                    );
                }
                for (i, a) in painter.hits.iter().enumerate() {
                    for b in &painter.hits[i + 1..] {
                        let overlap = a.rect[0] < b.rect[0] + b.rect[2]
                            && a.rect[0] + a.rect[2] > b.rect[0]
                            && a.rect[1] < b.rect[1] + b.rect[3]
                            && a.rect[1] + a.rect[3] > b.rect[1];
                        assert!(!overlap, "controls {} and {} overlap", a.id, b.id);
                    }
                    if crate::editor::is_field(a.id) && !workspace {
                        let label = [a.rect[0], a.rect[1] - 23.0, a.rect[2], 14.0];
                        for b in &painter.hits {
                            if b.id != a.id {
                                let overlap = label[0] < b.rect[0] + b.rect[2]
                                    && label[0] + label[2] > b.rect[0]
                                    && label[1] < b.rect[1] + b.rect[3]
                                    && label[1] + label[3] > b.rect[1];
                                assert!(!overlap, "label {} overlaps control {}", a.id, b.id);
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn hover_eases_in_and_out_then_stops() {
        let mut painter = Painter::new();
        let mut model = Model {
            hover: Some(3),
            ..Default::default()
        };
        assert!(painter.animate_buttons(&model, 1.0 / 60.0));
        assert!(painter.hover_amount[3] > 0.0 && painter.hover_amount[3] < 0.5);
        for _ in 0..30 {
            painter.animate_buttons(&model, 1.0 / 60.0);
        }
        assert_eq!(painter.hover_amount[3], 1.0);
        assert!(!painter.animate_buttons(&model, 1.0 / 60.0));
        model.hover = None;
        assert!(painter.animate_buttons(&model, 1.0 / 60.0));
        assert!(painter.hover_amount[3] > 0.5 && painter.hover_amount[3] < 1.0);
        for _ in 0..30 {
            painter.animate_buttons(&model, 1.0 / 60.0);
        }
        assert_eq!(painter.hover_amount[3], 0.0);
        assert!(!painter.animate_buttons(&model, 1.0 / 60.0));
    }
}
