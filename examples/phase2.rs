//! Reproducible CPU baseline: three 512 m synthetic scenes, one body, 60 Hz.
use pelagic::{
    World,
    math::Vec3,
    ocean::{Ocean, ShearedCurrent},
    physics::Vehicle,
    render::{Renderer, SoftwareRenderer},
    terrain::HeightGrid,
    waves::SeaState,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all("output/phase2")?;
    println!("scene,width,height,mean_ms,p95_ms,step_mean_us");
    for (name, kind, position, target) in [
        (
            "coast",
            1,
            Vec3::new(-95.0, -125.0, 34.0),
            Vec3::new(0.0, 15.0, 0.0),
        ),
        (
            "vehicle",
            1,
            Vec3::new(-18.0, -26.0, -22.0),
            Vec3::new(0.0, 15.0, -34.0),
        ),
        (
            "canyon",
            2,
            Vec3::new(-95.0, -125.0, 34.0),
            Vec3::new(0.0, 15.0, 0.0),
        ),
    ] {
        let ocean = Ocean {
            bottom: Box::new(HeightGrid::procedural(kind, 40.0)),
            waves: SeaState::spectrum(
                1.2,
                6.0,
                35_f64.to_radians(),
                25_f64.to_radians(),
                40.0,
                42,
            )?,
            current: Box::new(ShearedCurrent {
                surface: Vec3::new(0.3, 0.0, 0.0),
                shear: 0.008,
                tide_period: 0.0,
            }),
            ..Default::default()
        };
        let mut world = World::new(ocean, 1.0 / 60.0, 42)?;
        let mut vehicle = Vehicle::new(
            12.0,
            12.0 / 1025.0,
            Vec3::new(0.3, 0.4, 0.5),
            Vec3::new(8.0, 12.0, 15.0),
            Vec3::new(0.4, 0.4, 0.4),
        )?;
        vehicle.position = Vec3::new(0.0, 0.0, -20.0);
        world.add_vehicle(vehicle);
        let step_start = std::time::Instant::now();
        for _ in 0..600 {
            world.step();
        }
        let step_us = step_start.elapsed().as_secs_f64() * 1e6 / 600.0;
        for (quality, width, height) in [(0, 512, 320), (1, 800, 500), (2, 1120, 700)] {
            let mut renderer = SoftwareRenderer::new(width, height)?;
            renderer.quality = quality;
            renderer.camera = position;
            renderer.target = target;
            renderer.render(&world.ocean, &world.snapshot());
            let mut times = vec![];
            for _ in 0..30 {
                world.step();
                world.step();
                let start = std::time::Instant::now();
                let frame = renderer.render(&world.ocean, &world.snapshot());
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                std::hint::black_box(frame);
            }
            times.sort_by(f64::total_cmp);
            println!(
                "{name},{width},{height},{:.2},{:.2},{step_us:.2}",
                times.iter().sum::<f64>() / times.len() as f64,
                times[28]
            );
            if quality == 1 {
                renderer
                    .render(&world.ocean, &world.snapshot())
                    .write_ppm(std::fs::File::create(format!("output/phase2/{name}.ppm"))?)?;
            }
        }
    }
    Ok(())
}
