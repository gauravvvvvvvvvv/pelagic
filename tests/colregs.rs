//! Collision regulations: CPA geometry, roles under Rules 13–15 and 18, the
//! reference autopilot over the standard and random encounters, and a
//! debrief that catches wrong manoeuvres.
use pelagic::colregs::{
    self, Motion, Rules, Session, Situation, VesselKind, classify, cpa, relative_bearing, turn,
};
use pelagic::scene::Scene;
use std::sync::atomic::AtomicBool;

fn motion(x: f64, y: f64, heading: f64, speed: f64) -> Motion {
    Motion {
        position: [x, y],
        velocity: pelagic::sar::towards(speed, heading),
        heading_deg: heading,
    }
}

fn workboat() -> pelagic::entities::Entity {
    let text =
        std::fs::read_to_string("examples/vehicles/workboat-usv.pelagic-vehicle.json").unwrap();
    pelagic::definition::VehicleDefinition::parse(&text)
        .unwrap()
        .to_entity(1, pelagic::math::Vec3::ZERO, 0.0)
}

fn scene() -> Scene {
    Scene {
        structure: false,
        ..Scene::default()
    }
}

#[test]
fn closest_point_of_approach_and_bearings() {
    let own = motion(0.0, 0.0, 0.0, 5.0);
    let (d, t) = cpa(&own, &motion(0.0, 1000.0, 180.0, 5.0));
    assert!(d.abs() < 1e-9 && (t - 100.0).abs() < 1e-9);
    // Crossing at right angles, 100 m ahead of the meeting point.
    let (d, t) = cpa(&own, &motion(600.0, 500.0, 270.0, 6.0));
    let expected_t = (500.0 * 5.0 + 600.0 * 6.0) / 61.0;
    assert!((t - expected_t).abs() < 1e-6, "{t}");
    assert!(d > 0.0 && d < 150.0);
    // Opening: TCPA negative.
    assert!(cpa(&own, &motion(0.0, -100.0, 180.0, 5.0)).1 < 0.0);
    assert!((relative_bearing(&own, [100.0, 0.0]) - 90.0).abs() < 1e-9);
    let east = motion(0.0, 0.0, 90.0, 5.0);
    assert!((relative_bearing(&east, [0.0, 100.0]) - 270.0).abs() < 1e-9);
    assert_eq!(turn(350.0, 10.0), 20.0);
    assert_eq!(turn(10.0, 350.0), -20.0);
}

#[test]
fn roles_follow_rules_13_to_15_and_18() {
    let r = Rules::default();
    let own = motion(0.0, 0.0, 0.0, 5.0);
    let p = VesselKind::PowerDriven;
    let case = |t: Motion, k: VesselKind| classify(&own, p, &t, k, &r);
    assert_eq!(case(motion(0.0, 900.0, 180.0, 5.0), p), Situation::HeadOn);
    assert_eq!(
        case(motion(600.0, 600.0, 270.0, 5.0), p),
        Situation::CrossingGiveWay
    );
    assert_eq!(
        case(motion(-600.0, 600.0, 90.0, 5.0), p),
        Situation::CrossingStandOn
    );
    assert_eq!(case(motion(0.0, 400.0, 0.0, 2.0), p), Situation::Overtaking);
    assert_eq!(
        case(motion(0.0, -400.0, 0.0, 9.0), p),
        Situation::BeingOvertaken
    );
    // Rule 18: a power-driven vessel keeps out of the way of a fishing
    // vessel even on its port side...
    assert_eq!(
        case(motion(-600.0, 600.0, 90.0, 3.0), VesselKind::Fishing),
        Situation::GiveWayRule18
    );
    // ...but an overtaking vessel keeps clear regardless (Rule 13).
    assert_eq!(
        case(motion(0.0, 400.0, 0.0, 2.0), VesselKind::Fishing),
        Situation::Overtaking
    );
    let nuc = classify(
        &own,
        VesselKind::NotUnderCommand,
        &motion(600.0, 600.0, 270.0, 5.0),
        p,
        &r,
    );
    assert_eq!(nuc, Situation::StandOnRule18);
    assert!(Situation::HeadOn.give_way() && !Situation::CrossingStandOn.give_way());
}

