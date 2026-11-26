//! Read-only viewport queries and bounded authoring transactions.
use crate::{project::Scene, view::Model};
use pelagic::{
    Snapshot,
    math::{Quat, Vec3},
    ocean::Ocean,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    Reference,
    Entity(u64),
    Structure,
}
impl Actor {
    pub fn selected(model: &Model) -> Option<Self> {
        if model.selection_empty {
            return None;
        }
        let scene = &model.document.as_ref()?.scene;
        if model.inspector_page == 1 {
            match model.selected_entity {
                Some(i) => scene.entities.get(i).map(|e| Self::Entity(e.id)),
                None if scene.vehicle => Some(Self::Reference),
                _ => None,
            }
        } else if model.inspect_structure && scene.structure && model.inspector_page == 0 {
            Some(Self::Structure)
        } else {
            None
        }
    }
    pub fn entity(self, scene: &Scene) -> Option<pelagic::entities::Entity> {
        match self {
            Self::Reference if scene.vehicle => Some(scene.reference_entity()),
            Self::Entity(id) => scene.entities.iter().find(|e| e.id == id).cloned(),
            _ => None,
        }
    }
    pub fn position(self, scene: &Scene) -> Option<Vec3> {
        if self == Self::Structure {
            scene.structure.then_some(Vec3::new(
                scene.structure_position[0],
                scene.structure_position[1],
                scene.structure_position[2],
            ))
        } else {
            self.entity(scene).map(|e| e.position)
        }
    }
    pub fn runtime_id(self) -> Option<u64> {
        match self {
            Self::Reference => Some(0),
            Self::Entity(id) => Some(id),
            Self::Structure => None,
        }
    }
    pub fn name(self, scene: &Scene) -> String {
        if self == Self::Structure {
            "Harbour structure".into()
        } else {
            self.entity(scene)
                .map_or_else(|| "Missing actor".into(), |e| e.name)
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Move,
    Rotate,
    Size,
}
impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "Move",
            Self::Rotate => "Rotate Z",
            Self::Size => "Size",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Snap {
    #[default]
    Off,
    Fine,
    Normal,
    Coarse,
}
impl Snap {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Snap: Off",
            Self::Fine => "Snap: Fine",
            Self::Normal => "Snap: Med",
            Self::Coarse => "Snap: Coarse",
        }
    }
    pub fn step(self, tool: Tool) -> f64 {
        match (self, tool) {
            (Self::Off, _) => 0.,
            (Self::Fine, Tool::Move) => 0.1,
            (Self::Normal, Tool::Move) => 0.5,
            (Self::Coarse, Tool::Move) => 1.,
            (Self::Fine, Tool::Rotate) => 5.,
            (Self::Normal, Tool::Rotate) => 15.,
            (Self::Coarse, Tool::Rotate) => 45.,
            (Self::Fine, Tool::Size) => 0.05,
            (Self::Normal, Tool::Size) => 0.1,
            (Self::Coarse, Tool::Size) => 0.25,
        }
    }
    pub fn apply(self, tool: Tool, value: f64) -> f64 {
        let step = self.step(tool);
        if step > 0. {
            (value / step).round() * step
        } else {
            value
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    X,
    Y,
    Z,
    Heading,
    Uniform,
}
impl Handle {
    pub fn id(self) -> usize {
        match self {
            Self::X => 290,
            Self::Y => 291,
            Self::Z => 292,
            Self::Heading => 293,
            Self::Uniform => 294,
        }
    }
    pub fn from_id(id: usize) -> Option<Self> {
        match id {
            290 => Some(Self::X),
            291 => Some(Self::Y),
            292 => Some(Self::Z),
            293 => Some(Self::Heading),
            294 => Some(Self::Uniform),
            _ => None,
        }
    }
    pub fn axis(self) -> Vec3 {
        match self {
            Self::X => Vec3::new(1., 0., 0.),
            Self::Y => Vec3::new(0., 1., 0.),
            _ => Vec3::new(0., 0., 1.),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::X => "Move X axis",
            Self::Y => "Move Y axis",
            Self::Z => "Move Z axis",
            Self::Heading => "Rotate heading",
            Self::Uniform => "Resize uniformly",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub rect: [f32; 4],
    pub eye: Vec3,
    pub forward: Vec3,
    right: Vec3,
    up: Vec3,
    pub focal: f64,
}
impl Camera {
    pub fn new(rect: [f32; 4], eye: Vec3, target: Vec3, up: Vec3, fov: f64) -> Self {
        let forward = (target - eye).normalized();
        let mut right = forward.cross(up).normalized();
        if right.length() < 0.01 {
            right = Vec3::new(1., 0., 0.);
        }
        Self {
            rect,
            eye,
            forward,
            right,
            up: right.cross(forward),
            focal: rect[3] as f64 / (2. * (fov.to_radians() * 0.5).tan()),
        }
    }
    pub fn contains(self, p: [f32; 2]) -> bool {
        p[0] >= self.rect[0]
            && p[0] < self.rect[0] + self.rect[2]
            && p[1] >= self.rect[1]
            && p[1] < self.rect[1] + self.rect[3]
    }
    pub fn ray(self, p: [f32; 2]) -> Vec3 {
        let x = (p[0] - self.rect[0] - self.rect[2] * 0.5) as f64 / self.focal;
        let y = (self.rect[1] + self.rect[3] * 0.5 - p[1]) as f64 / self.focal;
        (self.forward + self.right * x + self.up * y).normalized()
    }
    pub fn project(self, p: Vec3) -> Option<[f32; 2]> {
        let d = p - self.eye;
        let z = d.dot(self.forward);
        (z > 0.1).then(|| {
            [
                self.rect[0] + self.rect[2] * 0.5 + (d.dot(self.right) / z * self.focal) as f32,
                self.rect[1] + self.rect[3] * 0.5 - (d.dot(self.up) / z * self.focal) as f32,
            ]
        })
    }
    pub fn world_size(self, p: Vec3) -> f64 {
        (p - self.eye).dot(self.forward) * 70. / self.focal
    }
    pub fn axis_parameter(self, p: [f32; 2], pivot: Vec3, axis: Vec3) -> Option<f64> {
        let d = self.ray(p);
        let c = axis.dot(d);
        let denominator = 1. - c * c;
        if denominator < 1e-5 {
            return None;
        }
        let w = self.eye - pivot;
        Some((axis.dot(w) - c * d.dot(w)) / denominator)
    }
    fn plane_angle(self, p: [f32; 2], pivot: Vec3) -> Option<f64> {
        let d = self.ray(p);
        if d.z.abs() < 1e-6 {
            return None;
        }
        let t = (pivot.z - self.eye.z) / d.z;
        if t <= 0. {
            return None;
        }
        let offset = self.eye + d * t - pivot;
        (offset.x.hypot(offset.y) > 1e-6).then(|| offset.y.atan2(offset.x))
    }
}
fn box_hit(eye: Vec3, ray: Vec3, position: Vec3, orientation: Quat, half: Vec3) -> Option<f64> {
    let q = orientation.conjugate();
    pelagic::geometry::StaticBox {
        center: Vec3::ZERO,
        half,
        color: [0.; 3],
    }
    .ray_distance(q.rotate(eye - position), q.rotate(ray))
}
fn triangle_hit(eye: Vec3, ray: Vec3, v: [Vec3; 3]) -> Option<f64> {
    let e1 = v[1] - v[0];
    let e2 = v[2] - v[0];
    let h = ray.cross(e2);
    let determinant = e1.dot(h);
    if determinant.abs() < 1e-10 {
        return None;
    }
    let s = eye - v[0];
    let u = s.dot(h) / determinant;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = ray.dot(q) / determinant;
    if v < 0. || u + v > 1. {
        return None;
    }
    let t = e2.dot(q) / determinant;
    (t >= 0.1).then_some(t)
}
pub fn pick(camera: Camera, point: [f32; 2], state: &Snapshot, ocean: &Ocean) -> Option<Actor> {
    if !camera.contains(point) {
        return None;
    }
    let ray = camera.ray(point);
    let mut nearest: Option<(f64, Actor)> = None;
    for body in &state.vehicles {
        if box_hit(camera.eye, ray, body.position, body.orientation, body.half).is_none() {
            continue;
        }
        let distance = if let Some(mesh) = body.asset.and_then(|id| ocean.assets.get(&id)) {
            mesh.triangles
                .iter()
                .filter_map(|face| {
                    triangle_hit(
                        camera.eye,
                        ray,
                        face.map(|i| {
                            body.position
                                + body
                                    .orientation
                                    .rotate(mesh.vertices[i].component_mul(body.half))
                        }),
                    )
                })
                .min_by(f64::total_cmp)
        } else {
            box_hit(camera.eye, ray, body.position, body.orientation, body.half)
        };
        if let Some(t) = distance.filter(|t| *t >= 0.1 && nearest.is_none_or(|n| *t < n.0)) {
            nearest = Some((
                t,
                if body.id == 0 {
                    Actor::Reference
                } else {
                    Actor::Entity(body.id)
                },
            ));
        }
    }
    for structure in &ocean.structures {
        if let Some(t) = structure
            .ray_distance(camera.eye, ray)
            .filter(|t| *t >= 0.1 && nearest.is_none_or(|n| *t < n.0))
        {
            nearest = Some((t, Actor::Structure));
        }
    }
    let (distance, actor) = nearest?;
    if camera.eye.z <= ocean.bottom.elevation(camera.eye.x, camera.eye.y) {
        return None;
    }
    if pelagic::sensor::bottom_echo(
        ocean,
        camera.eye,
        ray,
        distance,
        (distance / 1024.).max(0.02),
    )
    .is_some_and(|hit| hit.range + 0.02 < distance)
    {
        return None;
    }
    Some(actor)
}
pub fn select(model: &mut Model, actor: Option<Actor>, outliner_height: f32) {
    model.selection_empty = actor.is_none();
    model.inspect_structure = false;
    model.inspect_ocean = false;
    model.selected_entity = None;
    if let Some(actor) = actor {
        model.outliner_filter.clear();
        model.tool_tab = 0;
        let scene = &model.document.as_ref().unwrap().scene;
        let row = match actor {
            Actor::Reference => {
                model.inspector_page = 1;
                4
            }
            Actor::Entity(id) => {
                model.inspector_page = 1;
                model.selected_entity = scene.entities.iter().position(|e| e.id == id);
                4 + usize::from(scene.vehicle) + model.selected_entity.unwrap_or(0)
            }
            Actor::Structure => {
                model.inspector_page = 0;
                model.inspect_structure = true;
                4 + usize::from(scene.vehicle) + scene.entities.len()
            }
        };
        let rows = ((outliner_height - 106.) / 21.).floor().max(1.) as usize;
        model.outliner_scroll = row.saturating_sub(rows / 2);
    } else {
        model.inspector_page = 0;
    }
}
pub fn can_edit(model: &Model) -> bool {
    !model.job_running
        && !model.importing
        && model.session.as_ref().is_some_and(|s| {
            !s.running && s.world.tick() == 0 && s.display.is_none() && s.capture.is_none()
        })
        && model.playback.is_none()
        && matches!(model.viewport_mode, 0 | 1)
}
pub fn unavailable_reason(model: &Model) -> &'static str {
    if model.job_running || model.importing {
        "Finish or cancel the current task before editing transforms."
    } else {
        "Reset the preview before editing initial transforms."
    }
}
pub fn resize_body(scene: &mut Scene, actor: Actor, factor: Vec3) -> Result<(), String> {
    let mut e = actor.entity(scene).ok_or("No editable body selected.")?;
    e.model.half = e.model.half.component_mul(factor);
    e.model.center_buoyancy = e.model.center_buoyancy.component_mul(factor);
    for t in &mut e.model.thrusters {
        t.position = t.position.component_mul(factor);
    }
    e.validate()
        .map_err(|e| format!("Body dimensions exceed supported bounds: {e}"))?;
    if scene.sensors.entity == actor.runtime_id().unwrap() {
        scene.sensors.mount = scene.sensors.mount.component_mul(factor);
    }
    put_entity(scene, actor, e)?;
    scene.validate()
}
fn put_entity(
    scene: &mut Scene,
    actor: Actor,
    entity: pelagic::entities::Entity,
) -> Result<(), String> {
    match actor {
        Actor::Reference => scene.set_reference(&entity),
        Actor::Entity(id) => {
            *scene
                .entities
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or("Actor was removed.")? = entity
        }
        Actor::Structure => return Err("Harbour has no body model.".into()),
    }
    Ok(())
}
pub fn transformed(
    base: &Scene,
    actor: Actor,
    handle: Handle,
    amount: f64,
) -> Result<Scene, String> {
    if !amount.is_finite() {
        return Err("Invalid transform.".into());
    }
    if handle == Handle::Heading && amount.abs() < 1e-10 {
        return Ok(base.clone());
    }
    let mut scene = base.clone();
    if actor == Actor::Structure {
        match handle {
            Handle::X => scene.structure_position[0] += amount,
            Handle::Y => scene.structure_position[1] += amount,
            Handle::Z => scene.structure_position[2] += amount,
            Handle::Uniform => scene.structure_scale *= amount,
            Handle::Heading => {
                return Err("Harbour heading is not supported by this scene format.".into());
            }
        }
    } else if handle == Handle::Uniform {
        resize_body(&mut scene, actor, Vec3::new(amount, amount, amount))?;
    } else {
        let mut e = actor.entity(&scene).ok_or("Actor was removed.")?;
        if handle == Handle::Heading {
            e.heading = (e.heading + amount).rem_euclid(360.);
        } else {
            e.position = e.position + handle.axis() * amount;
        }
        put_entity(&mut scene, actor, e)?;
    }
    scene.validate()?;
    Ok(scene)
}
pub struct Drag {
    pub base: Scene,
    pub actor: Actor,
    pub handle: Handle,
    camera: Camera,
    pivot: Vec3,
    start: [f32; 2],
    parameter: f64,
    last_angle: f64,
    angle: f64,
    snap: Snap,
}
impl Drag {
    pub fn begin(
        base: Scene,
        actor: Actor,
        handle: Handle,
        camera: Camera,
        start: [f32; 2],
        snap: Snap,
    ) -> Option<Self> {
        let pivot = actor.position(&base)?;
        let parameter = if matches!(handle, Handle::X | Handle::Y | Handle::Z) {
            camera.axis_parameter(start, pivot, handle.axis())?
        } else {
            0.
        };
        let last_angle = if handle == Handle::Heading {
            camera.plane_angle(start, pivot)?
        } else {
            0.
        };
        Some(Self {
            base,
            actor,
            handle,
            camera,
            pivot,
            start,
            parameter,
            last_angle,
            angle: 0.,
            snap,
        })
    }
    pub fn update(&mut self, point: [f32; 2]) -> Result<Scene, String> {
        let amount = match self.handle {
            Handle::Heading => {
                let next = self
                    .camera
                    .plane_angle(point, self.pivot)
                    .ok_or("View angle cannot resolve this rotation.")?;
                let d = next - self.last_angle;
                self.angle += d.sin().atan2(d.cos());
                self.last_angle = next;
                self.snap.apply(Tool::Rotate, self.angle.to_degrees())
            }
            Handle::Uniform => {
                let exponent =
                    ((point[0] - self.start[0]) - (point[1] - self.start[1])) as f64 / 150.;
                let factor = exponent.clamp(-3., 3.).exp();
                (1. + self.snap.apply(Tool::Size, factor - 1.)).max(0.01)
            }
            _ => {
                let current = self
                    .camera
                    .axis_parameter(point, self.pivot, self.handle.axis())
                    .ok_or("View angle cannot resolve this axis.")?;
                self.snap.apply(Tool::Move, current - self.parameter)
            }
        };
        transformed(&self.base, self.actor, self.handle, amount)
    }
}

pub struct GizmoHandle {
    pub handle: Handle,
    pub points: Vec<[f32; 2]>,
    pub bounds: [f32; 4],
}
pub struct Gizmo {
    pub camera: Camera,
    pub actor: Actor,
    pub handles: Vec<GizmoHandle>,
}
pub fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0] * d[0] + d[1] * d[1];
    let t = if len > 0.01 {
        ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len
    } else {
        0.
    };
    let t = t.clamp(0., 1.);
    (p[0] - a[0] - d[0] * t).hypot(p[1] - a[1] - d[1] * t)
}
impl Gizmo {
    pub fn new(camera: Camera, actor: Actor, pivot: Vec3, tool: Tool) -> Option<Self> {
        let center = camera.project(pivot)?;
        if !camera.contains(center) {
            return None;
        }
        let size = camera.world_size(pivot);
        if size <= 0. {
            return None;
        }
        let mut handles = vec![];
        let mut add = |handle, points: Vec<[f32; 2]>| {
            if points.len() < 2 || points.iter().any(|p| !camera.contains(*p)) {
                return;
            }
            let minx = points.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
            let maxx = points
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max);
            let miny = points.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
            let maxy = points
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max);
            let left = (minx - 7.).max(camera.rect[0]);
            let top = (miny - 7.).max(camera.rect[1]);
            handles.push(GizmoHandle {
                handle,
                points,
                bounds: [
                    left,
                    top,
                    (maxx + 7.).min(camera.rect[0] + camera.rect[2]) - left,
                    (maxy + 7.).min(camera.rect[1] + camera.rect[3]) - top,
                ],
            });
        };
        match tool {
            Tool::Move => {
                for handle in [Handle::X, Handle::Y, Handle::Z] {
                    if let (Some(a), Some(b)) = (
                        camera.project(pivot + handle.axis() * size * 0.18),
                        camera.project(pivot + handle.axis() * size),
                    ) && (a[0] - b[0]).hypot(a[1] - b[1]) > 12.
                    {
                        add(handle, vec![a, b]);
                    }
                }
            }
            Tool::Rotate if actor != Actor::Structure && camera.forward.z.abs() > 0.03 => {
                let points = (0..=64)
                    .filter_map(|i| {
                        let a = i as f64 * std::f64::consts::TAU / 64.;
                        camera.project(pivot + Vec3::new(a.cos(), a.sin(), 0.) * size)
                    })
                    .collect();
                add(Handle::Heading, points);
            }
            Tool::Size => add(
                Handle::Uniform,
                vec![
                    [center[0] + 12., center[1] - 12.],
                    [center[0] + 55., center[1] - 55.],
                ],
            ),
            _ => {}
        }
        Some(Self {
            camera,
            actor,
            handles,
        })
    }
    pub fn hit(&self, point: [f32; 2]) -> Option<Handle> {
        self.handles
            .iter()
            .filter_map(|h| {
                let distance = h
                    .points
                    .windows(2)
                    .map(|p| segment_distance(point, p[0], p[1]))
                    .fold(f32::INFINITY, f32::min);
                (distance < 7.).then_some((distance, h.handle))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|h| h.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_gestures_cannot_overwrite_a_newer_scene() {
        let base = Scene::robotics_demo();
        let mut changed = base.clone();
        changed.current[0] += 0.2;
        let mut model = Model {
            document: Some(crate::project::Document::new(changed.clone())),
            ..Default::default()
        };
        let next = transformed(&base, Actor::Reference, Handle::X, 1.).unwrap();
        assert!(crate::editor::commit_transform(&mut model, &base, next).is_err());
        assert_eq!(model.document.as_ref().unwrap().scene, changed);
        assert!(!model.document.as_ref().unwrap().can_undo());
    }
    #[test]
    fn snapped_zero_rotation_preserves_the_authored_heading_exactly() {
        let mut scene = Scene::robotics_demo();
        scene.reference_heading = -45.;
        assert_eq!(
            transformed(&scene, Actor::Reference, Handle::Heading, 0.).unwrap(),
            scene
        );
    }
    fn camera(scene: &Scene) -> Camera {
        Camera::new(
            [75., 110., 800., 480.],
            Vec3::new(scene.camera[0], scene.camera[1], scene.camera[2]),
            Vec3::new(scene.target[0], scene.target[1], scene.target[2]),
            Vec3::new(0., 0., 1.),
            58.497,
        )
    }
    #[test]
    fn projected_points_and_rays_agree_with_offset_letterboxing() {
        let mut scene = Scene::robotics_demo();
        scene.camera = [-4., -6., -9.];
        scene.target = [0., 0., -12.];
        let camera = camera(&scene);
        for p in [
            Vec3::new(0., 0., -12.),
            Vec3::new(1., 2., -11.),
            Vec3::new(-2., 1., -13.),
        ] {
            let screen = camera.project(p).unwrap();
            let ray = camera.ray(screen);
            assert!((ray - (p - camera.eye).normalized()).length() < 1e-6);
        }
        let world = pelagic::scene::world(&scene).unwrap();
        let before = world.checkpoint();
        assert!(pick(camera, [0., 0.], &world.snapshot(), &world.ocean).is_none());
        let screen = camera.project(Vec3::new(0., 0., -12.)).unwrap();
        assert_eq!(
            pick(camera, screen, &world.snapshot(), &world.ocean),
            Some(Actor::Reference)
        );
        assert_eq!(before, world.checkpoint());
    }
    #[test]
    fn picking_uses_nearest_body_mesh_triangles_and_solid_occlusion() {
        let mut scene = Scene {
            vehicle: false,
            structure: false,
            camera: [0., -10., -10.],
            target: [0., 0., -10.],
            ..Default::default()
        };
        scene.entities = vec![
            pelagic::entities::Entity::rov(2, Vec3::new(0., 5., -10.)),
            pelagic::entities::Entity::rov(1, Vec3::new(0., 0., -10.)),
        ];
        let c = camera(&scene);
        let p = c.project(Vec3::new(0., 0., -10.)).unwrap();
        let world = pelagic::scene::world(&scene).unwrap();
        assert_eq!(
            pick(c, p, &world.snapshot(), &world.ocean),
            Some(Actor::Entity(1))
        );
        scene.structure = true;
        scene.structure_position = [0., 0., -10.];
        let world = pelagic::scene::world(&scene).unwrap();
        assert_eq!(
            pick(c, p, &world.snapshot(), &world.ocean),
            Some(Actor::Structure)
        );
        scene.structure = false;
        scene.entities.truncate(1);
        scene.entities[0].position = Vec3::new(0., 0., -45.);
        let world = pelagic::scene::world(&scene).unwrap();
        let p = c.project(scene.entities[0].position).unwrap();
        assert_eq!(pick(c, p, &world.snapshot(), &world.ocean), None);
        scene.entities[0].position = Vec3::new(0., 0., -10.);
        let mesh = std::sync::Arc::new(
            pelagic::assets::Mesh::parse_obj("v -1 0 -1\nv 1 0 -1\nv -1 0 1\nf 1 2 3", "triangle")
                .unwrap(),
        );
        scene.entities[0].asset = Some(scene.library.add_mesh("triangle", mesh).unwrap());
        let world = pelagic::scene::world(&scene).unwrap();
        let p = c.project(Vec3::new(0.4, 0., -9.7)).unwrap();
        assert_eq!(
            pick(c, p, &world.snapshot(), &world.ocean),
            None,
            "empty part of imported mesh proxy must not select the mesh"
        );
    }
    #[test]
    fn axis_drag_snap_and_heading_wrap_are_continuous() {
        let mut scene = Scene::robotics_demo();
        scene.camera = [-4., -6., -9.];
        scene.target = [0., 0., -12.];
        let camera = camera(&scene);
        let pivot = Actor::Reference.position(&scene).unwrap();
        let size = camera.world_size(pivot);
        let start = camera.project(pivot + Vec3::new(size, 0., 0.)).unwrap();
        let end = camera
            .project(pivot + Vec3::new(size + 1.26, 0., 0.))
            .unwrap();
        let mut drag = Drag::begin(
            scene.clone(),
            Actor::Reference,
            Handle::X,
            camera,
            start,
            Snap::Normal,
        )
        .unwrap();
        let next = drag.update(end).unwrap();
        assert!((next.reference_xy[0] - scene.reference_xy[0] - 1.5).abs() < 1e-6);
        let point = |degrees: f64| {
            let a = degrees.to_radians();
            camera
                .project(pivot + Vec3::new(a.cos(), a.sin(), 0.) * size)
                .unwrap()
        };
        let mut drag = Drag::begin(
            scene.clone(),
            Actor::Reference,
            Handle::Heading,
            camera,
            point(170.),
            Snap::Off,
        )
        .unwrap();
        let next = drag.update(point(-170.)).unwrap();
        assert!((next.reference_heading - scene.reference_heading - 20.).abs() < 1e-4);
        let next = drag.update(point(-150.)).unwrap();
        assert!((next.reference_heading - scene.reference_heading - 40.).abs() < 1e-4);
        assert!(transformed(&scene, Actor::Reference, Handle::Uniform, -1.).is_err());
        assert!(transformed(&scene, Actor::Reference, Handle::X, f64::NAN).is_err());
    }
    #[test]
    fn preview_is_read_only_and_one_commit_roundtrips_history_and_save() {
        let scene = Scene::robotics_demo();
        let root = std::env::temp_dir().join(format!(
            "pelagic-transform-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut model = Model {
            workspace: true,
            inspector_page: 1,
            document: Some(crate::project::Document::new(scene.clone())),
            session: Some(crate::session::Session::from_scene(&scene)),
            storage_root: Some(root.clone()),
            ..Default::default()
        };
        let before = model.session.as_ref().unwrap().world.checkpoint();
        let observations = model.session.as_ref().unwrap().observations.clone();
        let mut next = scene.clone();
        for amount in [0.1, 0.3, 0.8, 1.2] {
            next = transformed(&scene, Actor::Reference, Handle::X, amount).unwrap();
            model
                .session
                .as_mut()
                .unwrap()
                .preview_scene(&next)
                .unwrap();
            assert_eq!(before, model.session.as_ref().unwrap().world.checkpoint());
            assert_eq!(observations, model.session.as_ref().unwrap().observations);
            assert_eq!(scene, model.document.as_ref().unwrap().scene);
        }
        model.session.as_mut().unwrap().clear_preview();
        assert!(!model.document.as_ref().unwrap().can_undo());
        crate::editor::edit(&mut model, next.clone(), true).unwrap();
        crate::editor::history(&mut model, false).unwrap();
        assert_eq!(model.document.as_ref().unwrap().scene, scene);
        assert!(!model.document.as_ref().unwrap().can_undo());
        crate::editor::history(&mut model, true).unwrap();
        assert_eq!(model.document.as_ref().unwrap().scene, next);
        let size = transformed(&next, Actor::Reference, Handle::Uniform, 1.5).unwrap();
        assert_eq!(size.reference_mass, next.reference_mass);
        assert_eq!(size.reference_volume, next.reference_volume);
        assert_eq!(size.body_model.half, next.body_model.half * 1.5);
        assert_eq!(
            size.body_model.thrusters[0].position,
            next.body_model.thrusters[0].position * 1.5
        );
        crate::editor::edit(&mut model, size.clone(), true).unwrap();
        let path = root.join("transformed.pelagic");
        crate::editor::save(&mut model, &path).unwrap();
        assert_eq!(crate::project::Document::open(&path).unwrap().scene, size);
        model.session.as_mut().unwrap().step();
        assert!(!can_edit(&model));
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(resolved).unwrap();
    }
}
