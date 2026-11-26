use pelagic::{
    World,
    assets::Mesh,
    dynamics::BodyModel,
    entities::Entity,
    experiment::{self, Recording},
    instruments::{self, SensorConfig},
    math::Vec3,
    ocean::{FlatBottom, Ocean},
    physics::Vehicle,
    scene::{Scene, world},
};
use std::sync::atomic::AtomicBool;
fn calm() -> Scene {
    Scene {
        terrain: 0,
        wave_height: 0.0,
        current: [0.0; 3],
        structure: false,
        particles: 0,
        school: pelagic::life::SchoolConfig {
            count: 0,
            ..Default::default()
        },
        ..Default::default()
    }
}
#[test]
fn surface_vessel_settles_and_contact_prevents_seabed_penetration() {
    let mut scene = calm();
    scene.vehicle = false;
    scene.entities.push(Entity::boat(1));
    let mut w = world(&scene).unwrap();
    for _ in 0..2400 {
        w.step();
    }
    let boat = &w.vehicles()[0];
    assert!(
        boat.position.z.abs() < 0.5,
        "boat height {}",
        boat.position.z
    );
    assert!(boat.velocity.z.abs() < 0.05, "boat did not settle");
    let mut scene = calm();
    scene.bottom_depth = 10.0;
    scene.vehicle_depth = 8.0;
    scene.reference_force.z = -100.0;
    scene.body_model.profile_density = false;
    let mut w = world(&scene).unwrap();
    for _ in 0..600 {
        w.step();
        assert!(w.vehicles()[0].position.z >= -9.60001);
    }
}
#[test]
fn added_mass_and_saturated_lagged_actuators_bound_acceleration() {
    let mut v = Vehicle::new(
        10.0,
        10.0 / 1025.0,
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::ZERO,
        Vec3::ZERO,
    )
    .unwrap();
    v.model = BodyModel {
        added_mass: Vec3::new(10.0, 0.0, 0.0),
        thrust_limit: 10.0,
        response_seconds: 0.0,
        ..Default::default()
    };
    v.force.x = 1000.0;
    let mut w = World::new(Ocean::default(), 0.01, 0).unwrap();
    w.add_vehicle(v);
    w.step();
    assert!((w.vehicles()[0].velocity.x - 0.005).abs() < 1e-10);
    let body = w.vehicle_mut(0).unwrap();
    body.model.response_seconds = 1.0;
    body.force.x = 0.0;
    w.step();
    assert!(w.vehicles()[0].velocity.x < 0.01);
}
#[test]
fn depth_heading_hold_converges_without_exceeding_limits() {
    let mut scene = calm();
    scene.vehicle_depth = 10.0;
    scene.body_model.hold_depth = Some(12.0);
    scene.body_model.hold_heading = Some(0.7);
    scene.body_model.profile_density = false;
    let mut w = world(&scene).unwrap();
    for _ in 0..1800 {
        w.step();
    }
    let v = &w.vehicles()[0];
    assert!((v.position.z + 12.0).abs() < 0.12, "depth {}", v.position.z);
    let f = v.orientation.rotate(Vec3::new(1.0, 0.0, 0.0));
    assert!((f.y.atan2(f.x) - 0.7).abs() < 0.05);
}
#[test]
fn body_contact_conserves_pair_momentum_and_removes_overlap() {
    let mut s = calm();
    s.vehicle = false;
    for (id, x, vx) in [(1, -0.4, 1.0), (2, 0.4, -1.0)] {
        let mut e = Entity::rov(id, Vec3::new(x, 0.0, -20.0));
        e.model.profile_density = false;
        e.model.added_mass = Vec3::ZERO;
        e.model.center_buoyancy = Vec3::ZERO;
        s.entities.push(e);
        let _ = vx;
    }
    let mut w = world(&s).unwrap();
    w.vehicle_mut(0).unwrap().velocity.x = 1.0;
    w.vehicle_mut(1).unwrap().velocity.x = -1.0;
    w.step();
    let a = &w.vehicles()[0];
    let b = &w.vehicles()[1];
    assert!((a.velocity.x + b.velocity.x).abs() < 1e-10);
    assert!((a.position - b.position).length() > 1.29);
}
#[test]
fn school_is_seeded_finite_depth_bounded_and_responds_to_vehicles() {
    let mut s = calm();
    s.school.count = 32;
    let mut a = world(&s).unwrap();
    let mut b = world(&s).unwrap();
    for _ in 0..120 {
        a.step();
        b.step();
    }
    assert_eq!(a.snapshot(), b.snapshot());
    assert!(
        a.snapshot()
            .fish
            .iter()
            .all(|f| f.position.is_finite() && f.position.z < 0.0 && f.position.z > -40.0)
    );
    let mut c = world(&s).unwrap();
    c.vehicle_mut(0).unwrap().position = s.school.center;
    for _ in 0..120 {
        c.step();
    }
    assert_ne!(a.snapshot().fish, c.snapshot().fish);
}
#[test]
fn sonar_hits_shared_geometry_and_repeats_seeded_observations() {
    let mut o = Ocean {
        bottom: Box::new(FlatBottom(-40.0)),
        ..Default::default()
    };
    o.structures.push(pelagic::geometry::StaticBox {
        center: Vec3::new(0.0, 0.0, -18.0),
        half: Vec3::new(2.0, 2.0, 1.0),
        color: [0.0; 3],
    });
    let mut w = World::new(o, 1.0 / 60.0, 42).unwrap();
    let mut v = Entity::rov(1, Vec3::new(0.0, 0.0, -10.0)).spawn();
    v.id = 0;
    w.add_vehicle(v);
    let c = SensorConfig {
        beams: 1,
        mount: Vec3::ZERO,
        tilt_degrees: 90.0,
        range_noise: 0.0,
        position_noise: 0.0,
        ..Default::default()
    };
    let obs = instruments::sample(&w.ocean, &w.snapshot(), &[Vec3::ZERO], 0, &c).unwrap();
    let beam = &obs.beams[0];
    assert!((beam.range.unwrap() - 7.0).abs() < 1e-8);
    assert_eq!(beam.target, 1_000_000);
    assert!(beam.travel_seconds.unwrap() > 0.008 && beam.travel_seconds.unwrap() < 0.011);
    assert_eq!(
        obs,
        instruments::sample(&w.ocean, &w.snapshot(), &[Vec3::ZERO], 0, &c).unwrap()
    );
}
#[test]
fn recording_roundtrip_replay_validation_and_cancel_are_independent_of_rendering() {
    let mut scene = calm();
    scene.school.count = 8;
    let stop = AtomicBool::new(false);
    let run = experiment::batch(&scene, 1.0, &stop, |_, _| {}).unwrap();
    assert_eq!(run.frames.len(), 11);
    experiment::verify(&run, &stop).unwrap();
    let mut bytes = vec![];
    run.write(&mut bytes).unwrap();
    let loaded = Recording::read(bytes.as_slice()).unwrap();
    assert_eq!(loaded, run);
    experiment::verify(&loaded, &stop).unwrap();
    let mut altered = loaded.clone();
    altered.frames[1].truth.vehicles[0].position.x += 0.1;
    assert!(experiment::verify(&altered, &stop).is_err());
    let stop = AtomicBool::new(true);
    assert!(experiment::batch(&scene, 1.0, &stop, |_, _| {}).is_err());
    let mut truth = vec![];
    let mut obs = vec![];
    run.export_csv(&mut truth, &mut obs).unwrap();
    assert!(truth.starts_with(b"tick,time_s,entity_id"));
    assert!(obs.starts_with(b"time_s,available_s"));
}
#[test]
fn data_adapters_meshes_and_schema_reject_ambiguous_inputs() {
    let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n";
    let mesh = Mesh::parse_obj(obj, "own triangle").unwrap();
    assert_eq!(mesh.triangles.len(), 1);
    assert!(Mesh::parse_obj("v 0 0 0\nf 1 2 3", "bad").is_err());
    let asc =
        "ncols 2\nnrows 2\nxllcorner 0\nyllcorner 0\ncellsize 2\nNODATA_value -9999\n1 2\n3 4";
    let grid = pelagic::data::esri_ascii(asc).unwrap();
    assert_eq!(grid.elevations, vec![3.0, 4.0, 1.0, 2.0]);
    assert!(pelagic::data::esri_ascii(&asc.replace("1 2", "-9999 2")).is_err());
    let mut scene = calm();
    let mut e = Entity::boat(7);
    e.asset = Some(
        scene
            .library
            .add_mesh("own triangle", std::sync::Arc::new(mesh))
            .unwrap(),
    );
    scene.entities.push(e);
    let encoded = serde_json::to_vec(&scene.value()).unwrap();
    assert_eq!(
        Scene::from_value(&serde_json::from_slice(&encoded).unwrap()).unwrap(),
        scene
    );
    let mut bad = scene.value();
    bad["entities"][0]["mass"] = serde_json::json!(-1);
    assert!(Scene::from_value(&bad).is_err());
}
#[test]
fn checkpoint_and_command_timeline_reproduce_a_run_started_mid_simulation() {
    let scene = calm();
    let mut w = world(&scene).unwrap();
    for _ in 0..45 {
        w.step();
    }
    let mut run = Recording::new(scene.value().to_string(), 1.0 / 60.0);
    run.checkpoint = Some(w.checkpoint());
    for i in 0..121 {
        if i == 20 {
            w.vehicle_mut(0).unwrap().force.x = 5.0;
        }
        if i == 80 {
            w.vehicle_mut(0).unwrap().force.x = -3.0;
        }
        run.capture_commands(&w);
        if i % 6 == 0 {
            run.append(experiment::RunFrame {
                truth: w.snapshot(),
                velocities: w.vehicles().iter().map(|v| v.velocity).collect(),
                observations: vec![],
            })
            .unwrap();
        }
        if i < 120 {
            w.step();
        }
    }
    experiment::verify(&run, &AtomicBool::new(false)).unwrap();
    let mut bytes = vec![];
    run.write(&mut bytes).unwrap();
    let copy = Recording::read(bytes.as_slice()).unwrap();
    experiment::verify(&copy, &AtomicBool::new(false)).unwrap();
    assert_eq!(copy.commands.len(), 3);
    let mut bad = copy;
    bad.commands[1].force.x += 1.0;
    assert!(experiment::verify(&bad, &AtomicBool::new(false)).is_err());
}
#[test]
fn sampling_observes_configured_latency_and_mavlink_validates_crc_and_axes() {
    let mut w = world(&calm()).unwrap();
    let mut suite = instruments::Suite::new(SensorConfig {
        latency_seconds: 0.2,
        ..Default::default()
    });
    suite.update(&w, 1.0 / 60.0);
    assert!(suite.latest.is_empty());
    for _ in 0..11 {
        w.step();
        suite.update(&w, 1.0 / 60.0);
    }
    assert!(suite.latest.is_empty());
    w.step();
    suite.update(&w, 1.0 / 60.0);
    assert_eq!(suite.latest[0].time, 0.0);
    assert_eq!(suite.latest[0].available_time, 0.2);
    let mut payload = vec![];
    for value in [1000i16, 500, -500, 250] {
        payload.extend_from_slice(&value.to_le_bytes());
    }
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.push(1);
    let packet = pelagic::bridge::packet(69, &payload, 243, 3);
    let command = pelagic::bridge::manual(&packet, 40.0).unwrap();
    assert_eq!(command.force, Vec3::new(40.0, -20.0, 20.0));
    let mut corrupt = packet;
    corrupt[7] ^= 1;
    assert!(pelagic::bridge::manual(&corrupt, 40.0).is_none());
    let ned = pelagic::bridge::local_position(Vec3::new(3.0, 4.0, -5.0), Vec3::ZERO, 1.0, 0);
    assert_eq!(f32::from_le_bytes(ned[10..14].try_into().unwrap()), 4.0);
    assert_eq!(f32::from_le_bytes(ned[14..18].try_into().unwrap()), 3.0);
    assert_eq!(f32::from_le_bytes(ned[18..22].try_into().unwrap()), 5.0);
}
#[test]
fn thruster_allocation_tracks_feasible_wrenches_and_saturates() {
    let t = pelagic::actuators::rov();
    let force = Vec3::new(10.0, 3.0, 5.0);
    let torque = Vec3::new(0.0, 1.0, 0.5);
    let values = pelagic::actuators::allocate(&t, force, torque);
    let actual = pelagic::actuators::wrench(&t, &values);
    assert!((actual.0 - force).length() < 0.02);
    assert!((actual.1 - torque).length() < 0.02);
    let saturated = pelagic::actuators::allocate(&t, force * 100.0, torque);
    assert!(saturated.iter().all(|v| v.abs() <= 20.0));
}
#[test]
fn real_regional_and_synthetic_current_data_keep_coordinates_and_units() {
    let text = include_str!("../examples/data/current-grid.csv");
    let flow = pelagic::data::current_csv(text).unwrap();
    let field = flow.field().unwrap();
    let p = Vec3::new(0.0, 0.0, -20.0);
    let initial = pelagic::ocean::CurrentField::velocity(&field, p, 0.0);
    assert!((initial.x - 0.25).abs() < 1e-12);
    assert!(pelagic::ocean::CurrentField::velocity(&field, p, 30.0).length() < 1e-12);
    let (grid, latitude, longitude) =
        pelagic::data::geographic_csv(include_str!("../examples/data/goa-etopo1.csv")).unwrap();
    assert!((latitude - 15.0).abs() < 1e-8);
    assert!((longitude - 73.5).abs() < 1e-8);
    assert!((1000.0..2000.0).contains(&grid.spacing));
    assert!(grid.elevations.iter().all(|v| v.is_finite()));
    let origin = pelagic::geometry::GeoReference {
        latitude,
        longitude,
        elevation: 0.0,
    };
    let corner = origin.local_to_geographic(Vec3::new(grid.origin[0], grid.origin[1], 0.0));
    assert!((corner[0] - 14.8).abs() < 1e-8);
    assert!((corner[1] - 73.3).abs() < 1e-8);
}
#[test]
fn sensor_motion_observations_have_specific_force_and_separate_truth() {
    let mut scene = calm();
    scene.body_model.profile_density = false;
    scene.sensors.latency_seconds = 0.0;
    scene.sensors.position_noise = 0.0;
    scene.sensors.velocity_sigma = 0.0;
    scene.sensors.gyro_sigma = 0.0;
    scene.sensors.acceleration_sigma = 0.0;
    let mut w = world(&scene).unwrap();
    let mut suite = instruments::Suite::new(scene.sensors.clone());
    suite.update(&w, 1.0 / 60.0);
    assert!((suite.latest[0].specific_force.z - 9.80665).abs() < 1e-10);
    assert_eq!(suite.latest[0].angular_velocity, Vec3::ZERO);
    for _ in 0..6 {
        w.step();
    }
    suite.update(&w, 1.0 / 60.0);
    assert!(suite.latest[0].specific_force.is_finite());
}
#[test]
fn exported_dataset_is_self_contained_and_marks_camera_calibration() {
    let scene = calm();
    let cancel = AtomicBool::new(false);
    let run = experiment::batch(&scene, 0.2, &cancel, |_, _| {}).unwrap();
    let path = std::env::temp_dir().join(format!(
        "pelagic-dataset-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    pelagic::dataset::camera(&path, &run, &cancel, |_, _| {}).unwrap();
    assert!(!path.join("INCOMPLETE.txt").exists());
    for name in [
        "scene.pelagic",
        "experiment.pelagic-run",
        "truth.csv",
        "sonar.csv",
        "navigation.csv",
        "fish.csv",
        "commands.csv",
        "camera/calibration.json",
    ] {
        assert!(path.join(name).is_file(), "{name}");
    }
    let loaded =
        Recording::read(std::fs::File::open(path.join("experiment.pelagic-run")).unwrap()).unwrap();
    experiment::verify(&loaded, &cancel).unwrap();
    assert!(pelagic::dataset::camera(&path, &run, &cancel, |_, _| {}).is_err());
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn camera_roll_and_fov_invalidate_cached_projection() {
    use pelagic::render::Renderer;
    let scene = Scene::default();
    let w = world(&scene).unwrap();
    let s = w.snapshot();
    let mut r = pelagic::render::SoftwareRenderer::new(64, 40).unwrap();
    let before = r.render(&w.ocean, &s);
    r.camera_up = Vec3::new(0.5, 0.0, 1.0);
    r.vertical_fov = 80.0;
    let changed = r.render(&w.ocean, &s);
    assert_ne!(before.rgb, changed.rgb);
    let mut fresh = pelagic::render::SoftwareRenderer::new(64, 40).unwrap();
    fresh.camera_up = r.camera_up;
    fresh.vertical_fov = r.vertical_fov;
    assert_eq!(fresh.render(&w.ocean, &s).rgb, changed.rgb);
    assert_eq!(w.snapshot(), s);
}
