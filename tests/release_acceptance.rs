//! Analytic and long-session release contracts. No field accuracy is inferred.
use pelagic::{
    World,
    math::Vec3,
    ocean::Ocean,
    physics::Vehicle,
    scene::{Scene, world},
};
use std::sync::atomic::AtomicBool;
fn neutral() -> Vehicle {
    Vehicle::new(
        10.0,
        10.0 / 1025.0,
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::ZERO,
        Vec3::ZERO,
    )
    .unwrap()
}
#[test]
fn accelerated_wave_mesh_matches_point_queries() {
    let waves = pelagic::waves::SeaState::spectrum(2.0, 6.0, 1.7, 0.7, 40.0, 42).unwrap();
    for time in [0.0, 13.5, 3600.0] {
        let origin = [-384.0, -384.0];
        let spacing = 3.0;
        let grid = waves.surface_grid(origin, spacing, 257, 65, time).unwrap();
        for y in 0..65 {
            for x in 0..257 {
                let exact = waves.sample(
                    origin[0] + x as f64 * spacing,
                    origin[1] + y as f64 * spacing,
                    time,
                );
                let fast = grid[y * 257 + x];
                assert!((exact.height - fast.height).abs() < 1e-10);
                assert!((exact.normal - fast.normal).length() < 1e-10);
                assert!((exact.vertical_speed - fast.vertical_speed).abs() < 1e-10);
            }
        }
    }
    assert!(waves.surface_grid([0.0, 0.0], 1.0, 258, 1, 0.0).is_err());
}
#[test]
fn one_hour_neutral_drift_and_orientation_norm() {
    let mut w = World::new(Ocean::default(), 1.0 / 60.0, 7).unwrap();
    w.add_vehicle(neutral());
    for _ in 0..216_000 {
        w.step();
    }
    assert!((w.vehicles()[0].position - Vec3::new(0.0, 0.0, -10.0)).length() < 1e-8);
    assert_eq!(w.time(), 3600.0);
    w.checkpoint().validate().unwrap();
}
#[test]
fn semi_implicit_translation_converges_under_timestep_refinement() {
    let mut previous = f64::INFINITY;
    for hz in [30, 60, 120, 240] {
        let mut w = World::new(Ocean::default(), 1.0 / hz as f64, 7).unwrap();
        let mut v = neutral();
        v.force.x = 10.0;
        w.add_vehicle(v);
        for _ in 0..hz * 10 {
            w.step();
        }
        let v = &w.vehicles()[0];
        let error = (v.position.x - 50.0).abs();
        assert!((v.velocity.x - 10.0).abs() < 1e-10);
        assert!(error < previous * 0.501);
        assert!(error <= 5.00000001 / hz as f64);
        previous = error;
    }
}
#[test]
fn terminal_drag_matches_analytic_force_balance() {
    let mut w = World::new(Ocean::default(), 1.0 / 120.0, 7).unwrap();
    let mut v = Vehicle::new(
        10.0,
        10.0 / 1025.0,
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(4.0, 4.0, 4.0),
        Vec3::ZERO,
    )
    .unwrap();
    v.force.x = 16.0;
    w.add_vehicle(v);
    for _ in 0..120 * 120 {
        w.step();
    }
    assert!((w.vehicles()[0].velocity.x - 2.0).abs() < 0.01);
}
#[test]
fn ten_minute_school_stays_finite_bounded_and_outside_obstacles() {
    let mut scene = Scene::ecology_demo();
    scene.school.count = 32;
    scene.particles = 32;
    scene.structure = true;
    scene.structure_position = [
        scene.school.center.x,
        scene.school.center.y,
        scene.school.center.z,
    ];
    scene.structure_scale = 1.0;
    let mut w = world(&scene).unwrap();
    for _ in 0..36_000 {
        w.step();
        for f in &w.schools[0].fish {
            assert!(f.position.is_finite() && f.velocity.is_finite());
            assert!(f.velocity.length() <= scene.school.speed * 1.500001);
            assert!(f.position.z >= w.ocean.bottom.elevation(f.position.x, f.position.y) + 0.59999);
            assert!(
                f.position.z <= w.ocean.surface(f.position.x, f.position.y, w.time()).height - 0.59
            );
            for b in &w.ocean.structures {
                let d = f.position - b.center;
                assert!(d.x.abs() >= b.half.x || d.y.abs() >= b.half.y || d.z.abs() >= b.half.z);
            }
        }
    }
    w.checkpoint().validate().unwrap();
}
#[test]
fn observation_verification_detects_tampering_and_survives_serialization() {
    let scene = Scene::robotics_demo();
    let cancel = AtomicBool::new(false);
    let mut run = pelagic::experiment::batch(&scene, 2.0, &cancel, |_, _| {}).unwrap();
    let mut bytes = vec![];
    run.write(&mut bytes).unwrap();
    let restored = pelagic::experiment::Recording::read(bytes.as_slice()).unwrap();
    pelagic::experiment::verify(&restored, &cancel).unwrap();
    let f = run
        .frames
        .iter_mut()
        .find(|f| !f.observations.is_empty())
        .unwrap();
    f.observations[0].position.x += 0.001;
    assert!(
        pelagic::experiment::verify(&run, &cancel)
            .unwrap_err()
            .contains("Observation")
    );
}
