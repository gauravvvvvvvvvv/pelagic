//! Editor operations on the project asset library. Pure scene → scene
//! functions so every change goes through `editor::edit` (validation, undo,
//! recovery, save) and can be tested without a window.
use crate::project::Scene;
use pelagic::{
    assets::{Material, Mesh},
    entities::Entity,
    math::Vec3,
};
use std::sync::Arc;

/// A Content Browser selection. Presets are built-in materials not yet copied
/// into the project; they are identified by their preset index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Mesh(u64),
    Material(u64),
    Preset(usize),
}

/// Details categories for a selected actor.
pub const ENTITY_TABS: [&str; 6] = [
    "Transform",
    "Body / actuators",
    "Controller",
    "Dimensions",
    "Appearance",
    "Material surface",
];

/// First UI ID of library mesh cards (by library index, up to 32).
pub const MESH_CARD: usize = 448;
/// First UI ID of material cards (project materials, then unused presets).
pub const MATERIAL_CARD: usize = 480;

/// Project materials followed by built-in presets that are not yet in the
/// project (matched by name). At most 31 entries.
pub fn material_items(scene: &Scene) -> Vec<(Item, Material)> {
    let mut items: Vec<_> = scene
        .library
        .materials
        .iter()
        .map(|m| (Item::Material(m.id), m.clone()))
        .collect();
    for (i, preset) in Material::presets().into_iter().enumerate() {
        if !scene
            .library
            .materials
            .iter()
            .any(|m| m.name == preset.name)
        {
            items.push((Item::Preset(i), preset));
        }
    }
    items
}

pub fn material(scene: &Scene, item: Item) -> Option<Material> {
    match item {
        Item::Material(id) => scene.library.material(id).cloned(),
        Item::Preset(i) => Material::presets().get(i).cloned(),
        Item::Mesh(_) => None,
    }
}

pub fn name(scene: &Scene, item: Item) -> String {
    match item {
        Item::Mesh(id) => scene
            .library
            .mesh(id)
            .map_or("Missing mesh".into(), |m| m.name.clone()),
        _ => material(scene, item).map_or("Missing material".into(), |m| m.name),
    }
}

/// Number of actors (reference included) using a library item.
pub fn usage(scene: &Scene, item: Item) -> usize {
    let mut handles = vec![(scene.reference_asset, scene.reference_material)];
    handles.extend(scene.entities.iter().map(|e| (e.asset, e.material)));
    handles
        .iter()
        .filter(|(a, m)| match item {
            Item::Mesh(id) => *a == Some(id),
            Item::Material(id) => *m == Some(id),
            Item::Preset(_) => false,
        })
        .count()
}

fn actor_mut(scene: &mut Scene, actor: Option<usize>) -> Result<ActorRef<'_>, String> {
    match actor {
        None => Ok(ActorRef::Reference(scene)),
        Some(i) => scene
            .entities
            .get_mut(i)
            .map(ActorRef::Entity)
            .ok_or_else(|| "The selected actor no longer exists.".into()),
    }
}
enum ActorRef<'a> {
    Reference(&'a mut Scene),
    Entity(&'a mut Entity),
}
impl ActorRef<'_> {
    fn set_asset(&mut self, asset: Option<u64>) {
        match self {
            Self::Reference(s) => s.reference_asset = asset,
            Self::Entity(e) => e.asset = asset,
        }
    }
    fn set_material(&mut self, material: Option<u64>) {
        match self {
            Self::Reference(s) => s.reference_material = material,
            Self::Entity(e) => e.material = material,
        }
    }
}

/// Resolves a material item to a project handle, copying a preset in first.
fn project_material(scene: &mut Scene, item: Item) -> Result<u64, String> {
    match item {
        Item::Material(id) => scene
            .library
            .material(id)
            .map(|m| m.id)
            .ok_or_else(|| "That material is no longer in the library.".into()),
        Item::Preset(i) => {
            let preset = Material::presets()
                .get(i)
                .cloned()
                .ok_or("Unknown preset.")?;
            scene.library.add_material(&preset)
        }
        Item::Mesh(_) => Err("Select a material.".into()),
    }
}

