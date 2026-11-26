use pelagic::{
    entities::Entity,
    experiment::{self, Recording},
    math::{Quat, Vec3},
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
        vehicle: false,
        school: pelagic::life::SchoolConfig {
            count: 0,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn tilt_deg(q: Quat) -> f64 {
    q.rotate(Vec3::new(0.0, 0.0, 1.0))
        .z
        .clamp(-1.0, 1.0)
        .acos()
        .to_degrees()
}

/// The shipped reference was recorded by the editor executable; this test
/// binary is a different executable. Exact re-simulation across them proves
/// the simulation maths is portable (see `math::Det`).
#[test]
fn shipped_reference_recording_verifies_in_another_executable() {
    let file = std::fs::File::open("examples/runs/v5-reference.pelagic-run").unwrap();
    let run = Recording::read(file).unwrap();
    experiment::verify(&run, &AtomicBool::new(false)).unwrap();
}

#[test]
fn a_heeled_boat_rights_itself_in_calm_water() {
    let mut scene = calm();
    scene.entities.push(Entity::boat(1));
    let mut w = world(&scene).unwrap();
    for _ in 0..1200 {
        w.step();
    }
    let draft = w.vehicles()[0].position.z;
    let boat = w.vehicle_mut(0).unwrap();
    boat.orientation = Quat::from_rotation_vector(Vec3::new(0.35, 0.0, 0.0)) * boat.orientation;
    let heeled = tilt_deg(w.vehicles()[0].orientation);
    assert!(heeled > 19.0, "{heeled}");
    let mut worst = 0.0f64;
    for i in 0..1800 {
        w.step();
        let tilt = tilt_deg(w.vehicles()[0].orientation);
        assert!(
            tilt.is_finite() && tilt < 45.0,
            "capsized: {tilt}° at step {i}"
        );
        if i > 1200 {
            worst = worst.max(tilt);
        }
    }
    assert!(worst < 2.0, "still heeled {worst}° after 20 s");
    let z = w.vehicles()[0].position.z;
    assert!((z - draft).abs() < 0.05, "draft moved {draft} -> {z}");
}

#[test]
fn a_boat_heaves_and_pitches_with_waves_without_instability() {
    let mut scene = calm();
    scene.wave_height = 1.2;
    scene.wave_period = 5.0;
    scene.wave_spread = 0.3;
    scene.entities.push(Entity::boat(1));
    let mut w = world(&scene).unwrap();
    let (mut tilt_max, mut z_min, mut z_max) = (0.0f64, f64::INFINITY, f64::NEG_INFINITY);
    for i in 0..3600 {
        w.step();
        let boat = &w.vehicles()[0];
        assert!(boat.position.is_finite() && boat.velocity.is_finite());
        if i > 600 {
            tilt_max = tilt_max.max(tilt_deg(boat.orientation));
            z_min = z_min.min(boat.position.z);
            z_max = z_max.max(boat.position.z);
        }
    }
    // It rides the waves: visible pitch/roll and heave, but stays afloat.
    assert!(tilt_max > 1.0 && tilt_max < 35.0, "tilt {tilt_max}°");
    assert!(z_max - z_min > 0.2, "heave range {}", z_max - z_min);
    assert!(z_min > -2.0 && z_max < 2.0, "z {z_min}..{z_max}");
}

#[test]
fn submerged_bodies_keep_the_lumped_model() {
    let scene = Scene {
        vehicle: true,
        vehicle_depth: 10.0,
        ..calm()
    };
    let w = world(&scene).unwrap();
    let v = &w.vehicles()[0];
    assert!(v.model.partial_buoyancy);
    assert!(pelagic::dynamics::waterline(v, &w.ocean, 0.0, 1025.0).is_none());
    let mut boat_scene = calm();
    boat_scene.entities.push(Entity::boat(1));
    let w = world(&boat_scene).unwrap();
    let line = pelagic::dynamics::waterline(&w.vehicles()[0], &w.ocean, 0.0, 1025.0).unwrap();
    assert!(line.fraction > 0.0 && line.fraction < 1.0);
    assert!(line.pitch_stiffness > line.roll_stiffness && line.roll_stiffness > 0.0);
}
