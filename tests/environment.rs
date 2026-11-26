use pelagic::{
    World,
    geometry::{GeoReference, StaticBox},
    math::Vec3,
    ocean::{Bathymetry, CurrentField, Ocean, ShearedCurrent},
    render::{Renderer, SoftwareRenderer},
    terrain::HeightGrid,
    waves::{SeaState, wave_number},
};
#[test]
fn dispersion_matches_linear_gravity_wave_equation() {
    for depth in [0.5, 2.0, 40.0, 1000.0] {
        for omega in [0.3, 1.0, 3.0] {
            let k = wave_number(omega, depth);
            let predicted = 9.80665 * k * (k * depth).tanh();
            assert!((predicted - omega * omega).abs() / (omega * omega) < 1e-12);
        }
    }
}
#[test]
fn spectrum_energy_seed_and_analytic_derivatives_agree() {
    let sea = SeaState::spectrum(1.2, 6.0, 0.4, 0.5, 40.0, 42).unwrap();
    let repeated = SeaState::spectrum(1.2, 6.0, 0.4, 0.5, 40.0, 42).unwrap();
    assert!((sea.significant_height() - 1.2).abs() < 1e-12);
    let p = sea.sample(3.0, -2.0, 1.0);
    let q = repeated.sample(3.0, -2.0, 1.0);
    assert_eq!(p.height, q.height);
    let epsilon = 1e-4;
    let dx = (sea.sample(3.0 + epsilon, -2.0, 1.0).height
        - sea.sample(3.0 - epsilon, -2.0, 1.0).height)
        / (2.0 * epsilon);
    let dt = (sea.sample(3.0, -2.0, 1.0 + epsilon).height
        - sea.sample(3.0, -2.0, 1.0 - epsilon).height)
        / (2.0 * epsilon);
    assert!((dx + p.normal.x / p.normal.z).abs() < 1e-7);
    assert!((dt - p.vertical_speed).abs() < 1e-7);
    let velocity = sea.orbital_velocity(Vec3::ZERO, 1.0);
    let surface = sea.sample(0.0, 0.0, 1.0);
    assert!((velocity.z - surface.vertical_speed).abs() < 1e-12);
    assert!(
        sea.orbital_velocity(Vec3::new(0.0, 0.0, -40.0), 1.0)
            .z
            .abs()
            < 1e-12
    );
    assert!(SeaState::spectrum(1.0, 0.0, 0.0, 0.0, 40.0, 1).is_err());
}
#[test]
fn long_sample_agrees_with_spectrum_variance() {
    let sea = SeaState::spectrum(2.0, 7.0, 0.0, 0.3, 80.0, 123).unwrap();
    let n = 8192;
    let heights: Vec<f64> = (0..n)
        .map(|i| sea.sample(7.0, 5.0, i as f64 * 0.25).height)
        .collect();
    let mean = heights.iter().sum::<f64>() / n as f64;
    let variance = heights.iter().map(|h| (h - mean).powi(2)).sum::<f64>() / n as f64;
    assert!(mean.abs() < 0.02);
    assert!((4.0 * variance.sqrt() - 2.0).abs() < 0.10);
}
#[test]
fn terrain_interpolates_clamps_and_invalidates_geometry() {
    let mut grid = HeightGrid::parse_csv("# test\n-10,-20\n-30,-40\n", [0.0, 0.0], 2.0).unwrap();
    assert_eq!(grid.elevation(1.0, 1.0), -25.0);
    assert_eq!(grid.elevation(-10.0, -10.0), -10.0);
    assert_eq!(grid.bounds(), [0.0, 0.0, 2.0, 2.0]);
    let revision = grid.revision();
    grid.elevations[0] = -11.0;
    assert_ne!(grid.revision(), revision);
    assert!(HeightGrid::parse_csv("1,2\n3", [0.0, 0.0], 1.0).is_err());
    assert!(HeightGrid::parse_csv("1,NaN\n3,4", [0.0, 0.0], 1.0).is_err());
    assert!((grid.normal(1.0, 1.0).length() - 1.0).abs() < 1e-12);
}
#[test]
fn spatial_water_current_geometry_and_coordinates_are_queryable() {
    let ocean = Ocean::default();
    let shallow = ocean.water(Vec3::new(0.0, 0.0, -1.0));
    let deep = ocean.water(Vec3::new(0.0, 0.0, -100.0));
    assert!(shallow.temperature > deep.temperature);
    assert!(deep.density > shallow.density);
    assert!(deep.sound_speed > 1400.0 && deep.sound_speed < 1600.0);
    let current = ShearedCurrent {
        surface: Vec3::new(1.0, 0.0, 0.0),
        shear: 0.01,
        tide_period: 100.0,
    };
    assert!((current.velocity(Vec3::new(0.0, 0.0, -50.0), 0.0).x - 0.5).abs() < 1e-12);
    assert!(current.velocity(Vec3::ZERO, 50.0).x < current.velocity(Vec3::ZERO, 0.0).x);
    let object = StaticBox {
        center: Vec3::ZERO,
        half: Vec3::new(1.0, 1.0, 1.0),
        color: [1.0; 3],
    };
    assert_eq!(
        object.ray_distance(Vec3::new(-5.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        Some(4.0)
    );
    assert_eq!(
        object.ray_distance(Vec3::new(-5.0, 5.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        None
    );
    let reference = GeoReference {
        latitude: 15.0,
        longitude: 72.0,
        elevation: 0.0,
    };
    let p = Vec3::new(200.0, -100.0, -30.0);
    assert!((reference.geographic_to_local(reference.local_to_geographic(p)) - p).length() < 1e-8);
}
#[test]
fn quality_and_resize_do_not_modify_the_simulation() {
    let ocean = Ocean {
        waves: SeaState::spectrum(1.2, 6.0, 0.0, 0.4, 40.0, 42).unwrap(),
        bottom: Box::new(HeightGrid::procedural(1, 40.0)),
        ..Default::default()
    };
    let world = World::new(ocean, 1.0 / 60.0, 42).unwrap();
    let before = world.snapshot();
    let mut renderer = SoftwareRenderer::new(96, 64).unwrap();
    renderer.quality = 0;
    let low = renderer.render(&world.ocean, &before);
    renderer.quality = 2;
    renderer.resize(128, 80).unwrap();
    let high = renderer.render(&world.ocean, &before);
    assert_eq!(low.rgb.len(), 96 * 64 * 3);
    assert_eq!(high.rgb.len(), 128 * 80 * 3);
    assert_eq!(world.snapshot(), before);
    assert_eq!(high.rgb, renderer.render(&world.ocean, &before).rgb);
    assert!(renderer.resize(0, 1).is_err());
}
