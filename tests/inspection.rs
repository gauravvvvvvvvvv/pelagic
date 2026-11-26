//! Inspection: stand-off from GSD and water clarity, passes covering every
//! underwater face, routes around the structure, coverage judged per patch,
//! and a hovering ROV flying the plan with its physics.
use pelagic::entities::Entity;
use pelagic::inspection::{self, Setup};
use pelagic::math::Vec3;
use pelagic::scene::Scene;
use std::sync::atomic::AtomicBool;

fn calm() -> Scene {
    Scene {
        wave_height: 0.0,
        ..Scene::default()
    }
}

/// A 20 × 6 × 4 m box hull floating with a 1.5 m draft.
fn barge() -> Entity {
    let mut e = Entity::boat(7);
    e.name = "Barge".into();
    e.model.half = Vec3::new(10.0, 3.0, 2.0);
    e.volume = 20.0 * 6.0 * 4.0;
    e.mass = 1025.0 * 20.0 * 6.0 * 1.5;
    e.position = Vec3::new(0.0, 0.0, 0.0);
    e.heading = 30.0;
    e
}

#[test]
fn standoff_follows_gsd_and_visibility() {
    let s = Setup::default();
    // 70° over 1920 px: 0.636 mrad per pixel; 1 mm/px at 1.57 m.
    assert!((s.gsd_range_m() - 1.5716).abs() < 1e-3);
    // Contrast 0.25 at c = 0.5/m: ln 4 / 0.5 = 2.77 m.
    assert!((s.visibility_m() - 2.7726).abs() < 1e-3);
    let (d, notes) = s.standoff();
    assert!((d - 0.85 * 1.5716).abs() < 1e-3 && notes.is_empty());
    // Murky water: visibility shorter than the clearance.
    let murky = Setup {
        attenuation: 3.0,
        ..Setup::default()
    };
    let (d, notes) = murky.standoff();
    assert_eq!(d, murky.clearance_m);
    assert_eq!(notes.len(), 1);
}

#[test]
fn a_hull_is_covered_from_a_safe_stand_off() {
    let target = inspection::hull(&barge(), 1025.0).unwrap();
    // Bottom and four sides below the waterline (draft 1.5 m).
    assert_eq!(target.faces.len(), 5);
    let side = target
        .faces
        .iter()
        .find(|f| f.name.contains("port"))
        .unwrap();
    assert!((side.half[1] - 0.7).abs() < 0.01, "{side:?}");
    let setup = Setup::default();
    let (path, d, _) = inspection::plan(&target, &setup).unwrap();
    let run = inspection::ideal(&target, &setup, &path, d);
    assert!(run.covered_percent > 99.0, "{:?}", run.faces);
    // Every point of the path keeps the clearance, including the transits.
    for w in &path {
        assert!(
            target.boxes[0].distance(w.position) >= setup.clearance_m * 0.89,
            "{w:?}"
        );
    }
    for pair in path.windows(2) {
        for k in 0..=20 {
            let p = pair[0].position + (pair[1].position - pair[0].position) * (k as f64 / 20.0);
            assert!(target.boxes[0].distance(p) > 0.3, "{p:?}");
        }
    }
    // In murky water the frames do not meet the requirement.
    let murky = Setup {
        attenuation: 3.0,
        ..Setup::default()
    };
    let (path, d, notes) = inspection::plan(&target, &murky).unwrap();
    let run = inspection::ideal(&target, &murky, &path, d);
    assert!(
        !notes.is_empty() && run.covered_percent < 5.0,
        "{}",
        run.covered_percent
    );
}

#[test]
fn an_rov_inspects_the_pilings_with_its_physics() {
    let scene = calm();
    let target = inspection::pilings(&scene).unwrap();
    assert_eq!(target.faces.len(), 16);
    let setup = Setup {
        attenuation: 0.2,
        ..Setup::default()
    };
    let rov = scene.reference_entity();
    let r = inspection::run(
        &scene,
        target,
        &setup,
        Some(&rov),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(r.ideal.covered_percent > 95.0, "{}", r.summary());
    let sim = r.simulated.as_ref().unwrap();
    assert!(sim.covered_percent > 85.0, "{}", r.summary());
    assert!(sim.position_rms_m < 0.3, "{}", r.summary());
    assert!(sim.min_clearance_m > 0.0, "{}", r.summary());
    let folder = std::env::temp_dir().join(format!("pelagic-inspect-{}", std::process::id()));
    r.write(&folder).unwrap();
    for name in [
        "summary.txt",
        "inspection.json",
        "faces.csv",
        "path.csv",
        "track.csv",
    ] {
        assert!(folder.join(name).exists(), "{name}");
    }
    let _ = std::fs::remove_dir_all(&folder);
    // A floating vessel cannot hover for the inspection.
    let boat = Entity::boat(3);
    let target = inspection::pilings(&scene).unwrap();
    assert!(
        inspection::run(
            &scene,
            target,
            &setup,
            Some(&boat),
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
}
