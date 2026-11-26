use pelagic::{fields::CurrentGrid, math::Vec3, ocean::CurrentField, waves::SeaState};
#[test]
fn current_grid_reconstructs_affine_space_time_and_clamps_edges() {
    let mut values = vec![];
    for t in [0.0, 10.0] {
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    values.push(Vec3::new(
                        x as f64 + t,
                        2.0 * y as f64 - t,
                        z as f64 + 0.5 * t,
                    ));
                }
            }
        }
    }
    let g = CurrentGrid::new(
        [2, 2, 2],
        Vec3::ZERO,
        Vec3::new(1.0, 1.0, 1.0),
        vec![0.0, 10.0],
        values.clone(),
    )
    .unwrap();
    let v = g.velocity(Vec3::new(0.2, 0.7, 0.4), 3.0);
    assert!((v - Vec3::new(3.2, -1.6, 1.9)).length() < 1e-12);
    assert!(
        (g.velocity(Vec3::new(-3.0, 5.0, 3.0), 50.0) - Vec3::new(10.0, -8.0, 6.0)).length() < 1e-12
    );
    assert!(
        CurrentGrid::new(
            [2, 2, 2],
            Vec3::ZERO,
            Vec3::new(1.0, 1.0, 1.0),
            vec![10.0, 0.0],
            values
        )
        .is_err()
    );
}
#[test]
fn regular_wave_has_declared_height_period_and_direction() {
    let w = SeaState::regular(2.0, 6.0, 0.0, 40.0).unwrap();
    assert!((w.sample(0.0, 0.0, 0.0).height - 1.0).abs() < 1e-12);
    assert!((w.sample(0.0, 0.0, 3.0).height + 1.0).abs() < 1e-12);
    assert!((w.sample(0.0, 123.0, 6.0).height - 1.0).abs() < 1e-12);
}
