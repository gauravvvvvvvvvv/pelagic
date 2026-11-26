//! Release-mode measurements including physical entities and sensor scheduling.
use pelagic::{
    instruments::Suite,
    render::{Renderer, SoftwareRenderer},
    scene::{Scene, world},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "scene,bodies,fish,tracers,cold_render_ms,mean_render_ms,p95_render_ms,mean_step_us,p95_step_us"
    );
    for scene in [Scene::robotics_demo(), Scene::ecology_demo()] {
        let mut w = world(&scene)?;
        let mut suite = Suite::new(scene.sensors.clone());
        let mut steps = vec![];
        for _ in 0..600 {
            let start = std::time::Instant::now();
            w.step();
            suite.update(&w, 1.0 / 60.0);
            steps.push(start.elapsed().as_secs_f64() * 1e6);
        }
        let mut renderer = SoftwareRenderer::new(512, 320)?;
        renderer.quality = 0;
        renderer.camera =
            pelagic::math::Vec3::new(scene.camera[0], scene.camera[1], scene.camera[2]);
        renderer.target =
            pelagic::math::Vec3::new(scene.target[0], scene.target[1], scene.target[2]);
        let start = std::time::Instant::now();
        renderer.render(&w.ocean, &w.snapshot());
        let cold = start.elapsed().as_secs_f64() * 1000.0;
        let mut frames = vec![];
        for _ in 0..30 {
            w.step();
            w.step();
            let start = std::time::Instant::now();
            std::hint::black_box(renderer.render(&w.ocean, &w.snapshot()));
            frames.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        frames.sort_by(f64::total_cmp);
        steps.sort_by(f64::total_cmp);
        println!(
            "{},{},{},{},{:.2},{:.2},{:.2},{:.2},{:.2}",
            scene.name,
            w.vehicles().len(),
            scene.school.count,
            scene.particles,
            cold,
            frames.iter().sum::<f64>() / frames.len() as f64,
            frames[28],
            steps.iter().sum::<f64>() / steps.len() as f64,
            steps[569]
        );
    }
    Ok(())
}
