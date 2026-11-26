use pelagic::{
    World,
    math::Vec3,
    ocean::{Bathymetry, Ocean, UniformCurrent},
    physics::Vehicle,
    render::{Renderer, SoftwareRenderer},
};

struct Canyon;
impl Bathymetry for Canyon {
    fn elevation(&self, x: f64, y: f64) -> f64 {
        -28.0 - 12.0 * (-(x * x) / 180.0).exp() + 2.0 * (y / 12.0).sin()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ocean = Ocean {
        bottom: Box::new(Canyon),
        current: Box::new(UniformCurrent(Vec3::new(0.15, 0.0, 0.0))),
        ..Ocean::default()
    };
    let density = ocean.density;
    let mut world = World::new(ocean, 1.0 / 60.0, 42)?;
    let mut vehicle = Vehicle::new(
        12.0,
        12.0 / density,
        Vec3::new(0.3, 0.4, 0.5),
        Vec3::new(8.0, 12.0, 15.0),
        Vec3::new(0.4, 0.4, 0.4),
    )?;
    vehicle.position = Vec3::new(0.0, 0.0, -20.0);
    vehicle.force = Vec3::new(0.3, 0.0, 0.0);
    world.add_vehicle(vehicle);
    for _ in 0..600 {
        world.step();
    }
    let snapshot = world.snapshot();
    println!(
        "time={:.3}s tick={} seed={} position={:?}",
        snapshot.time, snapshot.tick, snapshot.seed, snapshot.vehicles[0].position
    );
    std::fs::create_dir_all("output")?;
    let mut renderer = SoftwareRenderer::new(640, 360)?;
    renderer
        .render(&world.ocean, &snapshot)
        .write_ppm(std::fs::File::create("output/ocean.ppm")?)?;
    println!("Rendered output/ocean.ppm");
    Ok(())
}
