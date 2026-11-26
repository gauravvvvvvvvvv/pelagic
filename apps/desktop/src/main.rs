#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod accessibility;
#[cfg(all(windows, test))]
mod accessibility_tests;
mod ais;
mod backups;
mod cli;
mod editor;
mod forecast;
mod jobs;
mod layout;
mod library;
mod project;
mod session;
mod sound;
mod studio;
mod view;
mod viewport_edit;
#[cfg(windows)]
mod windows;

fn main() {
    // Reproducible artwork previews use the same renderer as the actual window.
    let args: Vec<String> = std::env::args().collect();
    if let Some(path) = args.iter().find_map(|a| a.strip_prefix("--test-storage="))
        && let Err(error) = project::configure_storage(std::path::Path::new(path))
    {
        eprintln!("Pelagic storage: {error}");
        std::process::exit(1);
    }
    if args.get(1).is_some_and(|a| a == "--campaign") {
        std::process::exit(cli::campaign(&args));
    }
    if args.get(1).is_some_and(|a| a == "--forecast") {
        std::process::exit(cli::sea_forecast(&args));
    }
    if args.get(1).is_some_and(|a| a == "--seakeeping") {
        std::process::exit(cli::seakeeping(&args));
    }
    if args.get(1).is_some_and(|a| a == "--spill") {
        std::process::exit(cli::spill(&args));
    }
    if args.get(1).is_some_and(|a| a == "--inspect") {
        std::process::exit(cli::inspect(&args));
    }
    if args.get(1).is_some_and(|a| a == "--colregs") {
        std::process::exit(cli::colregs(&args));
    }
    if args.get(1).is_some_and(|a| a == "--survey") {
        std::process::exit(cli::survey(&args));
    }
    if args.get(1).is_some_and(|a| a == "--sar") {
        std::process::exit(cli::sar(&args));
    }
    if args.get(1).is_some_and(|a| a == "--trials") {
        std::process::exit(cli::trials(&args));
    }
    if args.get(1).is_some_and(|a| a == "--rerecord") {
        std::process::exit(cli::rerecord(&args));
    }
    if args.get(1).is_some_and(|a| a == "--benchmark-ui") {
        let mut reports = vec![];
        let mut painter = view::Painter::new();
        for (width, height) in [(1100, 720), (1920, 990)] {
            for dark_mode in [false, true] {
                let scene = project::Scene::robotics_demo();
                let mut model = view::Model {
                    dark_mode,
                    workspace: true,
                    session: Some(session::Session::from_scene(&scene)),
                    document: Some(project::Document::new(scene)),
                    ..Default::default()
                };
                let mut times = vec![];
                for page in 0..6 {
                    model.inspector_page = page;
                    for _ in 0..5 {
                        let start = std::time::Instant::now();
                        painter.paint(width, height, &model, None);
                        times.push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                times.sort_by(f64::total_cmp);
                reports.push(serde_json::json!({"dimensions":[width,height],"dark":dark_mode,"mean_paint_ms":times.iter().sum::<f64>()/times.len() as f64,"p95_paint_ms":times[28],"max_paint_ms":times[29]}));
            }
        }
        let path = args.get(2).expect("new report path");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("new report");
        serde_json::to_writer_pretty(file, &reports).expect("write UI benchmarks");
        return;
    }
    if args.get(1).is_some_and(|a| a == "--benchmark-frame") {
        let mut painter = view::Painter::new();
        let mut reports = vec![];
        let mut learning = project::Scene::default();
        learning.school.count = 0;
        learning.particles = 0;
        for (name, scene) in [
            ("learning", learning),
            ("vehicle", project::Scene::robotics_demo()),
            ("ecology", project::Scene::ecology_demo()),
        ] {
            for (width, height) in [(1100, 720), (1920, 990)] {
                let mut model = view::Model {
                    dark_mode: true,
                    workspace: true,
                    session: Some(session::Session::from_scene(&scene)),
                    document: Some(project::Document::new(scene.clone())),
                    ..Default::default()
                };
                let scale = width as f64 / 1100.0;
                let h = (512.0 * (height as f64 - 221.0 * scale) / (654.0 * scale))
                    .clamp(180.0, 900.0) as usize;
                let session = model.session.as_mut().unwrap();
                session.resize(512, h);
                session.running = true;
                let mut times = vec![];
                for _ in 0..35 {
                    let start = std::time::Instant::now();
                    model
                        .session
                        .as_mut()
                        .unwrap()
                        .advance_view(1.0 / 30.0, true);
                    painter.paint(width, height, &model, None);
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                times.drain(..5);
                times.sort_by(f64::total_cmp);
                reports.push(serde_json::json!({"name":name,"window":[width,height],"viewport":[512,h],"mean_frame_ms":times.iter().sum::<f64>()/30.0,"p95_frame_ms":times[28],"max_frame_ms":times[29],"includes":"physics, sensors, renderer, full UI paint; excludes OS blit/event scheduling"}));
            }
        }
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(args.get(2).expect("new report path"))
            .expect("new report");
        serde_json::to_writer_pretty(file, &reports).unwrap();
        return;
    }
    if args.get(1).is_some_and(|a| a == "--check-project") {
        let start = std::time::Instant::now();
        let doc =
            project::Document::open(std::path::Path::new(args.get(2).expect("project input")))
                .expect("valid project");
        let session = session::Session::from_scene(&doc.scene);
        let report = serde_json::json!({"name":doc.scene.name,"schema":doc.scene.value()["version"],"open_and_first_frame_ms":start.elapsed().as_secs_f64()*1000.0,"dimensions":[session.frame.width,session.frame.height],"valid":true});
        std::fs::write(
            args.get(3).expect("report output"),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .expect("write project check");
        return;
    }
    if args
        .get(1)
        .is_some_and(|a| a == "--export-reference-project" || a == "--refresh-reference-project")
    {
        let mut scene = project::Scene::default();
        if args.iter().any(|a| a == "--surface") {
            scene = project::Scene::surface_demo();
        } else if args.iter().any(|a| a == "--sensors") {
            scene = project::Scene::sensor_demo();
        } else if args.iter().any(|a| a == "--library") {
            scene = library::demo_scene();
        } else if args.iter().any(|a| a == "--robotics") {
            scene = project::Scene::robotics_demo();
        } else if args.iter().any(|a| a == "--ecology") {
            scene = project::Scene::ecology_demo();
        } else if args.iter().any(|a| a == "--regional") {
            scene = jobs::goa().expect("bundled regional scene");
        } else if args.iter().any(|a| a == "--underwater") {
            scene.name = "Underwater vehicle study".into();
            scene.camera = [-7.0, -10.0, -15.0];
            scene.target = [0.0, 0.0, -20.0];
        } else if args.iter().any(|a| a == "--canyon") {
            scene.name = "Canyon environment".into();
            scene.terrain = 2;
            scene.structure = false;
        } else {
            scene.name = "Coastal waves".into();
        }
        let path = std::path::Path::new(args.get(2).expect("project output path"));
        assert!(
            !path.exists()
                || (args[1] == "--refresh-reference-project"
                    && project::Scene::read(path).is_ok_and(|mut old| {
                        if args.iter().any(|a| a == "--regional")
                            && let (Some(grid), Some(expected)) = (&mut old.grid, &scene.grid)
                        {
                            let former_center = [
                                -((grid.columns - 1) as f64) * grid.spacing * 0.5,
                                -((grid.rows - 1) as f64) * grid.spacing * 0.5,
                            ];
                            if grid.origin == former_center {
                                std::sync::Arc::make_mut(grid).origin = expected.origin;
                            }
                        }
                        old == scene
                    })),
            "Reference export never overwrites a changed project"
        );
        project::Document::new(scene)
            .save(path)
            .expect("save reference project");
        return;
    }
    if args.get(1).is_some_and(|a| a == "--capture-ocean") {
        let mut scene = project::Scene::default();
        if args.iter().any(|a| a == "--calm") {
            scene.wave_height = 0.0;
        }
        if args.iter().any(|a| a == "--underwater") {
            scene.camera = [-7.0, -10.0, -15.0];
            scene.target = [0.0, 0.0, -20.0];
        }
        let mut session = session::Session::from_scene(&scene);
        session.resize(960, 600);
        let path = args.get(2).expect("PNG output path");
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(path).expect("PNG output")),
            session.frame.width as u32,
            session.frame.height as u32,
        );
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("PNG header")
            .write_image_data(&session.frame.rgb)
            .expect("PNG pixels");
        return;
    }
    if args.get(1).is_some_and(|a| {
        a == "--capture-home" || a == "--capture-workspace" || a == "--capture-new-project"
    }) {
        let path = args.get(2).expect("provide a PNG output path");
        let mut painter = view::Painter::new();
        let mut model = view::Model {
            dark_mode: args.iter().any(|arg| arg == "--dark"),
            ..Default::default()
        };
        if let Some(root) = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--test-storage="))
        {
            model.storage_root = Some(root.into());
            editor::preferences(&mut model);
            if args.iter().any(|arg| arg == "--dark") {
                model.dark_mode = true;
            }
            if args.iter().any(|arg| arg == "--light") {
                model.dark_mode = false;
            }
        }
        model.viewport_maximized = args.iter().any(|arg| arg == "--maximize-viewport");
        if args[1] == "--capture-workspace" {
            let mut scene = project::Scene {
                vehicle: !args.iter().any(|a| a == "--empty"),
                ..Default::default()
            };
            if args.iter().any(|a| a == "--underwater") {
                scene.camera = [-7.0, -10.0, -15.0];
                scene.target = [4.0, 2.0, -18.0];
                model.viewport_mode = 1;
            }
            if args.iter().any(|a| a == "--robotics") {
                scene = project::Scene::robotics_demo();
                model.viewport_mode = 1;
            }
            if args.iter().any(|a| a == "--library") {
                scene = library::demo_scene();
                model.viewport_mode = 1;
                model.selected_library = scene
                    .library
                    .meshes
                    .first()
                    .map(|m| library::Item::Mesh(m.id));
            }
            if let Some(filter) = args.iter().find_map(|a| {
                a.strip_prefix("--content=")
                    .and_then(|v| v.parse::<usize>().ok())
            }) {
                model.content_filter = filter.min(4);
            }
            model.project = Some(scene.name.clone());
            model.session = Some(session::Session::from_scene(&scene));
            model.document = Some(project::Document::new(scene));
            model.inspect_ocean = true;
            model.workspace = true;
            if args.iter().any(|a| a == "--section") {
                model.viewport_mode = 2;
            }
            if let Some(tab) = args
                .iter()
                .find_map(|a| a.strip_prefix("--tab=").and_then(|v| v.parse::<u8>().ok()))
            {
                model.environment_tab = tab.min(4) as usize;
            }
            if let Some(page) = args.iter().find_map(|a| {
                a.strip_prefix("--page=")
                    .and_then(|v| v.parse::<usize>().ok())
            }) {
                model.inspector_page = page.min(5);
            }
            if let Some(mode) = args
                .iter()
                .find_map(|a| a.strip_prefix("--mode=").and_then(|v| v.parse::<u8>().ok()))
            {
                model.viewport_mode = mode.min(6);
            }
            if let Some(tab) = args.iter().find_map(|a| {
                a.strip_prefix("--tool=")
                    .and_then(|v| v.parse::<usize>().ok())
            }) {
                model.tool_tab = tab.min(5);
            }
            if let Some(index) = args.iter().find_map(|a| {
                a.strip_prefix("--select=")
                    .and_then(|v| v.parse::<usize>().ok())
            }) && index < model.document.as_ref().unwrap().scene.entities.len()
            {
                model.selected_entity = Some(index);
                model.inspector_page = 1;
                model.inspect_ocean = false;
            }
            if args.iter().any(|a| a == "--run") {
                let scene = &model.document.as_ref().unwrap().scene;
                let run = pelagic::experiment::batch(
                    scene,
                    3.0,
                    &std::sync::atomic::AtomicBool::new(false),
                    |_, _| {},
                )
                .expect("capture run");
                model.run = Some(std::sync::Arc::new(run));
            }
            if let Some(s) = model.session.as_mut() {
                s.running = true;
                if !args.iter().any(|a| a == "--edit-scene") {
                    s.advance_view(0.2, false);
                }
                s.running = false;
                s.set_camera(&model.document.as_ref().unwrap().scene);
                s.view(
                    model.viewport_mode == 6,
                    if model.inspector_page == 1 {
                        Some(
                            model.selected_entity.map_or(0, |i| {
                                model.document.as_ref().unwrap().scene.entities[i].id
                            }),
                        )
                    } else {
                        None
                    },
                );
                if model.viewport_mode == 6 {
                    s.view(true, None);
                }
            }
        }
        if args[1] == "--capture-new-project" {
            model.new_project = true;
            model.new_name = "Arabian Sea study".into();
            model.new_folder = "C:\\Users\\Harry\\Documents\\pelagic\\Projects".into();
            model.editing = Some((20, model.new_name.clone()));
            model.input_selected = true;
        }
        if args.iter().any(|arg| arg == "--hover") {
            model.hover = Some(3);
            for _ in 0..6 {
                painter.animate_buttons(&model, 1.0 / 60.0);
            }
        }
        let (width, height) = if args.iter().any(|arg| arg == "--fullhd") {
            (1920, 990)
        } else {
            (1100, 720)
        };
        if let Some(session) = model.session.as_mut() {
            let layout = view::EditorLayout::new(
                width as f32,
                height as f32,
                model.layout,
                model.content_collapsed,
                model.viewport_maximized,
            );
            let quality = model
                .document
                .as_ref()
                .map_or(0, |d| d.scene.quality.min(2) as usize);
            let w = layout.viewport[2]
                .min([512.0, 800.0, 1120.0][quality])
                .max(320.0) as usize;
            let h =
                (w as f32 * layout.viewport[3] / layout.viewport[2]).clamp(180.0, 900.0) as usize;
            session.resize(w, h);
        }
        if let Some(tool) = args.iter().find_map(|a| a.strip_prefix("--transform=")) {
            model.transform_tool = match tool {
                "rotate" => viewport_edit::Tool::Rotate,
                "size" => viewport_edit::Tool::Size,
                _ => viewport_edit::Tool::Move,
            };
            if model.transform_tool == viewport_edit::Tool::Size && model.inspector_page == 1 {
                model.tool_tab = 3;
            }
        }
        if args.iter().any(|arg| arg == "--gpu")
            && let Some(session) = model.session.as_mut()
        {
            let gpu = pelagic_render_gpu::GpuRenderer::new(
                session.frame.width as u32,
                session.frame.height as u32,
            )
            .expect("native GPU preview");
            session.install_gpu(Box::new(gpu));
            assert!(
                session.backend_notice.is_empty(),
                "{}",
                session.backend_notice
            );
        }
        painter.paint(width, height, &model, None);
        painter.save_png(path).expect("write preview");
        return;
    }
    if !args.iter().any(|a| a == "--legacy-ui" || a == "--cpu") {
        let value = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let options = studio::Options {
            open: value("--open"),
            storage: args
                .iter()
                .find_map(|a| a.strip_prefix("--test-storage="))
                .map(Into::into),
            capture: value("--capture-studio").map(|path| {
                let frames = args
                    .iter()
                    .find_map(|a| a.strip_prefix("--frames=").and_then(|v| v.parse().ok()))
                    .unwrap_or(30);
                (path, frames)
            }),
            capture_size: args.iter().find_map(|a| {
                let (w, h) = a.strip_prefix("--size=")?.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            }),
            library_demo: args.iter().any(|a| a == "--library"),
            script: args
                .iter()
                .filter_map(|a| a.strip_prefix("--script=").map(str::to_string))
                .collect(),
            automation_handle: args
                .iter()
                .find_map(|a| a.strip_prefix("--automation-handle="))
                .map(str::to_string),
        };
        let unattended = options.capture.is_some();
        if let Err(error) = studio::run(options) {
            eprintln!("Pelagic Studio: {error}");
            if unattended {
                std::process::exit(1);
            }
            #[cfg(windows)]
            windows::run(Some(&error));
        }
        return;
    }
    #[cfg(windows)]
    windows::run(None);
    #[cfg(not(windows))]
    eprintln!("The first native desktop shell currently targets Windows.");
}