/// Assigns a mesh or material to an actor (`None` = reference vehicle).
/// Physical properties are unchanged: a mesh is drawn inside the actor's
/// declared body box, and contact keeps using that box.
pub fn assign(scene: &Scene, actor: Option<usize>, item: Item) -> Result<Scene, String> {
    let mut next = scene.clone();
    match item {
        Item::Mesh(id) => {
            if next.library.mesh(id).is_none() {
                return Err("That mesh is no longer in the library.".into());
            }
            actor_mut(&mut next, actor)?.set_asset(Some(id));
        }
        _ => {
            let id = project_material(&mut next, item)?;
            actor_mut(&mut next, actor)?.set_material(Some(id));
        }
    }
    next.validate()?;
    Ok(next)
}

/// Clears an actor's mesh (procedural body) or material (default look).
pub fn clear(scene: &Scene, actor: Option<usize>, mesh: bool) -> Result<Scene, String> {
    let mut next = scene.clone();
    let mut target = actor_mut(&mut next, actor)?;
    if mesh {
        target.set_asset(None);
    } else {
        target.set_material(None);
    }
    next.validate()?;
    Ok(next)
}

fn unique_id(scene: &Scene) -> u64 {
    scene.entities.iter().map(|e| e.id).max().unwrap_or(0) + 1
}

/// Places a new body instance of a library mesh near `near` (normally the
/// camera target), clamped between the surface and the seabed. Returns the
/// new scene and the new entity index. The instance uses ROV physical
/// defaults; mass, volume and controller values remain editable in Details.
pub fn place(
    scene: &Scene,
    mesh: u64,
    material: Option<Item>,
    near: [f64; 3],
) -> Result<(Scene, usize), String> {
    let asset = scene
        .library
        .mesh(mesh)
        .ok_or("That mesh is no longer in the library.")?;
    if scene.entities.len() >= 15 {
        return Err("Scene supports 16 bodies including the reference vehicle.".into());
    }
    let mut next = scene.clone();
    let z = near[2].clamp(-(scene.bottom_depth - 1.0).max(0.6), -0.5);
    let mut position = Vec3::new(
        near[0].clamp(-250.0, 250.0),
        near[1].clamp(-250.0, 250.0),
        z,
    );
    // Offset from any body already at the same spot so instances stay pickable.
    let occupied = |p: Vec3| {
        let reference = Vec3::new(
            scene.reference_xy[0],
            scene.reference_xy[1],
            -scene.vehicle_depth,
        );
        (scene.vehicle && (reference - p).length() < 1.5)
            || scene
                .entities
                .iter()
                .any(|e| (e.position - p).length() < 1.5)
    };
    for _ in 0..16 {
        if !occupied(position) {
            break;
        }
        position.x += 2.0;
    }
    let mut entity = Entity::rov(unique_id(&next), position);
    entity.name = asset.name.chars().take(80).collect();
    entity.asset = Some(mesh);
    // Size the body box to the mesh's proportions (largest half-size 0.75 m),
    // moving buoyancy centre and thrusters with it like a resize would.
    let half = Vec3::new(
        (asset.proportions[0] * 0.75).max(0.05),
        (asset.proportions[1] * 0.75).max(0.05),
        (asset.proportions[2] * 0.75).max(0.05),
    );
    let old = entity.model.half;
    let factor = Vec3::new(half.x / old.x, half.y / old.y, half.z / old.z);
    entity.model.half = half;
    entity.model.center_buoyancy = entity.model.center_buoyancy.component_mul(factor);
    for t in &mut entity.model.thrusters {
        t.position = t.position.component_mul(factor);
    }
    if let Some(item) = material {
        entity.material = Some(project_material(&mut next, item)?);
    }
    next.entities.push(entity);
    next.validate()?;
    let index = next.entities.len() - 1;
    Ok((next, index))
}

