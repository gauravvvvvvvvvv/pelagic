use pelagic::{
    definition::VehicleDefinition,
    entities::Entity,
    math::Vec3,
    scene::{Scene, world},
};

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

fn run(scene: &Scene, steps: usize) -> pelagic::Snapshot {
    let mut w = world(scene).unwrap();
    for _ in 0..steps {
        w.step();
    }
    w.snapshot()
}

#[test]
fn exporting_and_reimporting_a_vehicle_preserves_its_simulation_exactly() {
    let mut original = Entity::rov(3, Vec3::new(1.0, 2.0, -8.0));
    original.force = Vec3::new(5.0, 0.0, 0.0);
    let text = VehicleDefinition::from_entity(&original).to_json().unwrap();
    let parsed = VehicleDefinition::parse(&text).unwrap();
    assert!(parsed.model.inertia.is_some() && parsed.model.drag.is_some());
    let mut copy = parsed.to_entity(3, original.position, original.heading);
    copy.force = original.force;
    let (mut a, mut b) = (calm(), calm());
    a.entities.push(original);
    b.entities.push(copy);
    assert_eq!(run(&a, 300), run(&b, 300));
}

#[test]
fn drag_and_inertia_overrides_change_the_dynamics() {
    // Physically scaled rotational damping keeps the hull from tumbling
    // under the (real) destabilising Munk moment at speed.
    let mut base = Entity::rov(3, Vec3::new(0.0, 0.0, -8.0));
    let estimate = pelagic::definition::estimate_hydrodynamics(
        base.model.half,
        base.mass,
        base.volume,
        1025.0,
        false,
    );
    base.model.angular_drag = Some(estimate.angular_drag);
    let mut slow = base.clone();
    slow.model.drag = Some(base.effective_drag() * 4.0);
    let (mut a, mut b) = (calm(), calm());
    for (scene, mut e) in [(&mut a, base), (&mut b, slow)] {
        e.force = Vec3::new(20.0, 0.0, 0.0);
        scene.entities.push(e);
    }
    let fast = run(&a, 600).vehicles[0].velocity.x;
    let dragged = run(&b, 600).vehicles[0].velocity.x;
    assert!(fast > 0.0 && dragged > 0.0);
    // Terminal speed scales with 1/sqrt(drag): about half with 4x drag.
    assert!((dragged / fast - 0.5).abs() < 0.1, "{fast} {dragged}");
}

#[test]
fn bad_definitions_are_rejected_with_reasons() {
    let good = VehicleDefinition::from_entity(&Entity::boat(1));
    assert!(good.validate().is_ok());
    let mut wrong = good.clone();
    wrong.format = "something-else".into();
    assert!(
        wrong
            .validate()
            .unwrap_err()
            .contains("Not a vehicle definition")
    );
    let mut newer = good.clone();
    newer.version = 99;
    assert!(newer.validate().unwrap_err().contains("newer"));
    let mut negative = good.clone();
    negative.model.inertia = Some(Vec3::new(-1.0, 1.0, 1.0));
    assert!(negative.validate().is_err());
    let mut heavy = good;
    heavy.mass = -5.0;
    assert!(heavy.validate().is_err());
    assert!(VehicleDefinition::parse("{not json").is_err());
}

#[test]
fn shipped_example_definitions_load_and_behave_plausibly() {
    for (file, kind) in [
        ("examples/vehicles/survey-auv.pelagic-vehicle.json", 0),
        ("examples/vehicles/workboat-usv.pelagic-vehicle.json", 1),
    ] {
        let d = VehicleDefinition::parse(&std::fs::read_to_string(file).unwrap()).unwrap();
        assert_eq!(d.kind, kind, "{file}");
        let mut scene = calm();
        let start = if kind == 1 {
            Vec3::new(0.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 0.0, -15.0)
        };
        scene.entities.push(d.to_entity(2, start, 0.0));
        let s = run(&scene, 1800);
        let v = &s.vehicles[0];
        assert!(v.position.is_finite(), "{file}");
        if kind == 1 {
            // The workboat floats near its design draft.
            assert!(
                v.position.z > -0.6 && v.position.z < 0.3,
                "{file} z {}",
                v.position.z
            );
        } else {
            // The AUV holds its 15 m depth.
            assert!(
                (v.position.z + 15.0).abs() < 0.5,
                "{file} z {}",
                v.position.z
            );
        }
    }
    // Net weight sign: the AUV is slightly buoyant.
    let auv = VehicleDefinition::parse(
        &std::fs::read_to_string("examples/vehicles/survey-auv.pelagic-vehicle.json").unwrap(),
    )
    .unwrap();
    let net = auv.net_weight(1025.0, 9.81);
    assert!(net < 0.0 && net > -10.0, "{net}");
}

#[test]
fn hull_estimates_match_hand_derived_coefficients() {
    // The example AUV's cross-flow added mass and drag were derived by hand
    // with the same formulas (2.2 m x 0.32 m cylinder approximated as a box).
    let e = pelagic::definition::estimate_hydrodynamics(
        Vec3::new(1.1, 0.16, 0.16),
        45.0,
        0.0444,
        1025.0,
        true,
    );
    assert!((e.added_mass.y - 181.3).abs() < 1.0, "{:?}", e.added_mass);
    assert!((e.added_mass.x - 4.55).abs() < 0.1);
    // Box cross-flow area (2.2 x 0.32) vs the 0.704 m2 used by hand.
    assert!((e.drag.y - 0.5 * 1025.0 * 2.2 * 0.32).abs() < 1e-9);
    // A 1.3 m ROV frame has rotational damping in the tens of N·m/(rad/s)².
    let rov = Entity::rov(1, Vec3::ZERO);
    let r = pelagic::definition::estimate_hydrodynamics(
        rov.model.half,
        rov.mass,
        rov.volume,
        1025.0,
        false,
    );
    assert!(
        r.angular_drag.y > 30.0 && r.angular_drag.y < 60.0,
        "{:?}",
        r.angular_drag
    );
    let mut model = rov.model.clone();
    r.apply(&mut model);
    assert!(model.validate().is_ok());
    assert_eq!(model.drag, Some(r.drag));
}