#[test]
fn the_autopilot_meets_the_rules_in_every_standard_and_random_encounter() {
    let (scene, boat) = (scene(), workboat());
    let (speed, length) = colregs::service_speed(&scene, &boat).unwrap();
    assert!(speed > 3.0 && speed < 12.0, "{speed}");
    let rules = Rules::for_length(length);
    let mut scenarios = colregs::standard(speed, length, &rules);
    assert_eq!(scenarios.len(), 8);
    for seed in 1..=6 {
        scenarios.push(colregs::random(speed, length, &rules, seed));
    }
    let results = colregs::run_suite(
        &scene,
        &boat,
        &scenarios,
        &rules,
        4,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap();
    for (a, log) in &results {
        assert!(a.passed, "{}", a.summary());
        assert!(a.autopilot && !log.is_empty());
        for t in &a.targets {
            assert!(!t.collided);
            assert!(t.min_range_m >= rules.safe_cpa_m * 0.95, "{}", a.summary());
        }
    }
    // The give-way head-on vessel turned to starboard, substantially.
    let head_on = &results[0].0.targets[0];
    assert_eq!(head_on.situation, Some(Situation::HeadOn));
    let action = head_on.action.as_ref().unwrap();
    assert!(action.course_change_deg >= 29.0, "{action:?}");
    // The stand-on vessel whose give-way vessel acted kept her course.
    assert!(
        results[2].0.targets[0].action.is_none(),
        "{}",
        results[2].0.summary()
    );
    let folder = std::env::temp_dir().join(format!("pelagic-colregs-{}", std::process::id()));
    colregs::write_suite(&folder, &results).unwrap();
    assert!(folder.join("summary.txt").exists() && folder.join("colregs.json").exists());
    assert!(folder.join("track-head-on.csv").exists());
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn the_debrief_catches_wrong_manoeuvres() {
    let (scene, boat) = (scene(), workboat());
    let (speed, length) = colregs::service_speed(&scene, &boat).unwrap();
    let rules = Rules::for_length(length);
    let set = colregs::standard(speed, length, &rules);
    // Head-on: the helm turns to port.
    let mut s = Session::new(&scene, &boat, set[0].clone(), rules.clone()).unwrap();
    s.step(3.0);
    s.set_order(320.0, speed);
    while !s.finished() {
        s.step(1.0);
    }
    let a = s.assess();
    assert!(!a.passed);
    let failed: Vec<&str> = a.targets[0]
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.rule.as_str())
        .collect();
    assert!(failed.contains(&"Rule 14"), "{}", a.summary());
    // Crossing, stand on: the helm alters early instead of standing on.
    let mut s = Session::new(&scene, &boat, set[2].clone(), rules.clone()).unwrap();
    s.step(3.0);
    s.set_order(60.0, speed);
    while !s.finished() {
        s.step(1.0);
    }
    let a = s.assess();
    assert!(
        a.targets[0]
            .checks
            .iter()
            .any(|c| c.rule == "Rule 17(a)(i)" && !c.passed),
        "{}",
        a.summary()
    );
    // Crossing, give way: no action at all ends too close.
    let mut s = Session::new(&scene, &boat, set[1].clone(), rules).unwrap();
    while !s.finished() {
        s.step(1.0);
    }
    let a = s.assess();
    assert!(!a.passed && a.targets[0].min_range_m < s.rules().safe_cpa_m);
}

#[test]
fn sessions_are_deterministic_and_need_a_surface_vessel() {
    let (scene, boat) = (scene(), workboat());
    let rules = Rules::for_length(6.0);
    let sc = colregs::random(5.0, 6.0, &rules, 3);
    let run = || {
        let mut s = Session::new(&scene, &boat, sc.clone(), rules.clone()).unwrap();
        s.set_autopilot(true);
        s.step(60.0);
        s.log().last().cloned().unwrap()
    };
    assert_eq!(run(), run());
    let text =
        std::fs::read_to_string("examples/vehicles/survey-auv.pelagic-vehicle.json").unwrap();
    let auv = pelagic::definition::VehicleDefinition::parse(&text)
        .unwrap()
        .to_entity(1, pelagic::math::Vec3::ZERO, 0.0);
    assert!(Session::new(&scene, &auv, sc, rules).is_err());
}
