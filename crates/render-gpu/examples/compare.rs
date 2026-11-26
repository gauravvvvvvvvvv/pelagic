//! Hardware qualification and same-snapshot CPU/GPU preview. No desktop capture.
use pelagic::{
    math::Vec3,
    render::SoftwareRenderer,
    render_scene::RenderScene,
    scene::{Scene, world},
};
use pelagic_render_gpu::{GpuRenderer, View};
fn save(
    folder: &std::path::Path,
    name: &str,
    frame: &pelagic::render::Frame,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(folder.join(name))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        frame.width as u32,
        frame.height as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&frame.rgb)?;
    Ok(())
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let folder = std::path::Path::new(args.get(1).ok_or("provide a new output directory")?);
    std::fs::create_dir(folder)?;
    let t = std::time::Instant::now();
    let mut gpu = GpuRenderer::new(960, 600)?;
    let initialization = t.elapsed().as_secs_f64() * 1000.;
    let mut reports = vec![];
    for under in [false, true] {
        let mut scene = Scene::robotics_demo();
        if under {
            scene.camera = [-4.5, -6., -9.6];
            scene.target = [0., 0., -12.];
        } else {
            scene.camera = [-12., -18., 5.];
            scene.target = [2., 2., -0.5];
        }
        let w = world(&scene)?;
        let before = w.checkpoint();
        let snapshot = w.snapshot();
        let t = std::time::Instant::now();
        let geometry = RenderScene::build(&w.ocean, &snapshot, 1, 0)?;
        let build_ms = t.elapsed().as_secs_f64() * 1000.;
        let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
        let view = View {
            eye: v(scene.camera),
            target: v(scene.target),
            up: Vec3::new(0., 0., 1.),
            fov_y: 58.497,
            sun_elevation: scene.sun_elevation.to_radians(),
            sun_azimuth: pelagic_render_gpu::LEGACY_SUN_AZIMUTH,
            clouds: 0.0,
            time: snapshot.time,
            attenuation: scene.attenuation,
            underwater: under,
        };
        let t = std::time::Instant::now();
        let gpu_frame = gpu.render(&geometry, view)?;
        let first_ms = t.elapsed().as_secs_f64() * 1000.;
        let mut cpu = SoftwareRenderer::new(960, 600)?;
        cpu.camera = view.eye;
        cpu.target = view.target;
        cpu.sun_elevation = view.sun_elevation;
        let t = std::time::Instant::now();
        let cpu_frame = cpu.render_shared(&w.ocean, &snapshot, &geometry);
        let cpu_ms = t.elapsed().as_secs_f64() * 1000.;
        let mut times = vec![];
        for _ in 0..10 {
            let t = std::time::Instant::now();
            let repeat = gpu.render(&geometry, view)?;
            assert_eq!(repeat.rgb, gpu_frame.rgb);
            times.push(t.elapsed().as_secs_f64() * 1000.);
        }
        assert_eq!(
            w.checkpoint(),
            before,
            "rendering must not modify physical truth"
        );
        let variation = gpu_frame.rgb.iter().max().unwrap() - gpu_frame.rgb.iter().min().unwrap();
        assert!(variation > 40, "blank GPU frame");
        let name = if under { "underwater" } else { "surface" };
        save(folder, &format!("{name}-gpu.png"), &gpu_frame)?;
        save(folder, &format!("{name}-cpu.png"), &cpu_frame)?;
        times.sort_by(f64::total_cmp);
        reports.push(serde_json::json!({"view":name,"dimensions":[960,600],"opaque_triangles":geometry.opaque.len()/3,"water_triangles":geometry.water.len()/3,"geometry_ms":build_ms,"gpu_first_ms":first_ms,"gpu_readback_mean_ms":times.iter().sum::<f64>()/10.,"gpu_readback_p95_ms":times[9],"cpu_shared_ms":cpu_ms,"repeated_frames_exact":true,"world_unchanged":true}));
        // Non-aligned rows and resize exercise staging-buffer stride/lifetime.
        gpu.resize(513, 287)?;
        let odd = gpu.render(&geometry, view)?;
        assert_eq!(odd.rgb.len(), 513 * 287 * 3);
        gpu.resize(960, 600)?;
    }
    let report = serde_json::json!({"engine_build":pelagic::BUILD_ID,"renderer_build":format!("{:016x}",pelagic_render_gpu::RENDERER_BUILD_ID),"adapter":format!("{:?}",gpu.adapter),"initialization_ms":initialization,"measurements":reports,"stage":"experimental offscreen GPU preview, includes readback; CPU approximate materials; no shadow maps, imported textures or GPU sensor export yet"});
    std::fs::write(
        folder.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
