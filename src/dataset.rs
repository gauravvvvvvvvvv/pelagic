//! Bounded camera datasets generated from recorded truth, with exact intrinsics/times.
use crate::{
    experiment::{Recording, export_bundle},
    math::Vec3,
    render::{Renderer, SoftwareRenderer},
};
use std::{
    io::Write,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
pub fn camera(
    folder: &Path,
    run: &Recording,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Camera export cancelled".into());
    }
    export_bundle(folder, run)?;
    let marker = folder.join("INCOMPLETE.txt");
    std::fs::write(&marker, "Camera export is incomplete.").map_err(|e| e.to_string())?;
    let scene = crate::scene::Scene::from_value(
        &serde_json::from_str(&run.scene_json).map_err(|e| e.to_string())?,
    )?;
    let world = crate::scene::world(&scene)?;
    let images = folder.join("camera");
    std::fs::create_dir(&images).map_err(|e| e.to_string())?;
    let mut renderer = SoftwareRenderer::new(320, 200).map_err(str::to_string)?;
    renderer.quality = 0;
    renderer.vertical_fov = scene.sensors.camera_fov;
    renderer.sun_elevation = scene.sun_elevation.to_radians();
    renderer.terrain_material = scene.terrain_material;
    let stride = run.frames.len().div_ceil(120).max(1);
    let total = run.frames.len().div_ceil(stride);
    let mut samples = vec![];
    for (index, frame) in run.frames.iter().enumerate().step_by(stride) {
        if cancel.load(Ordering::Relaxed) {
            return Err(
                "Camera export cancelled; incomplete directory retained and marked.".into(),
            );
        }
        let Some(body) = frame
            .truth
            .vehicles
            .iter()
            .find(|p| p.id == scene.sensors.entity)
            .or_else(|| frame.truth.vehicles.first())
        else {
            return Err("Camera dataset needs a mounted entity.".into());
        };
        renderer.camera = body.position + body.orientation.rotate(scene.sensors.mount);
        renderer.target = renderer.camera + body.orientation.rotate(Vec3::new(1.0, 0.0, -0.15));
        renderer.camera_up = body.orientation.rotate(Vec3::new(0.0, 0.0, 1.0));
        let image = renderer.render(&world.ocean, &frame.truth);
        let name = format!("frame-{index:06}.ppm");
        let mut writer = std::io::BufWriter::new(
            std::fs::File::create(images.join(&name)).map_err(|e| e.to_string())?,
        );
        image.write_ppm(&mut writer).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        samples.push(serde_json::json!({"file":name,"sample":index,"tick":frame.truth.tick,"time_s":frame.truth.time,"entity":body.id,"position_world_m":renderer.camera,"target_world_m":renderer.target,"orientation_body_to_world":body.orientation}));
        progress(samples.len(), total);
    }
    let focal = 200.0 / (2.0 * (scene.sensors.camera_fov.to_radians() * 0.5).tan());
    let metadata = serde_json::json!({"format":"pelagic-camera-dataset","version":1,"width":320,"height":200,"fx":focal,"fy":focal,"cx":160.0,"cy":100.0,"format_image":"P6 PPM; RGB8","source_stride":stride,"up_body":[0.0,0.0,1.0],"pixel_convention":"upper-left corner origin; pixel centres n+0.5","mount_body_m":scene.sensors.mount,"forward_body":[1.0,0.0,-0.15],"optics":"pinhole plus illustrative underwater extinction; no lens distortion","samples":samples});
    std::fs::write(
        images.join("calibration.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let manifest = folder.join("manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    value["camera"] = serde_json::json!("camera/calibration.json");
    std::fs::write(manifest, serde_json::to_vec_pretty(&value).unwrap())
        .map_err(|e| e.to_string())?;
    std::fs::remove_file(marker).map_err(|e| e.to_string())?;
    Ok(())
}