/// Adds a vehicle from a definition near `near` (surface vessels float at
/// the surface), with its hull mesh added to the library when given.
/// Returns the new scene and the new entity index.
pub fn place_definition(
    scene: &Scene,
    definition: &pelagic::definition::VehicleDefinition,
    mesh: Option<(String, Mesh)>,
    near: [f64; 3],
) -> Result<(Scene, usize), String> {
    definition.validate()?;
    if scene.entities.len() >= 15 {
        return Err("Scene supports 16 bodies including the reference vehicle.".into());
    }
    let mut next = scene.clone();
    let asset = match mesh {
        Some((name, mut mesh)) => {
            mesh.validate().map_err(str::to_string)?;
            let proportions = mesh.fill_unit_box();
            Some(
                next.library
                    .add_mesh_with(&name, Arc::new(mesh), proportions)?,
            )
        }
        None => None,
    };
    let z = if definition.kind == 1 {
        0.0
    } else {
        near[2].clamp(-(scene.bottom_depth - 1.0).max(0.6), -0.5)
    };
    let mut position = Vec3::new(
        near[0].clamp(-250.0, 250.0),
        near[1].clamp(-250.0, 250.0),
        z,
    );
    let clearance = definition.model.half.length() * 2.0 + 0.5;
    for _ in 0..32 {
        if !scene
            .entities
            .iter()
            .any(|e| (e.position - position).length() < clearance)
        {
            break;
        }
        position.x += clearance;
    }
    let mut entity = definition.to_entity(unique_id(&next), position, 0.0);
    entity.asset = asset;
    next.entities.push(entity);
    next.validate()?;
    let index = next.entities.len() - 1;
    Ok((next, index))
}

/// The hull mesh of an entity as Wavefront OBJ text in metres (body frame),
/// or `None` for procedural bodies.
pub fn hull_obj(scene: &Scene, entity: &Entity) -> Option<String> {
    let asset = scene.library.mesh(entity.asset?)?;
    let h = entity.model.half;
    let mut text = format!(
        "# {} hull exported by Pelagic (metres, X forward, Y left, Z up)
",
        entity.name
    );
    for v in &asset.mesh.vertices {
        text += &format!(
            "v {:.6} {:.6} {:.6}
",
            v.x * h.x,
            v.y * h.y,
            v.z * h.z
        );
    }
    for t in &asset.mesh.triangles {
        text += &format!(
            "f {} {} {}
",
            t[0] + 1,
            t[1] + 1,
            t[2] + 1
        );
    }
    Some(text)
}

/// Removes an unused library item. Items still used by an actor are kept so
/// a removal never silently changes the scene's appearance.
pub fn remove(scene: &Scene, item: Item) -> Result<Scene, String> {
    let used = usage(scene, item);
    if used > 0 {
        return Err(format!(
            "{} is used by {used} actor{}; reassign or delete them first.",
            name(scene, item),
            if used == 1 { "" } else { "s" }
        ));
    }
    let mut next = scene.clone();
    match item {
        Item::Mesh(id) => next.library.meshes.retain(|m| m.id != id),
        Item::Material(id) => next.library.materials.retain(|m| m.id != id),
        Item::Preset(_) => return Err("Built-in presets cannot be removed.".into()),
    }
    next.validate()?;
    Ok(next)
}

/// Adds an imported mesh to the library and either assigns it to the selected
/// entity or places a new instance. Returns the scene, mesh handle and the
/// entity index that now shows the mesh.
pub fn import(
    scene: &Scene,
    name: &str,
    mut mesh: Mesh,
    selected: Option<usize>,
) -> Result<(Scene, u64, usize), String> {
    let mut next = scene.clone();
    let proportions = mesh.fill_unit_box();
    let id = next
        .library
        .add_mesh_with(name, Arc::new(mesh), proportions)?;
    if let Some(index) = selected {
        next = assign(&next, Some(index), Item::Mesh(id))?;
        return Ok((next, id, index));
    }
    let near = [3.0, 0.0, -scene.vehicle_depth];
    let (next, index) = place(&next, id, None, near)?;
    Ok((next, id, index))
}

