//! Standard trials: each measured quantity is checked against the closed-form
//! value for the model or against a physical invariant.
use pelagic::{
    definition::VehicleDefinition,
    entities::Entity,
    math::Vec3,
    trials::{Criteria, Environment, Setup, run},
};
use std::sync::atomic::AtomicBool;

fn example(file: &str) -> Entity {
    let path = format!("examples/vehicles/{file}.pelagic-vehicle.json");
    VehicleDefinition::parse(&std::fs::read_to_string(path).unwrap())
        .unwrap()
        .to_entity(2, Vec3::ZERO, 0.0)
}

fn only(edit: impl FnOnce(&mut Setup)) -> Setup {
    let mut s = Setup {
        free_decay: false,
        speed: false,
        turning: false,
        zigzag: false,
        stopping: false,
        waves: false,
        ..Setup::quick()
    };
    edit(&mut s);
    s
}

fn trials(e: &Entity, setup: &Setup, criteria: &Criteria) -> pelagic::trials::Report {
    run(
        e,
        &Environment::default(),
        setup,
        criteria,
        4,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap()
}

#[test]
fn free_decay_periods_match_the_closed_form_values() {
    // Submerged AUV: T = 2π·√((I + added inertia)/(m·g·BG)).
    let auv = trials(
        &example("survey-auv"),
        &only(|s| s.free_decay = true),
        &Criteria::default(),
    );
    assert_eq!(auv.mode, "submerged");
    for d in &auv.decay {
        let (t, p) = (d.period_s.unwrap(), d.predicted_period_s.unwrap());
        // Lightly damped: the damped period is within 1 % of the undamped.
        assert!((t / p - 1.0).abs() < 0.015, "{} {t} {p}", d.axis);
        assert!(d.damping_ratio.unwrap() < 0.15);
        // The derived BG recovers the definition's centre of buoyancy.
        assert!(
            (d.metacentric_height_m.unwrap() - 0.02).abs() < 0.002,
            "{:?}",
            d.metacentric_height_m
        );
    }
    // Surface vessel heave: k = ρ·g·V/(2·half.z), ζ = 0.12 plus drag.
    let boat = trials(
        &example("workboat-usv"),
        &only(|s| s.free_decay = true),
        &Criteria::default(),
    );
    let heave = boat.decay.iter().find(|d| d.axis == "heave").unwrap();
    let zeta = heave.damping_ratio.unwrap();
    let undamped = heave.period_s.unwrap() * (1.0 - zeta * zeta).sqrt();
    assert!(
        (undamped / heave.predicted_period_s.unwrap() - 1.0).abs() < 0.02,
        "{heave:?}"
    );
    assert!((0.11..0.2).contains(&zeta), "{zeta}");
}

#[test]
fn speed_follows_the_drag_law_and_thrusters_below_the_centre_of_mass_give_full_thrust() {
    let auv = trials(
        &example("survey-auv"),
        &only(|s| s.speed = true),
        &Criteria::default(),
    );
    let top = auv.speed.last().unwrap();
    // Steady speed √(T / drag_x): 60 N against 12.4 N/(m/s)².
    assert!(
        (top.speed_mps / (60.0_f64 / 12.4).sqrt() - 1.0).abs() < 0.01,
        "{top:?}"
    );
    assert!(top.steady && top.max_tilt_deg < 10.0);
    // The workboat's propellers sit 0.3 m below its centre of mass; the
    // allocation must still deliver both thrusters' 2 kN.
    let boat = trials(
        &example("workboat-usv"),
        &only(|s| s.speed = true),
        &Criteria::default(),
    );
    assert!(
        (boat.forward_thrust_n - 4000.0).abs() < 40.0,
        "{}",
        boat.forward_thrust_n
    );
    assert!(
        boat.speed
            .windows(2)
            .all(|w| w[1].speed_mps > w[0].speed_mps)
    );
}

#[test]
fn turning_circles_are_symmetric_and_geometrically_consistent() {
    let r = trials(
        &example("workboat-usv"),
        &only(|s| s.turning = true),
        &Criteria::imo(6.0, 9.7),
    );
    assert_eq!(r.steering, "rudder");
    let (port, starboard) = (&r.turning[0], &r.turning[1]);
    assert_eq!(port.side, "port");
    assert!((port.advance_m.unwrap() - starboard.advance_m.unwrap()).abs() < 1e-6);
    let (advance, transfer, tactical) = (
        port.advance_m.unwrap(),
        port.transfer_m.unwrap(),
        port.tactical_diameter_m.unwrap(),
    );
    assert!(
        advance > 0.0 && transfer > 0.0 && tactical > transfer,
        "{port:?}"
    );
    // Steady circle: D = 2·U/r.
    let d = port.steady_diameter_m.unwrap();
    let kinematic =
        2.0 * port.steady_speed_mps.unwrap() / port.steady_yaw_rate_dps.unwrap().to_radians();
    assert!((d / kinematic - 1.0).abs() < 0.05, "{d} {kinematic}");
    assert_eq!(port.rudder_deg, Some(35.0));
    assert!(r.checks.iter().all(|c| c.passed), "{:?}", r.checks);
}

#[test]
fn zigzag_and_stopping_report_the_imo_quantities() {
    let r = trials(
        &example("survey-auv"),
        &only(|s| {
            s.zigzag = true;
            s.stopping = true;
            s.zigzag_angles = vec![10.0, 20.0];
        }),
        &Criteria::imo(2.2, 2.2),
    );
    for z in &r.zigzag {
        let first = z.first_overshoot_deg.unwrap();
        let second = z.second_overshoot_deg.unwrap();
        assert!(
            (0.0..10.0).contains(&first) && (0.0..10.0).contains(&second),
            "{z:?}"
        );
        assert!(z.period_s.unwrap() > z.initial_turning_time_s.unwrap());
        assert_eq!(z.rudder_deg, Some(z.angle_deg));
    }
    let stop = r.stopping.as_ref().unwrap();
    assert!(stop.track_reach_m.unwrap() > 0.0 && stop.astern_thrust_n > 0.0);
    assert!(r.passed(), "{}", r.summary());
    // IMO MSC.137(76) overshoot limits depend on L/V.
    let mid = Criteria::imo(100.0, 5.0);
    assert_eq!(mid.max_first_overshoot_deg, Some(15.0));
    assert_eq!(mid.max_second_overshoot_deg, Some(32.5));
    assert_eq!(Criteria::imo(6.0, 3.0).max_first_overshoot_deg, Some(10.0));
}

#[test]
fn long_waves_carry_a_surface_vessel_and_spare_a_deep_one() {
    let setup = only(|s| {
        s.waves = true;
        s.wave_periods = vec![12.0];
        s.wave_headings = vec![180.0];
    });
    let boat = trials(&example("workboat-usv"), &setup, &Criteria::default());
    let w = &boat.waves[0];
    // A 225 m wave lifts a 6 m boat bodily: heave RAO → 1, pitch → slope.
    assert!((w.heave_rao - 1.0).abs() < 0.1, "{w:?}");
    assert!((w.pitch_rao - 1.0).abs() < 0.25, "{w:?}");
    assert!(w.roll_deg < 0.05, "head seas make no roll: {w:?}");
    let auv = trials(&example("survey-auv"), &setup, &Criteria::default());
    assert!(auv.waves[0].heave_rao < 0.3, "{:?}", auv.waves[0]);
}

#[test]
fn trials_are_deterministic_and_independent_of_thread_count() {
    let e = example("workboat-usv");
    let setup = Setup {
        speed_levels: vec![1.0],
        zigzag_angles: vec![10.0],
        wave_periods: vec![4.0],
        ..Setup::quick()
    };
    let go = |threads| {
        run(
            &e,
            &Environment::default(),
            &setup,
            &Criteria::default(),
            threads,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap()
    };
    let (a, b) = (go(1), go(6));
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
    // The report round-trips through JSON.
    let text = serde_json::to_string(&a).unwrap();
    let back: pelagic::trials::Report = serde_json::from_str(&text).unwrap();
    assert_eq!(back, a);
}

#[test]
fn trials_that_cannot_run_are_skipped_not_failed() {
    let r = trials(
        &Entity::drifter(4),
        &only(|s| {
            s.turning = true;
            s.free_decay = true;
        }),
        &Criteria::imo(0.24, 1.0),
    );
    assert!(r.turning.is_empty() && !r.skipped.is_empty());
    assert!(r.checks.is_empty(), "{:?}", r.checks);
    // The default drifter is a uniform tall block: it floats on its side.
    assert!(r.warnings.iter().any(|w| w.contains("not stable upright")));
    // Legacy coefficients are flagged.
    assert!(r.warnings.iter().any(|w| w.contains("legacy")));
    // A surface vessel that cannot float is an error with a reason.
    let mut sinker = Entity::boat(3);
    sinker.volume = 0.1;
    let e = run(
        &sinker,
        &Environment::default(),
        &Setup::quick(),
        &Criteria::default(),
        1,
        &AtomicBool::new(false),
        |_, _| {},
    )
    .unwrap_err();
    assert!(e.contains("does not float"), "{e}");
}

#[test]
fn added_mass_and_fins_never_create_energy() {
    // A finned AUV coasting while spinning and sliding sideways: with no
    // thrust, its kinetic energy (rigid body plus added mass) must fall.
    let mut e = example("survey-auv");
    e.model.hold_depth = None;
    e.model.hold_heading = None;
    e.volume = e.mass / 1023.9;
    e.model.profile_density = false;
    let mut scene = pelagic::scene::Scene {
        terrain: 0,
        wave_height: 0.0,
        current: [0.0; 3],
        structure: false,
        particles: 0,
        vehicle: false,
        density: 1023.9,
        bottom_depth: 200.0,
        school: pelagic::life::SchoolConfig {
            count: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    e.position = Vec3::new(0.0, 0.0, -50.0);
    scene.entities.push(e.clone());
    let mut world = pelagic::scene::world(&scene).unwrap();
    {
        let v = world.vehicle_mut(0).unwrap();
        v.velocity = Vec3::new(2.0, 1.0, 0.0);
        v.angular_velocity = Vec3::new(0.0, 0.0, 1.5);
    }
    let inertia = e.effective_inertia() + e.model.added_inertia;
    let energy = |w: &pelagic::World| {
        let v = &w.vehicles()[0];
        let b = v.orientation.conjugate().rotate(v.velocity);
        let m = e.model.added_mass + Vec3::new(e.mass, e.mass, e.mass);
        let o = v.angular_velocity;
        0.5 * (m.x * b.x * b.x + m.y * b.y * b.y + m.z * b.z * b.z)
            + 0.5 * (inertia.x * o.x * o.x + inertia.y * o.y * o.y + inertia.z * o.z * o.z)
    };
    let mut last = energy(&world);
    for _ in 0..1200 {
        world.step();
        let now = energy(&world);
        assert!(now <= last * (1.0 + 1e-3) + 1e-9, "{last} -> {now}");
        last = now;
    }
    assert!(last < 1.0, "the body comes to rest: {last}");
}
