use pelagic::{
    math::Vec3,
    render::SoftwareRenderer,
    render_scene::RenderScene,
    scene::{Scene, world},
};
#[test]
fn shared_visual_scene_preserves_truth_and_samples_the_physical_surface() {
    let scene = Scene::robotics_demo();
    let mut world = world(&scene).unwrap();
    for _ in 0..30 {
        world.step();
    }
    let before = world.checkpoint();
    let snapshot = world.snapshot();
    let visual = RenderScene::build(&world.ocean, &snapshot, 0, 0).unwrap();
    assert!(visual.opaque.len() > 10000 && visual.water.len() > 10000);
    assert!(visual.opaque.len().is_multiple_of(3) && visual.water.len().is_multiple_of(3));
    for vertex in visual.water.iter().step_by(97) {
        let p = vertex.position;
        let physical = world.ocean.surface(p[0] as f64, p[1] as f64, snapshot.time);
        assert!((p[2] as f64 - physical.height).abs() < 1e-4);
        assert!(vertex.normal.iter().all(|v| v.is_finite()));
    }
    let mut renderer = SoftwareRenderer::new(128, 80).unwrap();
    renderer.camera = Vec3::new(-4., -6., -9.);
    renderer.target = Vec3::new(0., 0., -12.);
    let frame = renderer.render_shared(&world.ocean, &snapshot, &visual);
    assert_eq!(frame.rgb.len(), 128 * 80 * 3);
    assert!(frame.rgb.iter().max() > frame.rgb.iter().min());
    assert_eq!(world.checkpoint(), before);
}
#[test]
fn visual_quality_changes_do_not_change_simulated_populations() {
    let world = world(&Scene::robotics_demo()).unwrap();
    let before = world.checkpoint();
    let state = world.snapshot();
    let low = RenderScene::build(&world.ocean, &state, 0, 0).unwrap();
    let high = RenderScene::build(&world.ocean, &state, 2, 0).unwrap();
    assert!(high.opaque.len() > low.opaque.len());
    assert!(high.water.len() > low.water.len());
    assert_eq!(before, world.checkpoint());
}
#[test]
fn static_layer_is_time_independent_and_layers_compose_the_full_scene() {
    let mut world = world(&Scene::robotics_demo()).unwrap();
    let early = RenderScene::build_static(&world.ocean, 1, 0);
    for _ in 0..40 {
        world.step();
    }
    let state = world.snapshot();
    let late = RenderScene::build_static(&world.ocean, 1, 0);
    assert!(early.water.is_empty() && !early.opaque.is_empty());
    assert_eq!(early.opaque.len(), late.opaque.len());
    assert!(
        early
            .opaque
            .iter()
            .zip(&late.opaque)
            .all(|(a, b)| a.position == b.position && a.albedo == b.albedo)
    );
    let dynamic = RenderScene::build_dynamic(&world.ocean, &state, 1).unwrap();
    let full = RenderScene::build(&world.ocean, &state, 1, 0).unwrap();
    assert_eq!(full.opaque.len(), late.opaque.len() + dynamic.opaque.len());
    assert_eq!(full.water.len(), dynamic.water.len());
    let tail = &full.opaque[late.opaque.len()..];
    assert!(
        tail.iter()
            .zip(&dynamic.opaque)
            .all(|(a, b)| a.position == b.position)
    );
}
#[test]
fn flat_water_matches_sampled_water_layout_and_bodies_layer_has_no_water() {
    let mut world = world(&Scene::robotics_demo()).unwrap();
    for _ in 0..10 {
        world.step();
    }
    let state = world.snapshot();
    for quality in 0..=2 {
        let flat = RenderScene::flat_water(quality);
        let sampled = RenderScene::build_dynamic(&world.ocean, &state, quality).unwrap();
        assert_eq!(flat.len(), sampled.water.len());
        assert!(flat.iter().zip(&sampled.water).all(|(f, s)| {
            f.position[0] == s.position[0]
                && f.position[1] == s.position[1]
                && f.position[2] == 0.
                && f.kind == pelagic::render_scene::WAVE_KIND
                && s.kind == 1.
        }));
    }
    let bodies = RenderScene::build_bodies(&world.ocean, &state).unwrap();
    let dynamic = RenderScene::build_dynamic(&world.ocean, &state, 1).unwrap();
    assert!(bodies.water.is_empty());
    assert_eq!(bodies.opaque.len(), dynamic.opaque.len());
}