/// Edits one surface value of a project material (shared by every actor
/// that uses it). `channel`: 0–2 sRGB components, 3 roughness, 4 metallic.
pub fn set_material_value(
    scene: &Scene,
    id: u64,
    channel: usize,
    value: f64,
) -> Result<Scene, String> {
    let mut next = scene.clone();
    let m = next
        .library
        .material_mut(id)
        .ok_or("That material is no longer in the library.")?;
    match channel {
        0..=2 => {
            if !value.is_finite() || !(0.0..=255.0).contains(&value) {
                return Err("Colour components must be 0–255.".into());
            }
            m.base_color[channel] = value.round() as u8;
        }
        3 => m.roughness = value,
        4 => m.metallic = value,
        _ => return Err("Unknown material property.".into()),
    }
    next.validate()?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cube() -> Mesh {
        Mesh::parse_obj(
            "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv 0 0 1\nf 1 2 3 4\nf 1 2 5\nf 2 3 5\nf 3 4 5\nf 4 1 5",
            "pyramid.obj",
        )
        .unwrap()
    }
    #[test]
    fn vehicle_definitions_place_with_meshes_and_export_their_hull() {
        let scene = Scene::robotics_demo();
        let text =
            std::fs::read_to_string("../../examples/vehicles/workboat-usv.pelagic-vehicle.json")
                .unwrap();
        let definition = pelagic::definition::VehicleDefinition::parse(&text).unwrap();
        let (next, index) = place_definition(
            &scene,
            &definition,
            Some(("hull.obj".into(), cube())),
            [5.0, 5.0, -10.0],
        )
        .unwrap();
        let e = &next.entities[index];
        assert_eq!((e.kind, e.name.as_str()), (1, "Workboat USV (example)"));
        assert_eq!(e.position.z, 0.0, "surface vessels start at the surface");
        assert_eq!(e.model.drag, definition.model.drag);
        assert!(next.entities.iter().filter(|o| o.id == e.id).count() == 1);
        let obj = hull_obj(&next, e).unwrap();
        let again = Mesh::parse_obj(&obj, "again.obj").unwrap();
        assert_eq!(
            again.triangles.len(),
            next.library
                .mesh(e.asset.unwrap())
                .unwrap()
                .mesh
                .triangles
                .len()
        );
        // Parsing normalises meshes, so read the written coordinates directly.
        let extent = obj
            .lines()
            .filter_map(|l| l.strip_prefix("v "))
            .filter_map(|l| l.split_whitespace().next()?.parse::<f64>().ok())
            .fold(0.0f64, |m, x| m.max(x.abs()));
        assert!(
            (extent - e.model.half.x).abs() < 1e-5,
            "OBJ is in metres: {extent}"
        );
        let (plain, i) = place_definition(&scene, &definition, None, [0.0, 0.0, -5.0]).unwrap();
        assert!(
            plain.entities[i].asset.is_none() && hull_obj(&plain, &plain.entities[i]).is_none()
        );
    }
    #[test]
    fn import_place_assign_remove_flow_keeps_valid_scenes_and_handles() {
        let scene = Scene::robotics_demo();
        let (scene, mesh, index) = import(&scene, "pyramid.obj", cube(), None).unwrap();
        assert_eq!(scene.entities[index].asset, Some(mesh));
        assert_eq!(scene.entities[index].name, "pyramid.obj");
        // Re-importing identical geometry reuses the asset.
        let (scene, again, _) = import(&scene, "copy.obj", cube(), Some(0)).unwrap();
        assert_eq!(again, mesh);
        assert_eq!(scene.library.meshes.len(), 1);
        assert_eq!(scene.entities[0].asset, Some(mesh));
        // Place two more instances near the same point; they do not overlap.
        let (scene, a) = place(&scene, mesh, Some(Item::Preset(0)), [1.0, 1.0, -5.0]).unwrap();
        let (scene, b) = place(&scene, mesh, Some(Item::Preset(0)), [1.0, 1.0, -5.0]).unwrap();
        assert!((scene.entities[a].position - scene.entities[b].position).length() >= 1.5);
        assert_eq!(scene.library.materials.len(), 1, "preset copied once");
        let paint = scene.library.materials[0].id;
        assert_eq!(scene.entities[b].material, Some(paint));
        // Pyramid source is 2×2×2 → box keeps those proportions.
        let half = scene.entities[a].model.half;
        assert!((half.x - 0.75).abs() < 1e-9 && (half.z - 0.75).abs() < 1e-9);
        // Placement depth is clamped into the water column.
        let (deep, d) = place(&scene, mesh, None, [0.0, 0.0, -9999.0]).unwrap();
        assert!(deep.entities[d].position.z > -deep.bottom_depth);
        // Reference vehicle can be painted.
        let painted = assign(&scene, None, Item::Material(paint)).unwrap();
        assert_eq!(painted.reference_entity().material, Some(paint));
        assert_eq!(usage(&painted, Item::Material(paint)), 3);
        // Used items cannot be removed; cleared ones can.
        assert!(remove(&painted, Item::Mesh(mesh)).is_err());
        let mut cleared = painted.clone();
        for i in 0..cleared.entities.len() {
            cleared = clear(&cleared, Some(i), true).unwrap();
            cleared = clear(&cleared, Some(i), false).unwrap();
        }
        cleared = clear(&cleared, None, false).unwrap();
        let cleaned = remove(&cleared, Item::Mesh(mesh)).unwrap();
        let cleaned = remove(&cleaned, Item::Material(paint)).unwrap();
        assert!(cleaned.library.meshes.is_empty() && cleaned.library.materials.is_empty());
        assert!(remove(&cleaned, Item::Preset(0)).is_err());
        // Material values are validated and shared.
        let edited = set_material_value(&painted, paint, 0, 12.4).unwrap();
        assert_eq!(edited.library.materials[0].base_color[0], 12);
        assert!(set_material_value(&painted, paint, 3, 0.0).is_err());
        assert!(set_material_value(&painted, paint, 1, 300.0).is_err());
        // Presets disappear from the picker once copied in.
        let items = material_items(&painted);
        assert_eq!(items.len(), Material::presets().len());
        assert_eq!(items[0].0, Item::Material(paint));
        assert!(items.len() <= 32);
    }
    #[test]
    fn slender_imports_keep_their_shape_in_a_matching_body_box() {
        let hull = Mesh::parse_obj(
            "v -4 -0.5 -0.5
v 4 -0.5 -0.5
v 4 0.5 -0.5
v -4 0.5 0.5
f 1 2 3
f 1 3 4",
            "hull.obj",
        )
        .unwrap();
        let (scene, id, index) = import(&Scene::robotics_demo(), "hull.obj", hull, None).unwrap();
        let asset = scene.library.mesh(id).unwrap();
        assert!((asset.proportions[0] - 1.0).abs() < 1e-9);
        assert!((asset.proportions[1] - 0.125).abs() < 1e-9);
        let max = asset
            .mesh
            .vertices
            .iter()
            .fold(0.0f64, |m, v| m.max(v.y.abs()));
        assert!((max - 1.0).abs() < 1e-9, "stored mesh fills the unit box");
        let half = scene.entities[index].model.half;
        assert!(
            (half.x / half.y - 8.0).abs() < 1e-6,
            "box keeps 8:1 proportions"
        );
        scene.validate().unwrap();
    }
    #[test]
    fn demo_scene_is_valid_portable_and_builds_a_world_with_painted_meshes() {
        let scene = demo_scene();
        assert_eq!(scene.library.meshes.len(), 3);
        assert_eq!(scene.library.materials.len(), 5);
        let bytes = serde_json::to_vec(&scene.value()).unwrap();
        assert!(bytes.len() < 2_000_000, "demo stays small: {}", bytes.len());
        let reopened = Scene::from_value(&serde_json::from_slice(&bytes).unwrap()).unwrap();
        assert_eq!(reopened, scene);
        let world = pelagic::scene::world(&scene).unwrap();
        assert_eq!(world.ocean.assets.len(), 3);
        assert_eq!(
            world.ocean.materials.len(),
            5,
            "reference, boat and three assets"
        );
        // Every placed asset sits inside the water column.
        for e in &scene.entities {
            assert!(e.position.z - e.model.half.z >= -scene.bottom_depth - 1e-9);
            assert!(e.position.z <= 0.0);
        }
        let auv = scene
            .entities
            .iter()
            .find(|e| e.name == "Survey AUV hull")
            .unwrap();
        assert!(
            auv.model.half.x / auv.model.half.y > 3.0,
            "slender hull box (fin span sets width)"
        );
    }
    #[test]
    fn placing_beyond_the_body_budget_is_refused() {
        let (mut scene, mesh, _) = import(&Scene::robotics_demo(), "p", cube(), None).unwrap();
        while scene.entities.len() < 15 {
            scene = place(&scene, mesh, None, [0.0, 0.0, -5.0]).unwrap().0;
        }
        assert!(place(&scene, mesh, None, [0.0, 0.0, -5.0]).is_err());
    }
}

