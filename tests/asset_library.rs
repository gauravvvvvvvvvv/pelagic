//! Project asset library: stable handles, v4 → v5 migration, validation and
//! material use by both renderers.
use pelagic::{
    assets::{Library, MAX_MESH_ASSETS, Material, Mesh},
    entities::Entity,
    scene::{Scene, world},
};
use std::sync::Arc;

fn triangle(scale: f64) -> Arc<Mesh> {
    Arc::new(
        Mesh::parse_obj(
            &format!("v 0 0 0\nv {scale} 0 0\nv 0 1 0\nf 1 2 3\n"),
            "triangle.obj",
        )
        .unwrap(),
    )
}

#[test]
fn v4_embedded_meshes_migrate_into_a_shared_library() {
    let mesh = triangle(1.0);
    let mut scene = Scene::robotics_demo();
    scene.library = Library::default();
    let mut v4 = scene.value();
    v4["version"] = serde_json::json!(4);
    v4.as_object_mut().unwrap().remove("library");
    // Two entities with identical embedded geometry and one without.
    for i in 0..2 {
        let entities = v4["entities"].as_array_mut().unwrap();
        entities[i].as_object_mut().unwrap().remove("asset");
        entities[i].as_object_mut().unwrap().remove("material");
        entities[i]["mesh"] = serde_json::to_value(&*mesh).unwrap();
    }
    let migrated = Scene::from_value(&v4).unwrap();
    assert_eq!(migrated.library.meshes.len(), 1, "identical meshes dedupe");
    let id = migrated.library.meshes[0].id;
    assert_eq!(migrated.library.meshes[0].name, "triangle.obj");
    assert!(migrated.entities.iter().all(|e| e.asset == Some(id)));
    // Saving writes v5 and reopening is lossless.
    let v5 = migrated.value();
    assert_eq!(v5["version"], 5);
    assert!(v5["entities"][0].get("mesh").is_none());
    assert_eq!(Scene::from_value(&v5).unwrap(), migrated);
    // Unknown future versions are still refused.
    let mut future = v5.clone();
    future["version"] = serde_json::json!(6);
    assert!(Scene::from_value(&future).is_err());
}

#[test]
fn handles_roundtrip_and_dangling_or_invalid_entries_are_rejected() {
    let mut scene = Scene::robotics_demo();
    let mesh = scene.library.add_mesh("hull", triangle(2.0)).unwrap();
    let paint = scene.library.add_material(&Material::presets()[0]).unwrap();
    assert_eq!(
        scene.library.add_material(&Material::presets()[0]).unwrap(),
        paint,
        "presets are copied once"
    );
    scene.entities[0].asset = Some(mesh);
    scene.entities[0].material = Some(paint);
    scene.reference_material = Some(paint);
    scene.validate().unwrap();
    let bytes = serde_json::to_vec(&scene.value()).unwrap();
    let reopened = Scene::from_value(&serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(reopened, scene);
    assert_eq!(reopened.reference_entity().material, Some(paint));

    let mut dangling = scene.clone();
    dangling.entities[0].asset = Some(99);
    assert!(dangling.validate().unwrap_err().contains("mesh missing"));
    let mut dangling = scene.clone();
    dangling.reference_material = Some(42);
    assert!(
        dangling
            .validate()
            .unwrap_err()
            .contains("material missing")
    );
    let mut bad = scene.clone();
    bad.library.materials[0].roughness = f64::NAN;
    assert!(bad.validate().is_err());
    let mut bad = scene.clone();
    bad.library.meshes.push(bad.library.meshes[0].clone());
    assert!(bad.validate().is_err(), "duplicate mesh IDs");
    let mut bad_json = scene.value();
    bad_json["library"]["materials"][0]["metallic"] = serde_json::json!(3.0);
    assert!(Scene::from_value(&bad_json).is_err());

    let mut full = Library::default();
    for i in 0..MAX_MESH_ASSETS {
        full.add_mesh("m", triangle(1.0 + i as f64)).unwrap();
    }
    assert!(full.add_mesh("one too many", triangle(100.0)).is_err());
    // Ids stay stable when an earlier asset is removed.
    full.meshes.remove(0);
    assert_eq!(full.meshes[0].id, 2);
}

#[test]
fn materials_reach_world_bodies_and_both_renderers_without_touching_physics() {
    let mut scene = Scene::robotics_demo();
    let plain = world(&scene).unwrap();
    let orange = scene
        .library
        .add_material(&Material::new(0, "Test orange", [250, 90, 10], 0.3, 0.7))
        .unwrap();
    let mesh = scene.library.add_mesh("hull", triangle(1.0)).unwrap();
    scene.reference_material = Some(orange);
    scene.reference_asset = Some(mesh);
    let id = scene.entities[1].id;
    scene.entities[1].material = Some(orange);
    let w = world(&scene).unwrap();
    assert_eq!(w.ocean.materials[&0].name, "Test orange");
    assert_eq!(w.ocean.materials[&id].base_color, [250, 90, 10]);
    assert!(w.ocean.assets.contains_key(&0));
    let snap = w.snapshot();
    let reference = snap.vehicles.iter().find(|v| v.id == 0).unwrap();
    assert_eq!(reference.asset, Some(0));
    // Same physical state as the unpainted scene apart from the visual mesh handle.
    let base = plain.snapshot();
    for (a, b) in snap.vehicles.iter().zip(&base.vehicles) {
        assert_eq!(
            (a.position, a.orientation, a.half),
            (b.position, b.orientation, b.half)
        );
    }
    let gpu = pelagic::render_scene::RenderScene::build(&w.ocean, &snap, 0, 0).unwrap();
    let linear = Material::new(1, "x", [250, 90, 10], 0.3, 0.7).linear();
    assert!(gpu.opaque.iter().any(|v| v.albedo == linear
        && (v.roughness - 0.3).abs() < 1e-6
        && (v.metallic - 0.7).abs() < 1e-6));
    let _ = Entity::rov(3, pelagic::math::Vec3::ZERO);
}
