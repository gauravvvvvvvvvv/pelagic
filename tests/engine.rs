use pelagic::{
    World,
    math::{Quat, Vec3},
    ocean::Ocean,
    physics::Vehicle,
    render::{Renderer, SoftwareRenderer},
    sensor::bottom_echo,
};
fn body() -> Vehicle {
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
fn neutral_body_remains_stationary() {
    let mut w = World::new(Ocean::default(), 1.0 / 60.0, 7).unwrap();
    w.add_vehicle(body());
    for _ in 0..600 {
        w.step();
    }
    assert!((w.vehicles()[0].position.z + 10.0).abs() < 1e-10);
    assert_eq!(w.time(), 10.0);
}
#[test]
fn constant_force_and_replay() {
    let run = || {
        let mut w = World::new(Ocean::default(), 0.01, 9).unwrap();
        let mut v = body();
        v.force.x = 10.0;
        w.add_vehicle(v);
        for _ in 0..100 {
            w.step();
        }
        w
    };
    let a = run();
    let b = run();
    assert_eq!(a.snapshot(), b.snapshot());
    assert!((a.vehicles()[0].velocity.x - 1.0).abs() < 1e-12);
    assert!((a.vehicles()[0].position.x - 0.505).abs() < 1e-12);
}
#[test]
fn commands_rotate_from_body_to_world() {
    let mut w = World::new(Ocean::default(), 0.01, 0).unwrap();
    let mut v = body();
    v.orientation = Quat::from_rotation_vector(Vec3::new(0.0, 0.0, std::f64::consts::FRAC_PI_2));
    v.force.x = 10.0;
    w.add_vehicle(v);
    w.step();
    assert!(w.vehicles()[0].velocity.x.abs() < 1e-12);
    assert!((w.vehicles()[0].velocity.y - 0.01).abs() < 1e-12);
}
#[test]
fn sonar_range_and_travel_time() {
    let o = Ocean::default();
    let e = bottom_echo(
        &o,
        Vec3::new(0.0, 0.0, -10.0),
        Vec3::new(0.0, 0.0, -1.0),
        100.0,
        3.0,
    )
    .unwrap();
    assert!((e.range - 90.0).abs() < 1e-8);
    assert!((e.round_trip_seconds - 0.12).abs() < 1e-10);
    assert!(
        bottom_echo(
            &o,
            Vec3::new(0.0, 0.0, -10.0),
            Vec3::new(0.0, 0.0, 1.0),
            100.0,
            1.0
        )
        .is_none()
    );
}
#[test]
fn render_is_repeatable_and_read_only() {
    let mut w = World::new(Ocean::default(), 0.01, 0).unwrap();
    w.add_vehicle(body());
    let before = w.snapshot();
    let mut r = SoftwareRenderer::new(32, 18).unwrap();
    let a = r.render(&w.ocean, &before);
    let b = r.render(&w.ocean, &before);
    assert_eq!(a.rgb.len(), 32 * 18 * 3);
    assert_eq!(a.rgb, b.rgb);
    assert_eq!(before, w.snapshot());
    let mut bytes = Vec::new();
    a.write_ppm(&mut bytes).unwrap();
    assert!(bytes.starts_with(b"P6\n32 18\n255\n"));
}
#[test]
fn invalid_parameters_are_rejected() {
    assert!(World::new(Ocean::default(), f64::NAN, 0).is_err());
    assert!(Vehicle::new(0.0, 0.0, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO).is_err());
    assert!(SoftwareRenderer::new(0, 1).is_err());
}