/// Small OBJ writer for the bundled demonstration assets. The demo goes
/// through the same `Mesh::parse_obj` path as a user's file.
#[derive(Default)]
struct Obj {
    v: Vec<[f64; 3]>,
    f: Vec<Vec<usize>>,
}
impl Obj {
    /// Surface of revolution: `profile` is (axial position, radius) pairs;
    /// `axis` 0 = X, 2 = Z. Ends with non-zero radius are capped.
    fn revolve(&mut self, profile: &[(f64, f64)], axis: usize, segments: usize, centre: [f64; 3]) {
        let base = self.v.len();
        for &(a, r) in profile {
            for s in 0..segments {
                let t = std::f64::consts::TAU * s as f64 / segments as f64;
                let (c, sn) = (t.cos() * r, t.sin() * r);
                let p = if axis == 0 { [a, c, sn] } else { [c, sn, a] };
                self.v
                    .push([p[0] + centre[0], p[1] + centre[1], p[2] + centre[2]]);
            }
        }
        for ring in 0..profile.len() - 1 {
            for s in 0..segments {
                let a = base + ring * segments + s;
                let b = base + ring * segments + (s + 1) % segments;
                self.f.push(vec![a, b, b + segments, a + segments]);
            }
        }
        for (ring, reverse) in [(0, true), (profile.len() - 1, false)] {
            if profile[ring].1 > 1e-6 {
                let mut cap: Vec<_> = (0..segments).map(|s| base + ring * segments + s).collect();
                if reverse {
                    cap.reverse();
                }
                self.f.push(cap);
            }
        }
    }
    /// Rectangular beam from `a` to `b` with half cross-section `t`.
    fn beam(&mut self, a: [f64; 3], b: [f64; 3], t: f64) {
        let d = Vec3::new(b[0] - a[0], b[1] - a[1], b[2] - a[2]).normalized();
        let helper = if d.z.abs() < 0.9 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        let u = d.cross(helper).normalized() * t;
        let w = d.cross(u);
        let base = self.v.len();
        for end in [a, b] {
            let end = Vec3::new(end[0], end[1], end[2]);
            for (su, sw) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = end + u * su + w * sw;
                self.v.push([p.x, p.y, p.z]);
            }
        }
        for face in [
            [0, 1, 2, 3],
            [4, 7, 6, 5],
            [0, 4, 5, 1],
            [1, 5, 6, 2],
            [2, 6, 7, 3],
            [3, 7, 4, 0],
        ] {
            self.f.push(face.iter().map(|i| base + i).collect());
        }
    }
    fn text(&self) -> String {
        let mut out = String::new();
        for v in &self.v {
            out.push_str(&format!("v {:.5} {:.5} {:.5}\n", v[0], v[1], v[2]));
        }
        for f in &self.f {
            out.push('f');
            for i in f {
                out.push_str(&format!(" {}", i + 1));
            }
            out.push('\n');
        }
        out
    }
}

