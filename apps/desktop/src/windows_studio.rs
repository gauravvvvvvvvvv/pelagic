//! Native menus and asynchronous authoring/experiment actions.
use super::*;
use crate::{
    jobs::{Event, Job, Output},
    project::{Document, Scene},
};
use std::sync::{Arc, atomic::Ordering};

unsafe fn start_job(hwnd: HWND, app: &mut App, job: Job) {
    unsafe {
        if app.job.is_some() {
            app.model.notice = "A task is already running; cancel or wait.".into();
            return;
        }
        app.job = Some(job);
        app.model.job_running = true;
        app.model.job_progress = "Working...".into();
        SetTimer(hwnd, 5, 50, None);
    }
}
pub(super) unsafe fn poll_job(hwnd: HWND, app: &mut App) {
    unsafe {
        let mut finished = None;
        if let Some(job) = &app.job {
            while let Ok(event) = job.events.try_recv() {
                match event {
                    Event::Progress(text) => app.model.job_progress = text,
                    Event::Finished(result) => {
                        finished = Some(result);
                        break;
                    }
                }
            }
        }
        if let Some(result) = finished {
            app.job = None;
            app.model.job_running = false;
            KillTimer(hwnd, 5);
            match result {
                Ok(Output::Gpu(gpu, request)) => {
                    if request != app.model.backend_generation {
                        app.model.job_progress =
                            "Renderer request superseded; current backend kept.".into();
                        return;
                    }
                    let name = gpu.adapter.name.clone();
                    if let Some(session) = app.model.session.as_mut() {
                        session.install_gpu(gpu);
                        app.model.wireframe = false;
                        app.model.notice = if session.backend_notice.is_empty() {
                            format!(
                                "GPU preview: {name}. Camera dataset exports retain the reference CPU renderer."
                            )
                        } else {
                            session.backend_notice.clone()
                        };
                    }
                }
                Ok(Output::Run(run)) => {
                    crate::editor::keep_run(&mut app.model, Arc::new(*run));
                    app.model.playback = None;
                    app.model.inspector_page = 4;
                    app.model.viewport_mode = 3;
                    app.model.job_progress = "Run ready / use Replay to inspect truth".into();
                }
                Ok(Output::Notice(text)) => {
                    app.model.job_progress = text.clone();
                    app.model.notice = text;
                }
                Ok(Output::Campaign(_))
                | Ok(Output::Trials(_))
                | Ok(Output::Seakeeping(_))
                | Ok(Output::Sar(_))
                | Ok(Output::Survey(_))
                | Ok(Output::Colregs(_))
                | Ok(Output::Spill(_))
                | Ok(Output::Inspection(_)) => {
                    app.model.notice = "Campaigns and trials are shown in the GPU editor.".into();
                }
                Ok(Output::Scene { before, after }) => {
                    if app
                        .model
                        .document
                        .as_ref()
                        .is_some_and(|d| d.scene == *before)
                    {
                        if let Err(error) = crate::editor::edit(&mut app.model, *after, true) {
                            app.model.notice = error;
                        } else {
                            app.model.notice =
                                "Imported asset/data is embedded in the project.".into();
                        }
                    } else {
                        app.model.notice =
                            "Scene changed during import. Import again to apply to this scene."
                                .into();
                    }
                }
                Err(error) => {
                    app.model.notice = error.clone();
                    app.model.job_progress = error;
                }
            }
            update_viewport(hwnd, app);
            if !app.model.pending_saves.is_empty() {
                SetTimer(hwnd, 6, 100, None);
            }
        }
    }
}
fn unique_id(scene: &Scene) -> u64 {
    scene.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1
}
unsafe fn apply(hwnd: HWND, app: &mut App, scene: Scene) {
    unsafe {
        pause(hwnd, app);
        if let Err(error) = crate::editor::edit(&mut app.model, scene, true) {
            app.model.notice = error;
        }
    }
}
unsafe fn entity_menu(hwnd: HWND, app: &mut App) {
    unsafe {
        let Some(doc) = &app.model.document else {
            return;
        };
        let mut labels = vec!["Reference vehicle".to_string()];
        labels.extend(doc.scene.entities.iter().map(|e| e.name.clone()));
        let refs: Vec<_> = labels.iter().map(String::as_str).collect();
        if let Some(i) = pick_menu(hwnd, &refs, app.model.selected_entity.map_or(0, |i| i + 1)) {
            app.model.selected_entity = i.checked_sub(1);
            app.model.inspector_page = 1;
            app.model.tool_tab = 0;
        }
    }
}
unsafe fn import_asset(hwnd: HWND, app: &mut App) {
    unsafe {
        if app.job.is_some() {
            app.model.notice = "Wait for or cancel the current task.".into();
            return;
        }
        let Some(path) = pick_file(
            hwnd,
            false,
            "Import geometry-only OBJ",
            "Wavefront OBJ\0*.obj\0",
            "obj",
        ) else {
            return;
        };
        let Some(doc) = &app.model.document else {
            return;
        };
        let before = doc.scene.clone();
        let selected = app.model.selected_entity;
        start_job(
            hwnd,
            app,
            Job::start(move |_, _| {
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
                    .unwrap_or("OBJ")
                    .to_string();
                let mesh =
                    pelagic::assets::Mesh::parse_obj(&text, &name).map_err(str::to_string)?;
                // Adds to the project library (identical geometry is reused) and
                // assigns it to the selected body, or places a new instance.
                let (after, _, _) = crate::library::import(&before, &name, mesh, selected)?;
                Ok(Output::Scene {
                    before: Box::new(before),
                    after: Box::new(after),
                })
            }),
        );
    }
}
/// Actor targeted by library commands: `Some(None)` is the reference vehicle.
fn library_target(app: &App) -> Option<Option<usize>> {
    let doc = app.model.document.as_ref()?;
    match crate::viewport_edit::Actor::selected(&app.model)? {
        crate::viewport_edit::Actor::Reference => Some(None),
        crate::viewport_edit::Actor::Entity(id) => {
            doc.scene.entities.iter().position(|e| e.id == id).map(Some)
        }
        _ => None,
    }
}
unsafe fn library_action(hwnd: HWND, app: &mut App, id: usize) {
    use crate::library::{self, Item};
    unsafe {
        let Some(doc) = &app.model.document else {
            return;
        };
        let scene = doc.scene.clone();
        let blocked = app.job.is_some() || app.model.transform_preview.is_some();
        match id {
            417 => {
                app.model.content_filter = 4;
                app.model.content_page = 0;
            }
            448..=479 => {
                if let Some(asset) = scene.library.meshes.get(id - library::MESH_CARD) {
                    let item = Item::Mesh(asset.id);
                    app.model.selected_library = Some(item);
                    app.model.notice = format!(
                        "{} selected · {} use(s). Place adds a new body; Assign sets the selected actor's mesh.",
                        asset.name,
                        library::usage(&scene, item)
                    );
                }
            }
            480..=511 => {
                if let Some((item, material)) = library::material_items(&scene)
                    .into_iter()
                    .nth(id - library::MATERIAL_CARD)
                {
                    app.model.selected_library = Some(item);
                    app.model.notice = format!(
                        "{} selected. Assign paints the selected actor{}.",
                        material.name,
                        if matches!(item, Item::Preset(_)) {
                            " and copies the preset into the project"
                        } else {
                            ""
                        }
                    );
                }
            }
            _ if blocked => {
                app.model.notice = "Wait for the current task or gesture to finish.".into();
            }
            418 => {
                let Some(item) = app.model.selected_library else {
                    return;
                };
                let (mesh, material) = match item {
                    Item::Mesh(mesh) => (mesh, None),
                    _ => match library_target(app).and_then(|actor| match actor {
                        None => scene.reference_asset,
                        Some(i) => scene.entities[i].asset,
                    }) {
                        // A material with a mesh-bodied actor selected places
                        // another painted instance of that mesh.
                        Some(mesh) => (mesh, Some(item)),
                        None => {
                            app.model.notice =
                                "Select a mesh card to place, or a mesh-bodied actor to place a painted copy.".into();
                            return;
                        }
                    },
                };
                let near = scene.target;
                match library::place(&scene, mesh, material, near) {
                    Ok((next, index)) => {
                        apply(hwnd, app, next);
                        if app
                            .model
                            .document
                            .as_ref()
                            .is_some_and(|d| d.scene.entities.len() > index)
                        {
                            app.model.selected_entity = Some(index);
                            app.model.inspector_page = 1;
                            app.model.selection_empty = false;
                            app.model.inspect_structure = false;
                            app.model.inspect_ocean = false;
                            app.model.notice = format!(
                                "Placed {} at the view target. Undo removes it.",
                                library::name(&scene, Item::Mesh(mesh))
                            );
                        }
                    }
                    Err(error) => app.model.notice = error,
                }
            }
            419 | 421 | 422 => {
                let Some(actor) = library_target(app) else {
                    app.model.notice = "Select an actor in the Outliner or viewport first.".into();
                    return;
                };
                let result = if id == 419 {
                    let Some(item) = app.model.selected_library else {
                        return;
                    };
                    library::assign(&scene, actor, item)
                } else if id == 421 {
                    let mut labels = vec!["Procedural (built-in)".to_string()];
                    labels.extend(scene.library.meshes.iter().map(|m| m.name.clone()));
                    let current = match actor {
                        None => scene.reference_asset,
                        Some(i) => scene.entities[i].asset,
                    };
                    let index = current
                        .and_then(|a| scene.library.meshes.iter().position(|m| m.id == a))
                        .map_or(0, |i| i + 1);
                    let refs: Vec<_> = labels.iter().map(String::as_str).collect();
                    match pick_menu(hwnd, &refs, index) {
                        Some(0) => library::clear(&scene, actor, true),
                        Some(i) => library::assign(
                            &scene,
                            actor,
                            Item::Mesh(scene.library.meshes[i - 1].id),
                        ),
                        None => return,
                    }
                } else {
                    let items = library::material_items(&scene);
                    let mut labels = vec!["Default look".to_string()];
                    labels.extend(items.iter().map(|(item, m)| match item {
                        Item::Preset(_) => format!("{} (preset)", m.name),
                        _ => m.name.clone(),
                    }));
                    let current = match actor {
                        None => scene.reference_material,
                        Some(i) => scene.entities[i].material,
                    };
                    let index = current
                        .and_then(|c| items.iter().position(|(i, _)| *i == Item::Material(c)))
                        .map_or(0, |i| i + 1);
                    let refs: Vec<_> = labels.iter().map(String::as_str).collect();
                    match pick_menu(hwnd, &refs, index) {
                        Some(0) => library::clear(&scene, actor, false),
                        Some(i) => library::assign(&scene, actor, items[i - 1].0),
                        None => return,
                    }
                };
                match result {
                    Ok(next) if next != scene => {
                        apply(hwnd, app, next);
                        if app.model.notice.starts_with("Scene updated") {
                            app.model.notice = "Appearance updated; preview reset.".into();
                        }
                    }
                    Ok(_) => {}
                    Err(error) => app.model.notice = error,
                }
            }
            420 => {
                let Some(item) = app.model.selected_library else {
                    return;
                };
                match library::remove(&scene, item) {
                    Ok(next) => {
                        let name = library::name(&scene, item);
                        apply(hwnd, app, next);
                        app.model.selected_library = None;
                        app.model.notice =
                            format!("Removed {name} from the library. Undo restores it.");
                    }
                    Err(error) => app.model.notice = error,
                }
            }
            _ => {}
        }
        InvalidateRect(hwnd, null(), 0);
    }
}
unsafe fn manage(hwnd: HWND, app: &mut App) {
    unsafe {
        let Some(choice) = pick_menu(
            hwnd,
            &[
                "Add underwater ROV",
                "Add surface vessel",
                "Add passive drifter",
                "Duplicate selected entity",
                "Delete selected entity",
                "Import OBJ geometry",
                "Select harbour structure",
                "Toggle reference vehicle",
                "Toggle harbour structure",
            ],
            0,
        ) else {
            return;
        };
        if choice == 5 {
            import_asset(hwnd, app);
            return;
        }
        if choice == 6 {
            app.model.inspector_page = 0;
            app.model.inspect_ocean = false;
            app.model.inspect_structure = true;
            return;
        }
        let Some(doc) = &app.model.document else {
            return;
        };
        let mut scene = doc.scene.clone();
        let id = unique_id(&scene);
        let mut selected = app.model.selected_entity;
        match choice {
            0 => {
                scene.entities.push(pelagic::entities::Entity::rov(
                    id,
                    pelagic::math::Vec3::new(3.0, 0.0, -scene.vehicle_depth),
                ));
                selected = Some(scene.entities.len() - 1);
            }
            1 => {
                scene.entities.push(pelagic::entities::Entity::boat(id));
                selected = Some(scene.entities.len() - 1);
            }
            2 => {
                scene.entities.push(pelagic::entities::Entity::drifter(id));
                selected = Some(scene.entities.len() - 1);
            }
            3 => {
                let mut e = selected
                    .and_then(|i| scene.entities.get(i))
                    .cloned()
                    .unwrap_or_else(|| scene.reference_entity());
                e.id = id;
                e.name = format!("{} copy", e.name);
                e.position.x += 3.0;
                scene.entities.push(e);
                selected = Some(scene.entities.len() - 1);
            }
            4 => {
                if let Some(i) = selected {
                    if i < scene.entities.len() {
                        scene.entities.remove(i);
                    }
                    selected = None;
                } else {
                    scene.vehicle = false;
                }
            }
            7 => scene.vehicle = !scene.vehicle,
            8 => scene.structure = !scene.structure,
            _ => {}
        }
        apply(hwnd, app, scene);
        app.model.selected_entity = selected;
        app.model.inspector_page = 1;
        app.model.tool_tab = 0;
    }
}
unsafe fn entity_actions(hwnd: HWND, app: &mut App) {
    unsafe {
        let Some(choice) = pick_menu(
            hwnd,
            &[
                "Enable depth / heading hold",
                "Free / manual control",
                "Toggle contact response",
                "Frame selected entity",
                "Manage / duplicate / delete",
            ],
            0,
        ) else {
            return;
        };
        if choice == 4 {
            manage(hwnd, app);
            return;
        }
        let Some(doc) = &app.model.document else {
            return;
        };
        let mut scene = doc.scene.clone();
        let mut e = app
            .model
            .selected_entity
            .and_then(|i| scene.entities.get(i))
            .cloned()
            .unwrap_or_else(|| scene.reference_entity());
        match choice {
            0 => {
                e.model.hold_depth = Some((-e.position.z).max(0.0));
                e.model.hold_heading = Some(e.heading.to_radians());
            }
            1 => {
                e.model.hold_depth = None;
                e.model.hold_heading = None;
            }
            2 => e.model.contacts = !e.model.contacts,
            3 => {
                scene.target = [e.position.x, e.position.y, e.position.z];
                scene.camera = [e.position.x - 7.0, e.position.y - 10.0, e.position.z + 5.0];
                app.model.viewport_mode = 1;
                if let Err(error) = crate::editor::edit(&mut app.model, scene, false) {
                    app.model.notice = error;
                }
                return;
            }
            _ => {}
        }
        if let Some(i) = app.model.selected_entity {
            scene.entities[i] = e;
        } else {
            scene.set_reference(&e);
        }
        apply(hwnd, app, scene);
    }
}
unsafe fn use_replay(hwnd: HWND, app: &mut App, index: usize) {
    unsafe {
        let Some(run) = app.model.run.clone() else {
            app.model.notice = "Record, load or batch a run first.".into();
            return;
        };
        let scene = serde_json::from_str(&run.scene_json)
            .map_err(|e| e.to_string())
            .and_then(|v| Scene::from_value(&v));
        match scene {
            Ok(scene) => {
                if !confirm_replace(hwnd, app) {
                    return;
                }
                pause(hwnd, app);
                crate::editor::install(&mut app.model, Document::new(scene));
                app.model.run = Some(run);
                app.model.inspector_page = 4;
                app.model.viewport_mode = 1;
                if let Err(error) = crate::editor::replay(&mut app.model, index) {
                    app.model.notice = error;
                } else {
                    app.model.notice = "Recorded truth playback. Live simulation is paused.".into();
                }
            }
            Err(error) => app.model.notice = error,
        }
    }
}
unsafe fn run_actions(hwnd: HWND, app: &mut App) {
    unsafe {
        let Some(choice) = pick_menu(
            hwnd,
            &[
                "Save loaded recording",
                "Open recording",
                "Export dataset bundle",
                "Run headless batch",
                "Cancel background task",
                "Replay loaded recording",
                "Verify re-simulation",
                "Return to live scene",
                "Export mounted camera",
                "Export camera dataset (max 120 images)",
                "Retry failed backups",
            ],
            0,
        ) else {
            return;
        };
        if choice == 10 {
            let failed = std::mem::take(&mut app.model.failed_runs);
            for run in failed {
                crate::editor::keep_run(&mut app.model, run);
            }
            SetTimer(hwnd, 6, 100, None);
            app.model.notice = "Retrying recording recovery saves.".into();
            return;
        }
        if !app.model.failed_runs.is_empty() || app.model.pending_saves.len() >= 4 {
            app.model.notice =
                "Finish or retry recording backups before starting another experiment.".into();
            return;
        }
        if choice == 4 {
            if let Some(job) = &app.job {
                job.cancel.store(true, Ordering::Relaxed);
                app.model.job_progress = "Cancelling...".into();
            }
            return;
        }
        if choice == 5 {
            use_replay(hwnd, app, 0);
            return;
        }
        if choice == 7 {
            app.model.playback = None;
            if let Some(session) = app.model.session.as_mut() {
                session.display = None;
                if let Some(doc) = &app.model.document {
                    session.set_camera(&doc.scene);
                }
            }
            app.model.viewport_mode = 1;
            return;
        }
        if choice == 8 {
            export_camera(hwnd, app);
            return;
        }
        if app.job.is_some() {
            app.model.notice = "A task is running; cancel it or wait.".into();
            return;
        }
        if choice == 1 {
            if let Some(path) = pick_file(
                hwnd,
                false,
                "Open recording",
                "Pelagic runs\0*.pelagic-run\0",
                "pelagic-run",
            ) {
                start_job(
                    hwnd,
                    app,
                    Job::start(move |_, _| {
                        let run = pelagic::experiment::Recording::read(
                            std::fs::File::open(path).map_err(|e| e.to_string())?,
                        )?;
                        let scene =
                            serde_json::from_str(&run.scene_json).map_err(|e| e.to_string())?;
                        Scene::from_value(&scene)?;
                        Ok(Output::Run(Box::new(run)))
                    }),
                );
            }
            return;
        }
        if choice == 3 {
            let Some(doc) = &app.model.document else {
                return;
            };
            let scene = doc.scene.clone();
            let seconds = app.model.batch_seconds.max(1.0);
            start_job(
                hwnd,
                app,
                Job::start(move |cancel, send| {
                    let run =
                        pelagic::experiment::batch(&scene, seconds, &cancel, |tick, total| {
                            let _ =
                                send.send(Event::Progress(format!("Batch {tick}/{total} ticks")));
                        })?;
                    Ok(Output::Run(Box::new(run)))
                }),
            );
            return;
        }
        crate::editor::archive_run(&mut app.model);
        let Some(run) = app.model.run.clone() else {
            app.model.notice = "No recording available.".into();
            return;
        };
        match choice {
            0 => {
                if let Some(path) = pick_file(
                    hwnd,
                    true,
                    "Save recording (new filename)",
                    "Pelagic runs\0*.pelagic-run\0",
                    "pelagic-run",
                ) {
                    start_job(
                        hwnd,
                        app,
                        Job::start(move |_, _| {
                            pelagic::experiment::write_atomic_with_replace(
                                std::path::Path::new(&path),
                                &run,
                                true,
                            )?;
                            Ok(Output::Notice(format!("Recording saved: {path}")))
                        }),
                    );
                }
            }
            2 | 9 => {
                if let Some(parent) = pick_folder(hwnd) {
                    let folder = std::path::Path::new(&parent).join(format!(
                        "Pelagic-dataset-{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                    ));
                    start_job(
                        hwnd,
                        app,
                        Job::start(move |cancel, send| {
                            if choice == 9 {
                                pelagic::dataset::camera(
                                    &folder,
                                    &run,
                                    &cancel,
                                    |current, total| {
                                        let _ = send.send(Event::Progress(format!(
                                            "Camera {current}/{total}"
                                        )));
                                    },
                                )?;
                            } else {
                                pelagic::experiment::export_bundle(&folder, &run)?;
                            }
                            Ok(Output::Notice(format!(
                                "Dataset exported: {}",
                                folder.display()
                            )))
                        }),
                    );
                }
            }
            6 => {
                start_job(
                    hwnd,
                    app,
                    Job::start(move |cancel, _| {
                        pelagic::experiment::verify(&run, &cancel)?;
                        Ok(Output::Notice(
                            "Re-simulation matches every recorded truth sample on this build."
                                .into(),
                        ))
                    }),
                );
            }
            _ => {}
        }
    }
}
unsafe fn export_camera(hwnd: HWND, app: &mut App) {
    unsafe {
        let Some(path) = pick_file(
            hwnd,
            true,
            "Export mounted camera",
            "PNG image\0*.png\0",
            "png",
        ) else {
            return;
        };
        let result = crate::editor::export_camera(&app.model, std::path::Path::new(&path));
        app.model.notice = match result {
            Ok(()) => format!("Camera image and calibration saved: {path}"),
            Err(e) => e,
        };
    }
}
pub(super) unsafe fn request_gpu_preview(hwnd: HWND, app: &mut App) {
    unsafe {
        if app.job.is_some() {
            app.model.notice =
                "Finish or cancel the current task before initializing the GPU.".into();
            return;
        }
        let Some(session) = app.model.session.as_ref() else {
            app.model.notice = "Open a project before choosing a viewport backend.".into();
            return;
        };
        let width = session.frame.width as u32;
        let height = session.frame.height as u32;
        app.model.backend_generation = app.model.backend_generation.wrapping_add(1);
        let request = app.model.backend_generation;
        start_job(
            hwnd,
            app,
            Job::start(move |cancel, _| {
                let renderer = pelagic_render_gpu::GpuRenderer::new(width, height)?;
                if cancel.load(Ordering::Relaxed) {
                    return Err("GPU initialization cancelled.".into());
                }
                Ok(Output::Gpu(Box::new(renderer), request))
            }),
        );
        app.model.job_progress = "Preparing native GPU preview...".into();
    }
}
pub(super) unsafe fn studio_action(hwnd: HWND, app: &mut App, id: usize) {
    unsafe {
        if (430..=446).contains(&id) {
            if let Err(error) = crate::editor::commit(&mut app.model) {
                app.model.notice = error;
                return;
            }
            let actor = app
                .painter
                .viewport_actors
                .iter()
                .find(|a| a.0 == id)
                .map(|a| a.1);
            let height = app.painter.editor_layout.map_or(200., |l| l.outliner[3]);
            crate::viewport_edit::select(&mut app.model, actor, height);
            return;
        }
        if matches!(id, 285..=289 | 295) {
            if let Err(error) = crate::editor::commit(&mut app.model) {
                app.model.notice = error;
                return;
            }
            use crate::viewport_edit::{Snap, Tool};
            match id {
                285 => {
                    if let Some(tool) = pick_menu(
                        hwnd,
                        &[
                            "Move · 1",
                            "Rotate heading / Z · 2",
                            "Uniform body size · 3",
                        ],
                        match app.model.transform_tool {
                            Tool::Move => 0,
                            Tool::Rotate => 1,
                            Tool::Size => 2,
                        },
                    ) {
                        action(hwnd, app, [295, 286, 287][tool]);
                    }
                }
                286 => {
                    app.model.focus = None;
                    app.model.transform_tool = Tool::Rotate;
                    app.model.tool_tab = 0;
                }
                287 => {
                    app.model.focus = None;
                    app.model.transform_tool = Tool::Size;
                    if app.model.inspector_page == 1 {
                        app.model.tool_tab = 3;
                    }
                }
                288 => {
                    if let Some(snap) = pick_menu(
                        hwnd,
                        &[
                            "Off",
                            "Fine: 0.1 m / 5 degrees / 5%",
                            "Medium: 0.5 m / 15 degrees / 10%",
                            "Coarse: 1 m / 45 degrees / 25%",
                        ],
                        match app.model.transform_snap {
                            Snap::Off => 0,
                            Snap::Fine => 1,
                            Snap::Normal => 2,
                            Snap::Coarse => 3,
                        },
                    ) {
                        app.model.transform_snap =
                            [Snap::Off, Snap::Fine, Snap::Normal, Snap::Coarse][snap];
                    }
                }
                289 => {
                    if let Err(error) = frame_actor(app) {
                        app.model.notice = error;
                    }
                }
                295 => {
                    app.model.focus = None;
                    app.model.transform_tool = Tool::Move;
                    app.model.tool_tab = 0;
                }
                _ => {}
            }
            if !crate::viewport_edit::can_edit(&app.model) && id != 289 {
                app.model.notice = crate::viewport_edit::unavailable_reason(&app.model).into();
            } else if crate::viewport_edit::Actor::selected(&app.model)
                == Some(crate::viewport_edit::Actor::Structure)
                && app.model.transform_tool == Tool::Rotate
            {
                app.model.notice="Heading rotation is available for vehicle actors. Use Move or Size for harbour structures.".into();
            }
            return;
        }
        if crate::viewport_edit::Handle::from_id(id).is_some() {
            app.model.focus = Some(id);
            return;
        }
        // Menu entries call the same authoring commands as keyboard shortcuts.
        if matches!(id, 236..=240 | 262 | 264 | 265) {
            let (labels, commands): (&[&str], &[usize]) = match id {
                236 => (
                    &[
                        "New project...",
                        "Open project...",
                        "Save",
                        "Save as...",
                        "Project home",
                    ],
                    &[2, 3, 16, 19, 10],
                ),
                237 => (
                    &["Undo", "Redo", "Add / duplicate / delete actor..."],
                    &[17, 18, 94],
                ),
                238 => (
                    &[
                        "Content Browser",
                        "Output Log",
                        "Ocean details",
                        "Marine life details",
                        "Sensor details",
                        "Experiments",
                        "Atlas / data",
                        "Toggle light / dark theme",
                        "Layout...",
                    ],
                    &[241, 242, 14, 90, 91, 92, 93, 7, 281],
                ),
                239 => (
                    &[
                        "Import OBJ mesh...",
                        "Import heightmap...",
                        "Experiments",
                        "Renderer...",
                    ],
                    &[243, 67, 92, 235],
                ),
                262 => (
                    &["Validate current scene", "Run / export experiment..."],
                    &[266, 101],
                ),
                264 => (
                    &[
                        "Ocean environment",
                        "Seabed",
                        "Select entity...",
                        "Marine life",
                        "Mounted sensors",
                    ],
                    &[14, 255, 121, 90, 91],
                ),
                265 => (
                    &["Add / manage actors...", "Selected actor actions..."],
                    &[94, 124],
                ),
                _ => (
                    &["Keyboard shortcuts", "About this development build"],
                    &[267, 268],
                ),
            };
            if let Some(choice) = pick_menu(hwnd, labels, 0) {
                action(hwnd, app, commands[choice]);
            }
            return;
        }
        if id == 281 || crate::layout::Splitter::from_id(id).is_some() {
            if let Some(choice) = pick_menu(
                hwnd,
                &[
                    "Maximize / restore viewport (F11)",
                    "Save workspace layout",
                    "Reset workspace layout",
                    "Narrower sidebar",
                    "Wider sidebar",
                    "Shorter Outliner",
                    "Taller Outliner",
                    "Shorter Content Browser",
                    "Taller Content Browser",
                ],
                0,
            ) {
                action(
                    hwnd,
                    app,
                    [269, 274, 273, 275, 276, 277, 278, 279, 280][choice],
                );
            }
            return;
        }
        if matches!(id, 269 | 273..=280) {
            if let Err(error) = crate::editor::commit(&mut app.model) {
                app.model.notice = error;
                return;
            }
            let result = match id {
                269 => {
                    app.model.viewport_maximized = !app.model.viewport_maximized;
                    app.model.focus = None;
                    Ok(())
                }
                273 => crate::editor::reset_layout(&mut app.model),
                274 => crate::editor::save_preferences(&app.model),
                _ => {
                    app.model.viewport_maximized = false;
                    let splitter = match id {
                        275 | 276 => crate::layout::Splitter::Sidebar,
                        277 | 278 => crate::layout::Splitter::Outliner,
                        _ => crate::layout::Splitter::Content,
                    };
                    let dimensions = workspace_dimensions(hwnd, app);
                    let layout = crate::view::EditorLayout::new(
                        dimensions[0],
                        dimensions[1],
                        app.model.layout,
                        false,
                        false,
                    );
                    let delta = if id.is_multiple_of(2) { 20.0 } else { -20.0 };
                    crate::editor::resize_panel(
                        &mut app.model,
                        dimensions,
                        splitter,
                        layout.range(splitter).0 + delta,
                    )
                }
            };
            if let Err(error) = result {
                app.model.notice = format!("Workspace preferences could not be saved: {error}");
            } else if id == 273 {
                app.model.notice = "Workspace layout reset.".into();
            } else if id == 274 {
                app.model.notice = "Workspace layout saved.".into();
            }
            return;
        }
        if matches!(id, 417..=422 | 448..=511) {
            if app.model.editing.is_some()
                && let Err(error) = crate::editor::commit(&mut app.model)
            {
                app.model.notice = error;
                return;
            }
            library_action(hwnd, app, id);
            return;
        }
        let aliased = match id {
            270 => 14,
            272 => 16,
            346 => 90,
            347 => 91,
            350 => 94,
            371 => 243,
            377 => 249,
            383 => 255,
            400 => 300,
            401..=416 => id - 100,
            _ => id,
        };
        if aliased != id {
            action(hwnd, app, aliased);
            return;
        }
        if matches!(id, 241..=250 | 251..=253 | 255 | 260 | 261 | 266..=268 | 300..=316) {
            if app.model.editing.is_some()
                && let Err(error) = crate::editor::commit(&mut app.model)
            {
                app.model.notice = error;
                return;
            }
            match id {
                241 => {
                    app.model.viewport_maximized = false;
                    app.model.content_tab = 0;
                    app.model.content_collapsed = false;
                }
                242 => {
                    app.model.viewport_maximized = false;
                    app.model.content_tab = 1;
                    app.model.content_collapsed = false;
                }
                243 => import_asset(hwnd, app),
                244 => {
                    if let Some(view) = pick_menu(
                        hwnd,
                        &[
                            "Surface / perspective",
                            "Underwater / perspective",
                            "Section / 2D",
                            "Recorded results",
                            "Mounted sonar",
                            "Global atlas",
                            "Mounted camera",
                        ],
                        app.model.viewport_mode as usize,
                    ) {
                        if view < 3 {
                            action(hwnd, app, [47, 48, 49][view]);
                        } else {
                            app.model.viewport_mode = view as u8;
                        }
                    }
                }
                245 => {
                    app.model.viewport_maximized = false;
                    app.model.content_tab = 1;
                    app.model.content_collapsed = false;
                }
                246 => {
                    if app.model.viewport_maximized {
                        app.model.viewport_maximized = false;
                        app.model.content_collapsed = false;
                    } else {
                        app.model.content_collapsed = !app.model.content_collapsed;
                    }
                }
                247 => app.model.outliner_scroll = app.model.outliner_scroll.saturating_sub(1),
                248 => app.model.outliner_scroll = (app.model.outliner_scroll + 1).min(20),
                249 => {
                    app.model.inspector_page = 0;
                    app.model.inspect_ocean = false;
                    app.model.inspect_structure = true;
                }
                250..=253 => {
                    app.model.content_filter = id - 250;
                    app.model.content_page = 0;
                }
                255 => {
                    app.model.inspector_page = 0;
                    app.model.inspect_ocean = true;
                    app.model.inspect_structure = false;
                    app.model.environment_tab = 3;
                }
                260 => app.model.content_page = app.model.content_page.saturating_sub(1),
                261 => app.model.content_page = (app.model.content_page + 1).min(20),
                266 => {
                    app.model.notice = match app
                        .model
                        .document
                        .as_ref()
                        .ok_or_else(|| "No scene open.".to_string())
                        .and_then(|d| d.scene.validate())
                    {
                        Ok(()) => {
                            "Scene validation passed. Units, bounds and embedded assets are valid."
                                .into()
                        }
                        Err(e) => format!("Scene validation failed: {e}"),
                    };
                    app.model.content_tab = 1;
                    app.model.content_collapsed = false;
                }
                267 | 268 => {
                    let text = if id == 267 {
                        "Ctrl+S: Save\nCtrl+Shift+S: Save as\nCtrl+O: Open project\nCtrl+Z / Ctrl+Y: Undo / Redo\nCtrl+Space: Content Drawer\nRMB drag: Orbit (3D viewport)\nWheel: Zoom (3D viewport), scroll (Outliner)\nWASD / QE: Move camera\nF: Frame scene\nTab / Enter: Focus / activate\nEnter: Commit property or actor search"
                    } else {
                        "Pelagic — native Rust ocean studio\nDevelopment build\nCPU reference and experimental GPU rendering\n\nThis editor layout is being rebuilt. Dock dragging, transform gizmos, visual scripting and game export remain under development."
                    };
                    MessageBoxW(
                        hwnd,
                        wide(text).as_ptr(),
                        wide("Pelagic Studio").as_ptr(),
                        MB_OK,
                    );
                }
                300..=316 => {
                    let selected = if id == 300 { None } else { Some(id - 301) };
                    if selected.is_none()
                        || app
                            .model
                            .document
                            .as_ref()
                            .is_some_and(|d| selected.unwrap() < d.scene.entities.len())
                    {
                        app.model.selected_entity = selected;
                        app.model.inspector_page = 1;
                        app.model.tool_tab = 0;
                        app.model.inspect_structure = false;
                        app.model.inspect_ocean = false;
                    }
                }
                _ => {}
            }
            if matches!(id, 241 | 242 | 245 | 246)
                && let Err(error) = crate::editor::save_preferences(&app.model)
            {
                app.model.notice = format!("Workspace preferences could not be saved: {error}");
            }
            return;
        }
        if id == 235 {
            pause(hwnd, app);
            if app.model.editing.is_some()
                && let Err(error) = crate::editor::commit(&mut app.model)
            {
                app.model.notice = error;
                return;
            }
            let Some(choice) = pick_menu(
                hwnd,
                &[
                    "CPU reference",
                    "CPU shared geometry (preview)",
                    "Native GPU (experimental preview)",
                ],
                0,
            ) else {
                return;
            };
            if choice < 2 {
                app.model.backend_generation = app.model.backend_generation.wrapping_add(1);
                if let Some(session) = app.model.session.as_mut() {
                    session.select_cpu(choice == 1);
                }
                app.model.notice = "CPU rendering selected. Simulation state is unchanged.".into();
            } else {
                request_gpu_preview(hwnd, app);
            }
            return;
        }
        if app.model.editing.as_ref().is_some_and(|e| e.0 != id)
            && let Err(error) = crate::editor::commit(&mut app.model)
        {
            app.model.notice = error;
            return;
        }
        if crate::editor::is_field(id) {
            if id != 254 {
                pause(hwnd, app);
            }
            begin_edit(app, id);
            return;
        }
        match id {
            90 => {
                app.model.inspector_page = 2;
                app.model.tool_tab = 0;
            }
            91 => {
                app.model.inspector_page = 3;
                app.model.tool_tab = 0;
            }
            92 => app.model.inspector_page = 4,
            93 => {
                app.model.inspector_page = 5;
                app.model.viewport_mode = 5;
            }
            94 => manage(hwnd, app),
            96 => {
                if let Some(view) = pick_menu(
                    hwnd,
                    &[
                        "Results / recorded traces",
                        "Mounted sonar",
                        "Mounted camera",
                        "Global overview",
                    ],
                    0,
                ) {
                    app.model.viewport_mode = [3, 4, 6, 5][view];
                    app.model.inspector_page = [4, 3, 3, 5][view];
                }
            }
            100 => {
                pause(hwnd, app);
                if app
                    .model
                    .session
                    .as_ref()
                    .is_some_and(|s| s.capture.is_some())
                {
                    crate::editor::archive_run(&mut app.model);
                    app.model.notice =
                        "Recording stopped. A recovery copy is being saved locally.".into();
                } else if let (Some(session), Some(doc)) =
                    (&mut app.model.session, &app.model.document)
                {
                    session.display = None;
                    app.model.playback = None;
                    session.start_recording(&doc.scene);
                    session.running = true;
                    app.last_tick = Instant::now();
                    SetTimer(hwnd, 2, 33, None);
                    app.model.notice =
                        "Recording truth and observations at the configured sensor rate.".into();
                }
            }
            101 => run_actions(hwnd, app),
            102 | 103 => {
                pause(hwnd, app);
                let length = app.model.run.as_ref().map_or(0, |r| r.frames.len());
                if length > 0 {
                    let i = app.model.playback.unwrap_or(0);
                    let next = if id == 102 {
                        i.saturating_sub(1)
                    } else {
                        (i + 1).min(length - 1)
                    };
                    if let Err(error) = crate::editor::replay(&mut app.model, next) {
                        app.model.notice = error;
                    }
                }
            }
            121 => entity_menu(hwnd, app),
            122 => {
                if let Some(tab) = pick_menu(hwnd, &crate::library::ENTITY_TABS, app.model.tool_tab)
                {
                    app.model.tool_tab = tab;
                }
            }
            124 => entity_actions(hwnd, app),
            145 => {
                if confirm_replace(hwnd, app) {
                    pause(hwnd, app);
                    match crate::jobs::goa() {
                        Ok(scene) => {
                            crate::editor::install(&mut app.model, Document::new(scene));
                            app.model.inspector_page = 5;
                            app.model.notice="NOAA ETOPO1 regional data; waves, profiles and life remain synthetic.".into();
                        }
                        Err(error) => app.model.notice = error,
                    }
                }
            }
            146 => {
                let Some(doc) = &app.model.document else {
                    return;
                };
                let mut scene = doc.scene.clone();
                scene.grid = None;
                scene.terrain = 1;
                scene.provenance.measured = false;
                scene.provenance.classification = "synthetic".into();
                scene.provenance.source = "Synthetic local coastal region".into();
                scene.provenance.source_url.clear();
                scene.provenance.resolution =
                    "4 m synthetic terrain; no location-specific measurements".into();
                scene.camera = Scene::default().camera;
                scene.target = Scene::default().target;
                scene.seed = scene.provenance.origin_latitude.to_bits()
                    ^ scene.provenance.origin_longitude.to_bits();
                apply(hwnd, app, scene);
                app.model.viewport_mode = 0;
            }
            147 => {
                if app.job.is_some() {
                    app.model.notice = "Wait for the current job.".into();
                    return;
                }
                if let Some(path) = pick_file(
                    hwnd,
                    false,
                    "Import complete XYZ/time current grid",
                    "Current CSV\0*.csv\0",
                    "csv",
                ) && let Some(doc) = &app.model.document
                {
                    let before = doc.scene.clone();
                    start_job(
                        hwnd,
                        app,
                        Job::start(move |_, _| {
                            use std::io::Read;
                            let mut text = String::new();
                            std::fs::File::open(path)
                                .map_err(|e| e.to_string())?
                                .take(8_388_609)
                                .read_to_string(&mut text)
                                .map_err(|e| e.to_string())?;
                            let field =
                                pelagic::data::current_csv(&text).map_err(str::to_string)?;
                            let mut after = before.clone();
                            after.flow_grid = Some(Arc::new(field));
                            after.validate()?;
                            Ok(Output::Scene {
                                before: Box::new(before),
                                after: Box::new(after),
                            })
                        }),
                    );
                }
            }
            148 => {
                if app.job.is_some() {
                    app.model.notice = "Wait for the current task.".into();
                    return;
                }
                if let Some(path) = pick_file(
                    hwnd,
                    false,
                    "Import latitude,longitude,altitude CSV",
                    "Geographic CSV\0*.csv\0",
                    "csv",
                ) && let Some(doc) = &app.model.document
                {
                    let before = doc.scene.clone();
                    start_job(
                        hwnd,
                        app,
                        Job::start(move |_, _| {
                            use std::io::Read;
                            let mut text = String::new();
                            std::fs::File::open(&path)
                                .map_err(|e| e.to_string())?
                                .take(8_388_609)
                                .read_to_string(&mut text)
                                .map_err(|e| e.to_string())?;
                            if text.len() > 8_388_608 {
                                return Err("Geographic input exceeds 8 MB.".into());
                            }
                            let (grid, lat, lon) =
                                pelagic::data::geographic_csv(&text).map_err(str::to_string)?;
                            let extent = grid.spacing * (grid.rows.max(grid.columns) - 1) as f64;
                            let depth = (-grid.elevations.iter().sum::<f64>()
                                / grid.elevations.len() as f64)
                                .clamp(5.0, 11000.0);
                            let mut after = before.clone();
                            after.bottom_depth = depth;
                            after.vehicle_depth = depth.min(30.0) * 0.7;
                            after.terrain = 3;
                            after.structure = false;
                            after.camera = [-extent * 0.35, -extent * 0.45, extent * 0.3];
                            after.target = [0.0, 0.0, 0.0];
                            after.provenance = pelagic::data::Provenance {
                                source: std::path::Path::new(&path)
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("User geographic grid")
                                    .into(),
                                source_url: String::new(),
                                licence: "User supplied; source permissions unverified".into(),
                                retrieved_utc: String::new(),
                                resolution: format!("Resampled local grid / {:.1} m", grid.spacing),
                                measured: false,
                                classification: "resampled user geographic grid".into(),
                                origin_latitude: lat,
                                origin_longitude: lon,
                            };
                            after.grid = Some(Arc::new(grid));
                            after.validate()?;
                            Ok(Output::Scene {
                                before: Box::new(before),
                                after: Box::new(after),
                            })
                        }),
                    );
                }
            }
            149 => {
                if let Some(doc) = &app.model.document {
                    let mut scene = doc.scene.clone();
                    scene.flow_grid = None;
                    apply(hwnd, app, scene);
                }
            }
            171 => {
                if let Some(preset) = pick_menu(
                    hwnd,
                    &[
                        "Reference school / 48 fish",
                        "Dense school / 128 fish",
                        "No fish",
                        "Move school near selected entity",
                        "Toggle 64 tracers",
                        "Population settings",
                        "Behaviour settings",
                    ],
                    0,
                ) && let Some(doc) = &app.model.document
                {
                    if preset >= 5 {
                        app.model.tool_tab = preset - 5;
                        return;
                    }
                    let mut scene = doc.scene.clone();
                    match preset {
                        0 => scene.school.count = 48,
                        1 => scene.school.count = 128,
                        2 => scene.school.count = 0,
                        3 => {
                            scene.school.center = app
                                .model
                                .selected_entity
                                .and_then(|i| scene.entities.get(i))
                                .map_or(
                                    pelagic::math::Vec3::new(0.0, 0.0, -scene.vehicle_depth),
                                    |e| e.position,
                                )
                                + pelagic::math::Vec3::new(5.0, 2.0, 0.0);
                        }
                        4 => scene.particles = if scene.particles == 0 { 64 } else { 0 },
                        _ => {}
                    }
                    apply(hwnd, app, scene);
                }
            }
            172 => {
                if let Some(choice) = pick_menu(
                    hwnd,
                    &[
                        "Sonar view",
                        "Camera view",
                        "Export camera PNG + calibration",
                        "Attach sensors to entity",
                    ],
                    0,
                ) {
                    if choice == 2 {
                        export_camera(hwnd, app);
                    } else if choice == 3 {
                        if let Some(doc) = &app.model.document {
                            let mut scene = doc.scene.clone();
                            let labels: Vec<_> = std::iter::once("Reference / primary".to_string())
                                .chain(scene.entities.iter().map(|e| e.name.clone()))
                                .collect();
                            let refs: Vec<_> = labels.iter().map(String::as_str).collect();
                            if let Some(index) = pick_menu(hwnd, &refs, 0) {
                                scene.sensors.entity = if index == 0 {
                                    0
                                } else {
                                    scene.entities[index - 1].id
                                };
                                apply(hwnd, app, scene);
                            }
                        }
                    } else {
                        app.model.viewport_mode = if choice == 0 { 4 } else { 6 };
                    }
                }
            }
            173 => {
                if let Some(tab) = pick_menu(
                    hwnd,
                    &[
                        "Sonar",
                        "Navigation / timing",
                        "Camera / mount",
                        "Motion noise",
                    ],
                    app.model.tool_tab,
                ) {
                    app.model.tool_tab = tab;
                }
            }
            _ => {}
        }
    }
}
unsafe fn pick_file(
    hwnd: HWND,
    save: bool,
    title: &str,
    filter: &str,
    extension: &str,
) -> Option<String> {
    unsafe {
        let mut path = vec![0u16; 32768];
        let filter = wide(filter);
        let title = wide(title);
        let ext = wide(extension);
        let mut dialog: OPENFILENAMEW = zeroed();
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = hwnd;
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
        let ok = if save {
            GetSaveFileNameW(&mut dialog)
        } else {
            GetOpenFileNameW(&mut dialog)
        };
        if ok == 0 {
            None
        } else {
            let n = path.iter().position(|c| *c == 0).unwrap_or(path.len());
            Some(String::from_utf16_lossy(&path[..n]))
        }
    }
}
