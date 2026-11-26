use crate::{
    project::{self, Document, Scene},
    session::Session,
    view::Model,
};
use std::path::{Path, PathBuf};

pub(crate) fn storage(model: &Model) -> PathBuf {
    model.storage_root.clone().unwrap_or_else(project::data_dir)
}
pub fn is_field(id: usize) -> bool {
    matches!(id,20|21|40..=44|50..=54|68|70..=77|80..=83|109..=120|130..=137|150|151|160..=169|174..=178|180|254|296..=298|423..=427)
}
pub fn preferences(model: &mut Model) {
    if let Ok(bytes) = std::fs::read(storage(model).join("preferences.json"))
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
    {
        model.dark_mode = v["dark"].as_bool().unwrap_or(false);
        model.muted = v["muted"].as_bool().unwrap_or(false);
        if v["workspace"]["version"].as_u64() == Some(1) {
            model.layout = serde_json::from_value::<crate::layout::LayoutSettings>(
                v["workspace"]["panels"].clone(),
            )
            .unwrap_or_default()
            .sanitized();
            model.content_collapsed = v["workspace"]["content_collapsed"]
                .as_bool()
                .unwrap_or(false);
        }
        model.recent = v["recent"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .take(8)
                    .collect()
            })
            .unwrap_or_default();
    }
    model.recovery_available = storage(model).join("recovery.json").is_file();
}
pub fn save_preferences(model: &Model) -> Result<(), String> {
    project::atomic_json(
        &storage(model).join("preferences.json"),
        &serde_json::json!({"dark":model.dark_mode,"muted":model.muted,"recent":model.recent,"workspace":{"version":1,"panels":model.layout.sanitized(),"content_collapsed":model.content_collapsed}}),
    )
}
pub fn resize_panel(
    model: &mut Model,
    dimensions: [f32; 2],
    splitter: crate::layout::Splitter,
    value: f32,
) -> Result<(), String> {
    model.viewport_maximized = false;
    if splitter == crate::layout::Splitter::Content {
        model.content_collapsed = false;
    }
    let layout = crate::layout::EditorLayout::new(
        dimensions[0],
        dimensions[1],
        model.layout,
        model.content_collapsed,
        false,
    );
    layout.set_value(&mut model.layout, splitter, value);
    save_preferences(model)
}
pub fn reset_layout(model: &mut Model) -> Result<(), String> {
    model.layout = Default::default();
    model.viewport_maximized = false;
    model.content_collapsed = false;
    model.focus = None;
    save_preferences(model)
}
pub fn install(model: &mut Model, document: Document) {
    model.selection_empty = false;
    model.transform_preview = None;
    archive_run(model);
    model.inspector_page = 0;
    model.selected_entity = None;
    model.playback = None;
    model.project = Some(document.scene.name.clone());
    model.project_path = document.path.as_ref().map(|p| p.to_string_lossy().into());
    if let Some(session) = model.session.as_mut() {
        session.reset_scene(&document.scene);
    } else {
        model.session = Some(Session::from_scene(&document.scene));
    }
    model.document = Some(document);
    model.workspace = true;
    model.viewport_mode = if model
        .document
        .as_ref()
        .is_some_and(|d| d.scene.camera[2] >= 0.0)
    {
        0
    } else {
        1
    };
    model.inspect_structure = false;
    model.wireframe = false;
    model.new_project = false;
    model.inspect_ocean = true;
    model.focus = None;
    model.hover = None;
    model.editing = None;
    model.notice.clear();
    remember(model);
}
pub fn remember(model: &mut Model) {
    if let Some(path) = &model.project_path {
        model.recent.retain(|p| p != path);
        model.recent.insert(0, path.clone());
        model.recent.truncate(8);
    }
    if let Err(e) = save_preferences(model) {
        model.notice = format!("Preferences: {e}");
    }
}
pub fn recover(model: &mut Model) -> Result<(), String> {
    let doc = Document::restore(&storage(model).join("recovery.json"))?;
    install(model, doc);
    model.notice = "Recovered scene. Save it to keep these changes.".into();
    Ok(())
}
pub fn recovery(model: &mut Model) -> Result<(), String> {
    if let Some(doc) = &model.document
        && doc.dirty()
    {
        doc.recover(&storage(model).join("recovery.json"))?;
        model.recovery_available = true;
    } else if model.document.is_some() && model.recovery_available {
        let path = storage(model).join("recovery.json");
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        model.recovery_available = false;
    }
    Ok(())
}
pub fn save(model: &mut Model, path: &Path) -> Result<(), String> {
    model
        .document
        .as_mut()
        .ok_or("No project open.")?
        .save(path)?;
    model.project_path = Some(path.to_string_lossy().into());
    model.recovery_available = false;
    let recovery = storage(model).join("recovery.json");
    if recovery.exists() {
        std::fs::remove_file(recovery).map_err(|e| e.to_string())?;
    }
    model.notice = "Scene and camera saved. Simulation progress is temporary.".into();
    remember(model);
    Ok(())
}
pub fn create(model: &mut Model) -> Result<(), String> {
    let name = model.new_name.trim();
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || reserved_name(name)
    {
        return Err("Use a name without filename punctuation or reserved device names.".into());
    }
    let folder = PathBuf::from(model.new_folder.trim());
    if !folder.is_absolute() {
        return Err("Choose an absolute location.".into());
    }
    let mut scene = Scene {
        name: name.into(),
        vehicle: model.new_vehicle,
        ..Scene::default()
    };
    if !model.new_vehicle {
        scene.school.count = 0;
        scene.particles = 0;
        scene.structure = false;
    }
    scene.validate()?;
    let path = folder.join(name).join(format!("{name}.pelagic"));
    if path.exists() {
        return Err("That project already exists. Choose another name or open it.".into());
    }
    let mut doc = Document::new(scene);
    doc.save(&path)?;
    install(model, doc);
    Ok(())
}
fn reserved_name(name: &str) -> bool {
    let upper = name.split('.').next().unwrap_or(name).to_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            upper
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.len() == 1 && suffix.as_bytes()[0].is_ascii_digit())
        })
}
pub fn field_value(model: &Model, id: usize) -> String {
    if id == 254 {
        return model.outliner_filter.clone();
    }
    if id >= 109 {
        return tool_value(model, id).to_string();
    }
    match id {
        20 => model.new_name.clone(),
        21 => model.new_folder.clone(),
        _ => {
            let scene = model
                .document
                .as_ref()
                .map(|d| d.scene.clone())
                .unwrap_or_default();
            format!(
                "{}",
                match id {
                    40 => scene.bottom_depth,
                    41 => scene.density,
                    42 => scene.current[0],
                    43 => scene.attenuation,
                    44 => scene.vehicle_depth,
                    50 => scene.wave_height,
                    51 => scene.wave_period,
                    52 => scene.wave_direction,
                    53 => scene.wave_spread,
                    54 => scene.sun_elevation,
                    68 => scene.grid.as_ref().map_or(4.0, |g| g.spacing),
                    70 => scene.temperature,
                    71 => scene.deep_temperature,
                    72 => scene.salinity,
                    73 => scene.thermocline,
                    74 => scene.shear,
                    75 => scene.tide_period,
                    76 => scene.current[1],
                    77 => scene.current[2],
                    80 => scene.structure_position[0],
                    81 => scene.structure_position[1],
                    82 => scene.structure_position[2],
                    83 => scene.structure_scale,
                    _ => 0.0,
                }
            )
        }
    }
}
pub fn commit(model: &mut Model) -> Result<(), String> {
    let Some((id, value)) = model.editing.clone() else {
        return Ok(());
    };
    match id {
        20 => model.new_name = value,
        21 => model.new_folder = value,
        254 => {
            model.outliner_filter = value.chars().take(80).collect();
            model.outliner_scroll = 0;
        }
        _ => {
            let number = value
                .trim()
                .parse::<f64>()
                .map_err(|_| "Enter a valid number.")?;
            if id >= 109 {
                commit_tool(model, id, number)?;
                model.editing = None;
                return Ok(());
            }
            let mut scene = model
                .document
                .as_ref()
                .ok_or("No scene open.")?
                .scene
                .clone();
            match id {
                40 => scene.bottom_depth = number,
                41 => scene.density = number,
                42 => scene.current[0] = number,
                43 => scene.attenuation = number,
                44 => scene.vehicle_depth = number,
                50 => scene.wave_height = number,
                51 => scene.wave_period = number,
                52 => scene.wave_direction = number,
                53 => scene.wave_spread = number,
                54 => scene.sun_elevation = number,
                68 => {
                    if !number.is_finite() || !(0.1..=5000.0).contains(&number) {
                        return Err("Grid spacing must be 0.1–1000 m.".into());
                    }
                    if let Some(grid) = scene.grid.as_mut() {
                        std::sync::Arc::make_mut(grid).spacing = number;
                    }
                }
                70 => scene.temperature = number,
                71 => scene.deep_temperature = number,
                72 => scene.salinity = number,
                73 => scene.thermocline = number,
                74 => scene.shear = number,
                75 => scene.tide_period = number,
                76 => scene.current[1] = number,
                77 => scene.current[2] = number,
                80 => scene.structure_position[0] = number,
                81 => scene.structure_position[1] = number,
                82 => scene.structure_position[2] = number,
                83 => scene.structure_scale = number,
                _ => {}
            }
            edit(model, scene, id != 54)?;
        }
    }
    model.editing = None;
    Ok(())
}
pub fn edit(model: &mut Model, scene: Scene, rebuild: bool) -> Result<(), String> {
    if !model.failed_runs.is_empty() || model.pending_saves.len() >= 4 {
        return Err(
            "Finish recording backups first. Experiments > Run actions > Retry failed backups."
                .into(),
        );
    }
    scene.validate()?;
    if rebuild {
        archive_run(model);
        model.playback = None;
    }
    let doc = model.document.as_mut().ok_or("No scene open.")?;
    doc.edit(scene)?;
    if rebuild {
        if let Some(session) = model.session.as_mut() {
            session.reset_scene(&doc.scene);
        } else {
            model.session = Some(Session::from_scene(&doc.scene));
        }
        if model.wireframe
            && let Some(session) = model.session.as_mut()
        {
            session.wireframe(true);
        }
        model.notice = "Scene updated; preview reset.".into();
    } else if let Some(session) = model.session.as_mut() {
        session.set_camera(&doc.scene);
    }
    recovery(model)
}
pub fn commit_transform(model: &mut Model, base: &Scene, next: Scene) -> Result<(), String> {
    if model.document.as_ref().is_none_or(|d| d.scene != *base) {
        return Err("Scene changed during the gesture; transform cancelled.".into());
    }
    if next == *base {
        return Ok(());
    }
    edit(model, next, true)
}
pub fn history(model: &mut Model, redo: bool) -> Result<(), String> {
    if !model.failed_runs.is_empty() || model.pending_saves.len() >= 4 {
        return Err("Finish recording backups before changing history.".into());
    }
    archive_run(model);
    model.playback = None;
    let doc = model.document.as_mut().ok_or("No scene open.")?;
    if if redo { doc.redo() } else { doc.undo() } {
        if let Some(session) = model.session.as_mut() {
            session.reset_scene(&doc.scene);
        } else {
            model.session = Some(Session::from_scene(&doc.scene));
        }
        if model.wireframe
            && let Some(session) = model.session.as_mut()
        {
            session.wireframe(true);
        }
        model.notice = if redo {
            "Edit restored; preview reset."
        } else {
            "Edit undone; preview reset."
        }
        .into();
        recovery(model)?;
    }
    Ok(())
}
pub fn archive_run(model: &mut Model) {
    if let Some(session) = model.session.as_mut()
        && let Some(run) = session.capture.take()
    {
        keep_run(model, std::sync::Arc::new(run));
    }
}
pub fn keep_run(model: &mut Model, run: std::sync::Arc<pelagic::experiment::Recording>) {
    model.run = Some(run.clone());
    let path = storage(model).join("Runs").join(format!(
        "run-{}.pelagic-run",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    model.pending_saves.push(crate::backups::submit(path, run));
}
/// Writes the mounted sensor camera's current view (reference CPU renderer)
/// as a PNG with a pinhole calibration sidecar (`<name>.camera.json`).
pub fn export_camera(model: &Model, path: &Path) -> Result<(), String> {
    let (Some(session), Some(doc)) = (&model.session, &model.document) else {
        return Err("Open a project first.".into());
    };
    let frame = session.camera(&doc.scene);
    let fov = doc.scene.sensors.camera_fov;
    let mount = doc.scene.sensors.mount;
    let time = session
        .display
        .as_ref()
        .map_or(session.world.time(), |s| s.time);
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        frame.width as u32,
        frame.height as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|e| e.to_string())?
        .write_image_data(&frame.rgb)
        .map_err(|e| e.to_string())?;
    let focal = frame.height as f64 / (2.0 * (fov.to_radians() * 0.5).tan());
    crate::project::atomic_json(
        &path.with_extension("camera.json"),
        &serde_json::json!({"time_s":time,"width":frame.width,"height":frame.height,"fx":focal,"fy":focal,"cx":frame.width as f64*0.5,"cy":frame.height as f64*0.5,"mount_body_m":mount,"forward_body":[1.0,0.0,-0.15],"model":"pinhole; approximate underwater optics"}),
    )
}
pub fn tool_value(model: &Model, id: usize) -> f64 {
    if id == 109 {
        return model.playback.unwrap_or(0) as f64;
    }
    if id == 180 {
        return model.batch_seconds.max(1.0);
    }
    let Some(doc) = &model.document else {
        return 0.0;
    };
    let s = &doc.scene;
    let e = model
        .selected_entity
        .and_then(|i| s.entities.get(i))
        .cloned()
        .unwrap_or_else(|| s.reference_entity());
    let c = &s.sensors;
    match id {
        110 => e.position.x,
        111 => e.position.y,
        112 => e.position.z,
        113 => e.heading,
        114 => e.mass,
        115 => e.volume,
        116 => e.force.x,
        117 => e.model.hold_depth.unwrap_or(s.vehicle_depth),
        118 => e.model.hold_heading.unwrap_or(0.0).to_degrees(),
        119 => e.model.thrust_limit,
        120 => e.model.response_seconds,
        296 => e.model.half.x * 2.,
        297 => e.model.half.y * 2.,
        298 => e.model.half.z * 2.,
        423..=427 => e
            .material
            .and_then(|id| s.library.material(id))
            .map_or(0.0, |m| match id {
                423..=425 => m.base_color[id - 423] as f64,
                426 => m.roughness,
                _ => m.metallic,
            }),
        130 => s.school.count as f64,
        131 => s.school.speed,
        132 => s.school.radius,
        133 => -s.school.center.z,
        134 => s.school.size,
        137 => s.particles as f64,
        135 => s.school.separation,
        136 => s.school.avoidance,
        150 => s.provenance.origin_latitude,
        151 => s.provenance.origin_longitude,
        160 => c.rate_hz,
        161 => c.range,
        162 => c.beams as f64,
        163 => c.range_noise,
        164 => c.mount.z,
        165 => c.mount.x,
        166 => c.tilt_degrees,
        167 => c.camera_fov,
        168 => c.position_noise,
        169 => c.latency_seconds,
        174 => c.fan_degrees,
        175 => c.mount.y,
        176 => c.velocity_sigma,
        177 => c.gyro_sigma,
        178 => c.acceleration_sigma,
        _ => 0.0,
    }
}
pub fn replay(model: &mut Model, index: usize) -> Result<(), String> {
    let run = model.run.as_ref().ok_or("No recording loaded.")?;
    if let Some(doc) = &model.document {
        let value = serde_json::from_str(&run.scene_json).map_err(|e| e.to_string())?;
        let mut expected = Scene::from_value(&value)?;
        expected.camera = doc.scene.camera;
        expected.target = doc.scene.target;
        expected.sun_elevation = doc.scene.sun_elevation;
        expected.quality = doc.scene.quality;
        expected.terrain_material = doc.scene.terrain_material;
        expected.name = doc.scene.name.clone();
        if expected != doc.scene {
            return Err("Use Run actions → Replay to restore this recording's environment before scrubbing.".into());
        }
    }
    let frame = run
        .frames
        .get(index)
        .ok_or("Playback sample is out of range.")?;
    if let Some(session) = model.session.as_mut() {
        session.present(frame.truth.clone());
        session.observations = frame.observations.clone();
    }
    model.playback = Some(index);
    Ok(())
}
fn commit_tool(model: &mut Model, id: usize, v: f64) -> Result<(), String> {
    if !v.is_finite() {
        return Err("Enter a finite number.".into());
    }
    if id == 109 {
        if v < 0.0 || v.fract() != 0.0 {
            return Err("Use a whole sample index.".into());
        }
        return replay(model, v as usize);
    }
    if id == 180 {
        if !(0.1..=120.0).contains(&v) {
            return Err("Batch duration must be 0.1–120 s.".into());
        }
        model.batch_seconds = v;
        return Ok(());
    }
    let mut s = model
        .document
        .as_ref()
        .ok_or("No project open.")?
        .scene
        .clone();
    if (423..=427).contains(&id) {
        let actor = crate::viewport_edit::Actor::selected(model).ok_or("Select an actor first.")?;
        let material = actor
            .entity(&s)
            .and_then(|e| e.material)
            .ok_or("Assign a material on the Appearance tab first.")?;
        let next = crate::library::set_material_value(&s, material, id - 423, v)?;
        return edit(model, next, true);
    }
    if (296..=298).contains(&id) {
        let actor =
            crate::viewport_edit::Actor::selected(model).ok_or("Select an editable body first.")?;
        let e = actor.entity(&s).ok_or("Select an editable body first.")?;
        let half = e.model.half;
        let factor = match id {
            296 => pelagic::math::Vec3::new(v / (2. * half.x), 1., 1.),
            297 => pelagic::math::Vec3::new(1., v / (2. * half.y), 1.),
            _ => pelagic::math::Vec3::new(1., 1., v / (2. * half.z)),
        };
        crate::viewport_edit::resize_body(&mut s, actor, factor)?;
    } else if (110..=120).contains(&id) {
        let mut e = model
            .selected_entity
            .and_then(|i| s.entities.get(i))
            .cloned()
            .unwrap_or_else(|| s.reference_entity());
        match id {
            110 => e.position.x = v,
            111 => e.position.y = v,
            112 => e.position.z = v,
            113 => e.heading = v,
            114 => e.mass = v,
            115 => e.volume = v,
            116 => e.force.x = v,
            117 => e.model.hold_depth = Some(v),
            118 => e.model.hold_heading = Some(v.to_radians()),
            119 => e.model.thrust_limit = v,
            120 => e.model.response_seconds = v,
            _ => {}
        }
        if let Some(i) = model.selected_entity {
            s.entities[i] = e;
        } else {
            s.set_reference(&e);
        }
    } else {
        match id {
            130 => {
                if v.fract() != 0.0 || !(0.0..=256.0).contains(&v) {
                    return Err("Use 0–256 whole fish.".into());
                }
                s.school.count = v as usize
            }
            131 => s.school.speed = v,
            132 => s.school.radius = v,
            133 => s.school.center.z = -v,
            134 => s.school.size = v,
            137 => {
                if v.fract() != 0.0 || !(0.0..=512.0).contains(&v) {
                    return Err("Use 0–512 whole tracers.".into());
                }
                s.particles = v as usize;
            }
            135 => s.school.separation = v,
            136 => s.school.avoidance = v,
            150 => {
                if s.provenance.classification != "synthetic" {
                    return Err("Imported geographic origin is fixed. Generate a synthetic region to change location.".into());
                }
                s.provenance.origin_latitude = v
            }
            151 => {
                if s.provenance.classification != "synthetic" {
                    return Err("Imported geographic origin is fixed. Generate a synthetic region to change location.".into());
                }
                s.provenance.origin_longitude = v
            }
            160 => s.sensors.rate_hz = v,
            161 => s.sensors.range = v,
            162 => {
                if v.fract() != 0.0 || !(1.0..=128.0).contains(&v) {
                    return Err("Use 1–128 whole beams.".into());
                }
                s.sensors.beams = v as usize
            }
            163 => s.sensors.range_noise = v,
            164 => s.sensors.mount.z = v,
            165 => s.sensors.mount.x = v,
            166 => s.sensors.tilt_degrees = v,
            167 => s.sensors.camera_fov = v,
            168 => s.sensors.position_noise = v,
            169 => s.sensors.latency_seconds = v,
            174 => s.sensors.fan_degrees = v,
            175 => s.sensors.mount.y = v,
            176 => s.sensors.velocity_sigma = v,
            177 => s.sensors.gyro_sigma = v,
            178 => s.sensors.acceleration_sigma = v,
            _ => {}
        }
    }
    edit(model, s, true)
}
pub fn orbit_scene(scene: &Scene, dx: f64, dy: f64) -> Scene {
    let mut result = scene.clone();
    let offset = [
        scene.camera[0] - scene.target[0],
        scene.camera[1] - scene.target[1],
        scene.camera[2] - scene.target[2],
    ];
    let r = offset.iter().map(|x| x * x).sum::<f64>().sqrt().max(0.1);
    let yaw = offset[1].atan2(offset[0]) + dx * 0.007;
    let pitch = ((offset[2] / r).asin() + dy * 0.007).clamp(-1.4, 1.4);
    result.camera = [
        scene.target[0] + r * pitch.cos() * yaw.cos(),
        scene.target[1] + r * pitch.cos() * yaw.sin(),
        scene.target[2] + r * pitch.sin(),
    ];
    result
}
pub fn camera(model: &mut Model, key: usize, amount: f64) -> Result<(), String> {
    let mut scene = model
        .document
        .as_ref()
        .ok_or("No scene open.")?
        .scene
        .clone();
    if key == 70 {
        let original = Scene::default();
        scene.camera = original.camera;
        scene.target = original.target;
        if model.viewport_mode != 1
            && let Some(grid) = &scene.grid
        {
            let b = grid.bounds();
            let extent = (b[2] - b[0]).max(b[3] - b[1]);
            if extent > 1024.0 {
                let x = (b[0] + b[2]) * 0.5;
                let y = (b[1] + b[3]) * 0.5;
                scene.target = [x, y, 0.0];
                scene.camera = [x - extent * 0.35, y - extent * 0.45, extent * 0.3];
            }
        }
        if model.viewport_mode == 1 {
            let depth = scene.vehicle_depth.min(scene.bottom_depth * 0.7);
            scene.target = [0.0, 0.0, -depth];
            scene.camera = [-12.0, -16.0, -depth + 6.0];
        }
    } else if key == 0 {
        let factor = (-amount * 0.12).exp();
        let distance = scene
            .camera
            .iter()
            .zip(scene.target)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        let max_distance = scene.grid.as_ref().map_or(5000.0, |g| {
            let b = g.bounds();
            ((b[2] - b[0]).hypot(b[3] - b[1]) * 2.0).clamp(5000.0, 100000.0)
        });
        let next = (distance * factor).clamp(2.0, max_distance);
        for i in 0..3 {
            scene.camera[i] =
                scene.target[i] + (scene.camera[i] - scene.target[i]) * next / distance;
        }
    } else {
        let x = scene.target[0] - scene.camera[0];
        let y = scene.target[1] - scene.camera[1];
        let len = x.hypot(y).max(0.01);
        let direction = match key {
            87 => [x / len, y / len, 0.0],
            83 => [-x / len, -y / len, 0.0],
            65 => [-y / len, x / len, 0.0],
            68 => [y / len, -x / len, 0.0],
            81 => [0.0, 0.0, -1.0],
            69 => [0.0, 0.0, 1.0],
            _ => [0.0; 3],
        };
        for (i, d) in direction.iter().enumerate() {
            scene.camera[i] += d * amount;
            scene.target[i] += d * amount;
        }
    }
    edit(model, scene, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_preferences_roundtrip_without_touching_the_project_or_world() {
        let root = std::env::temp_dir().join(format!(
            "pelagic-layout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let scene = Scene::robotics_demo();
        let mut model = Model {
            document: Some(Document::new(scene.clone())),
            session: Some(Session::from_scene(&scene)),
            selected_entity: Some(1),
            storage_root: Some(root.clone()),
            dark_mode: true,
            muted: true,
            recent: vec!["C:\\Projects\\ocean.pelagic".into()],
            ..Default::default()
        };
        let before = model.session.as_ref().unwrap().world.checkpoint();
        let dirty = model.document.as_ref().unwrap().dirty();
        for (splitter, value) in [
            (crate::layout::Splitter::Sidebar, 410.0),
            (crate::layout::Splitter::Outliner, 350.0),
            (crate::layout::Splitter::Content, 300.0),
        ] {
            resize_panel(&mut model, [1920.0, 990.0], splitter, value).unwrap();
        }
        model.viewport_maximized = true;
        model.content_collapsed = true;
        save_preferences(&model).unwrap();
        let mut reloaded = Model {
            storage_root: Some(root.clone()),
            ..Default::default()
        };
        preferences(&mut reloaded);
        assert_eq!(model.layout, reloaded.layout);
        assert!(reloaded.content_collapsed && !reloaded.viewport_maximized);
        assert!(reloaded.dark_mode && reloaded.muted);
        assert_eq!(reloaded.recent, model.recent);
        assert_eq!(before, model.session.as_ref().unwrap().world.checkpoint());
        assert_eq!(scene, model.document.as_ref().unwrap().scene);
        assert_eq!(dirty, model.document.as_ref().unwrap().dirty());
        assert_eq!(model.selected_entity, Some(1));
        reset_layout(&mut model).unwrap();
        preferences(&mut reloaded);
        assert_eq!(reloaded.layout, Default::default());
        assert!(!reloaded.content_collapsed);
        // Unknown schema and malformed saved sizes must not destroy other preferences.
        project::atomic_json(&root.join("preferences.json"), &serde_json::json!({"dark":true,"workspace":{"version":2,"panels":{"sidebar_width":450}}})).unwrap();
        preferences(&mut reloaded);
        assert_eq!(reloaded.layout, Default::default());
        assert!(reloaded.dark_mode);
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    fn regional_zoom_and_frame_preserve_the_region_scale() {
        let scene = crate::jobs::goa().unwrap();
        let root = std::env::temp_dir().join(format!(
            "pelagic-camera-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut model = Model {
            document: Some(Document::new(scene.clone())),
            storage_root: Some(root.clone()),
            ..Default::default()
        };
        let distance = |s: &Scene| {
            s.camera
                .iter()
                .zip(s.target)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        let before = distance(&scene);
        camera(&mut model, 0, -1.0).unwrap();
        let after = distance(&model.document.as_ref().unwrap().scene);
        assert!(after > before * 1.1 && after < before * 1.2);
        camera(&mut model, 70, 1.0).unwrap();
        assert!(model.document.as_ref().unwrap().scene.camera[2] > 1000.0);
        let target = root.canonicalize().unwrap();
        assert!(target.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(target).unwrap();
    }
    #[test]
    fn orbit_preserves_target_and_radius() {
        let scene = Scene::default();
        let next = orbit_scene(&scene, 150.0, 20.0);
        assert_eq!(scene.target, next.target);
        let radius = |s: &Scene| {
            s.camera
                .iter()
                .zip(s.target)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        assert!((radius(&scene) - radius(&next)).abs() < 1e-9);
        next.validate().unwrap();
    }
    #[test]
    fn full_project_edit_save_history_and_recovery_flow() {
        let root = std::env::temp_dir().join(format!(
            "pelagic-editor-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut model = Model {
            new_name: "My ocean".into(),
            new_folder: root.join("Projects").to_string_lossy().into(),
            storage_root: Some(root.join("settings")),
            ..Default::default()
        };
        create(&mut model).unwrap();
        assert!(model.workspace);
        assert!(model.session.as_ref().unwrap().world.vehicles().is_empty());
        let initial = model.document.as_ref().unwrap().scene.clone();
        model.editing = Some((40, "80".into()));
        commit(&mut model).unwrap();
        assert_eq!(model.document.as_ref().unwrap().scene.bottom_depth, 80.0);
        model.editing = Some((41, "NaN".into()));
        assert!(commit(&mut model).is_err());
        assert_eq!(
            model.document.as_ref().unwrap().scene.density,
            initial.density
        );
        model.editing = None;
        history(&mut model, false).unwrap();
        assert_eq!(model.document.as_ref().unwrap().scene, initial);
        history(&mut model, true).unwrap();
        assert_eq!(model.document.as_ref().unwrap().scene.bottom_depth, 80.0);
        camera(&mut model, 0, 1.0).unwrap();
        camera(&mut model, 87, 3.0).unwrap();
        let edited = model.document.as_ref().unwrap().scene.clone();
        assert_ne!(edited.camera, initial.camera);
        let path = model.document.as_ref().unwrap().path.clone().unwrap();
        save(&mut model, &path).unwrap();
        assert_eq!(Document::open(&path).unwrap().scene, edited);
        assert!(!model.document.as_ref().unwrap().dirty());
        model.editing = Some((42, "1.2".into()));
        commit(&mut model).unwrap();
        assert!(model.recovery_available);
        let mut restored = Model {
            storage_root: Some(root.join("settings")),
            ..Default::default()
        };
        recover(&mut restored).unwrap();
        assert_eq!(restored.document.as_ref().unwrap().scene.current[0], 1.2);
        assert!(restored.document.as_ref().unwrap().dirty());
        model.dark_mode = true;
        model.muted = true;
        save_preferences(&model).unwrap();
        preferences(&mut restored);
        assert!(restored.dark_mode && restored.muted);
        assert_eq!(
            restored.recent.first(),
            Some(&path.to_string_lossy().into_owned())
        );
        save(&mut restored, &path).unwrap();
        assert!(!root.join("settings/recovery.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