/// OBJ text for the three bundled demonstration assets.
pub fn demo_assets() -> Vec<(&'static str, String)> {
    // Torpedo AUV: tapered tail, parallel body, rounded nose, cruciform fins.
    let mut auv = Obj::default();
    let mut profile = vec![(-1.0, 0.0), (-0.98, 0.035)];
    for i in 0..=6 {
        let x = -0.95 + 0.4 * i as f64 / 6.0;
        profile.push((x, 0.04 + 0.085 * (i as f64 / 6.0).powf(0.8)));
    }
    profile.push((0.55, 0.125));
    for i in 1..=8 {
        let t = i as f64 / 8.0;
        profile.push((0.55 + 0.45 * t, 0.125 * (1.0 - t * t).max(0.0).sqrt()));
    }
    auv.revolve(&profile, 0, 24, [0.0; 3]);
    for (y, z) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        auv.beam(
            [-0.86, y * 0.05, z * 0.05],
            [-0.80, y * 0.27, z * 0.27],
            0.012,
        );
        auv.beam(
            [-0.70, y * 0.09, z * 0.09],
            [-0.80, y * 0.27, z * 0.27],
            0.012,
        );
    }
    // Mooring buoy: rounded float, mast and light, ballast stem.
    let mut buoy = Obj::default();
    let mut float = vec![(-0.32, 0.0)];
    for i in 0..=10 {
        let t = std::f64::consts::PI * (i as f64 / 10.0 - 0.5);
        float.push((t.sin() * 0.3, 0.35 + t.cos() * 0.25));
    }
    float.push((0.32, 0.0));
    buoy.revolve(&float, 2, 28, [0.0; 3]);
    buoy.revolve(&[(0.25, 0.05), (0.85, 0.035)], 2, 12, [0.0; 3]);
    buoy.revolve(
        &[(0.85, 0.0), (0.86, 0.09), (1.0, 0.09), (1.01, 0.0)],
        2,
        12,
        [0.0; 3],
    );
    buoy.revolve(&[(-1.0, 0.09), (-0.3, 0.05)], 2, 12, [0.0; 3]);
    // Seabed lander: instrument canister, deck ring and three splayed legs.
    let mut lander = Obj::default();
    lander.revolve(&[(-0.15, 0.2), (0.75, 0.2), (0.82, 0.14)], 2, 20, [0.0; 3]);
    lander.revolve(&[(0.3, 0.62), (0.38, 0.62)], 2, 24, [0.0; 3]);
    for k in 0..3 {
        let t = std::f64::consts::TAU * k as f64 / 3.0 + 0.5;
        let top = [0.55 * t.cos(), 0.55 * t.sin(), 0.34];
        let foot = [0.95 * t.cos(), 0.95 * t.sin(), -0.95];
        lander.beam(top, foot, 0.035);
        lander.beam(
            [0.0, 0.0, 0.0],
            [0.6 * t.cos(), 0.6 * t.sin(), -0.45],
            0.025,
        );
        lander.revolve(
            &[(-1.0, 0.13), (-0.94, 0.13)],
            2,
            12,
            [foot[0], foot[1], 0.0],
        );
    }
    vec![
        ("Survey AUV hull", auv.text()),
        ("Mooring buoy", buoy.text()),
        ("Seabed lander", lander.text()),
    ]
}

fn passive(e: &mut Entity, mass: f64, volume: f64) {
    e.mass = mass;
    e.volume = volume;
    e.model.thrust_limit = 0.0;
    e.model.thrusters.clear();
    e.model.hold_depth = None;
    e.model.hold_heading = None;
}

/// Marine robotics lab with three library assets placed and painted, built
/// through the same import/place/assign operations the editor uses.
pub fn demo_scene() -> Scene {
    let mut scene = Scene::robotics_demo();
    scene.name = "Asset library demo".into();
    scene.camera = [-7.5, -11.0, -6.5];
    scene.target = [1.5, 0.5, -10.5];
    scene.quality = 1;
    // Shallow 15 m survey site so seabed, mid-water and surface assets share a view.
    scene.bottom_depth = 15.0;
    scene.vehicle_depth = 10.0;
    scene.body_model.hold_depth = Some(10.0);
    scene.school.center = Vec3::new(5.0, 2.0, -9.0);
    let mut ids = vec![];
    for (name, text) in demo_assets() {
        let mut mesh = Mesh::parse_obj(&text, name).expect("bundled demo mesh");
        let proportions = mesh.fill_unit_box();
        ids.push(
            scene
                .library
                .add_mesh_with(name, Arc::new(mesh), proportions)
                .expect("library capacity"),
        );
    }
    // AUV: 2.2 m torpedo in front of the camera.
    let (s, auv) = place(&scene, ids[0], Some(Item::Preset(0)), [-1.5, -3.5, -8.0]).unwrap();
    scene = s;
    {
        let e = &mut scene.entities[auv];
        let f = 1.1 / e.model.half.x;
        e.model.half = e.model.half * f;
        e.model.center_buoyancy = e.model.center_buoyancy * f;
        for t in &mut e.model.thrusters {
            t.position = t.position * f;
        }
        e.heading = 35.0;
        e.mass = 45.0;
        e.volume = 45.0 / 1025.0;
    }
    // Buoy floating at the surface.
    let (s, buoy) = place(&scene, ids[1], Some(Item::Preset(4)), [7.0, 3.0, -0.6]).unwrap();
    scene = s;
    passive(&mut scene.entities[buoy], 40.0, 0.08);
    // Lander resting on the seabed.
    let (s, lander) = place(&scene, ids[2], Some(Item::Preset(2)), [5.5, 7.5, -14.0]).unwrap();
    scene = s;
    let floor = -scene.bottom_depth;
    let e = &mut scene.entities[lander];
    e.position.z = floor + e.model.half.z + 0.05;
    passive(e, 300.0, 0.08);
    scene = assign(&scene, Some(0), Item::Preset(1)).expect("boat material");
    scene = assign(&scene, None, Item::Preset(3)).expect("reference material");
    scene.validate().expect("valid demo scene");
    scene
}
