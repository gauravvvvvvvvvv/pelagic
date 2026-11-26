use super::*;
use crate::viewport_edit::{Actor, Camera, Gizmo, Handle};
use pelagic::math::{Quat, Vec3};

fn corners(position: Vec3, half: Vec3, rotation: Quat) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        position
            + rotation.rotate(Vec3::new(
                if i & 1 == 0 { -half.x } else { half.x },
                if i & 2 == 0 { -half.y } else { half.y },
                if i & 4 == 0 { -half.z } else { half.z },
            ))
    })
}
fn bounds(camera: Camera, points: &[Vec3]) -> Option<[f32; 4]> {
    let points: Vec<_> = points.iter().filter_map(|p| camera.project(*p)).collect();
    if points.is_empty() {
        return None;
    }
    let x = points
        .iter()
        .map(|p| p[0])
        .fold(f32::INFINITY, f32::min)
        .max(camera.rect[0]);
    let y = points
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min)
        .max(camera.rect[1]);
    let right = points
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .min(camera.rect[0] + camera.rect[2]);
    let bottom = points
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .min(camera.rect[1] + camera.rect[3]);
    (right > x && bottom > y).then_some([x, y, right - x, bottom - y])
}
fn clipped(a: [f32; 2], b: [f32; 2], r: [f32; 4]) -> Option<([f32; 2], [f32; 2])> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-d[0], a[0] - r[0]),
        (d[0], r[0] + r[2] - a[0]),
        (-d[1], a[1] - r[1]),
        (d[1], r[1] + r[3] - a[1]),
    ] {
        if p.abs() < 1e-7 {
            if q < 0. {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0. {
                low = low.max(t);
            } else {
                high = high.min(t);
            }
            if low > high {
                return None;
            }
        }
    }
    Some((
        [a[0] + d[0] * low, a[1] + d[1] * low],
        [a[0] + d[0] * high, a[1] + d[1] * high],
    ))
}
impl Painter {
    pub(super) fn viewport_overlay(&mut self, model: &Model) {
        let Some(camera) = self.viewport_camera else {
            return;
        };
        let session = model.session.as_ref().unwrap();
        let snapshot = session.view_snapshot();
        let ocean = session.view_ocean();
        let scene = model
            .transform_preview
            .as_ref()
            .unwrap_or(&model.document.as_ref().unwrap().scene);
        let selected = Actor::selected(model);
        let mut items: Vec<(Actor, Vec3, Vec<Vec3>, usize)> = vec![];
        for body in &snapshot.vehicles {
            let actor = if body.id == 0 {
                Actor::Reference
            } else {
                Actor::Entity(body.id)
            };
            let id = if body.id == 0 {
                430
            } else {
                let Some(i) = scene.entities.iter().position(|e| e.id == body.id) else {
                    continue;
                };
                431 + i
            };
            items.push((
                actor,
                body.position,
                corners(body.position, body.half, body.orientation).to_vec(),
                id,
            ));
        }
        if scene.structure {
            let pivot = Actor::Structure.position(scene).unwrap();
            let points = ocean
                .structures
                .iter()
                .flat_map(|s| corners(s.center, s.half, Quat::IDENTITY))
                .collect();
            items.push((Actor::Structure, pivot, points, 446));
        }
        for (actor, pivot, points, id) in items {
            let Some(rect) = bounds(camera, &points) else {
                continue;
            };
            let name = actor.name(scene);
            if camera.project(pivot).is_some_and(|p| {
                camera.contains(p)
                    && crate::viewport_edit::pick(camera, p, &snapshot, ocean) == Some(actor)
            }) {
                self.viewport_actors.push((id, actor));
                self.accessible.push(AccessibleControl {
                    id,
                    rect,
                    label: format!("Select in viewport: {name}"),
                    value: None,
                    disabled: false,
                });
            }
            if selected != Some(actor) {
                continue;
            }
            for cube in points.as_chunks::<8>().0 {
                for (a, b) in [
                    (0, 1),
                    (0, 2),
                    (0, 4),
                    (1, 3),
                    (1, 5),
                    (2, 3),
                    (2, 6),
                    (3, 7),
                    (4, 5),
                    (4, 6),
                    (5, 7),
                    (6, 7),
                ] {
                    if let (Some(a), Some(b)) = (camera.project(cube[a]), camera.project(cube[b]))
                        && let Some((a, b)) = clipped(a, b, camera.rect)
                    {
                        self.segment(a, b, 0xe7c776);
                    }
                }
            }
            self.text(
                &self.fit_text(&name, 180., 10., false),
                rect[0]
                    .min(camera.rect[0] + camera.rect[2] - 180.)
                    .max(camera.rect[0] + 2.),
                (rect[1] - 17.).max(camera.rect[1] + 25.),
                10.,
                0xe7c776,
                false,
            );
            if !crate::viewport_edit::can_edit(model) {
                continue;
            }
            let Some(gizmo) = Gizmo::new(camera, actor, pivot, model.transform_tool) else {
                continue;
            };
            for handle in &gizmo.handles {
                let active = model.hover == Some(handle.handle.id())
                    || model.focus == Some(handle.handle.id());
                let color = if active {
                    0xffdf84
                } else {
                    match handle.handle {
                        Handle::X => 0xee7777,
                        Handle::Y => 0x86cf8a,
                        Handle::Z => 0x78b5ff,
                        Handle::Heading => 0xf1cc78,
                        Handle::Uniform => 0xe7e7ef,
                    }
                };
                for pair in handle.points.windows(2) {
                    self.segment(pair[0], pair[1], color);
                    self.segment(
                        [pair[0][0] + 1., pair[0][1]],
                        [pair[1][0] + 1., pair[1][1]],
                        color,
                    );
                }
                if matches!(handle.handle, Handle::X | Handle::Y | Handle::Z) {
                    let end = *handle.points.last().unwrap();
                    let start = handle.points[handle.points.len() - 2];
                    let dx = end[0] - start[0];
                    let dy = end[1] - start[1];
                    let n = dx.hypot(dy).max(0.01);
                    for side in [-1., 1.] {
                        let p = [
                            end[0] - dx / n * 10. - dy / n * 4. * side,
                            end[1] - dy / n * 10. + dx / n * 4. * side,
                        ];
                        if camera.contains(p) {
                            self.segment(end, p, color);
                        }
                    }
                } else if handle.handle == Handle::Uniform {
                    let end = handle.points.last().unwrap();
                    let r = camera.rect;
                    let x = (end[0] - 3.).max(r[0]);
                    let y = (end[1] - 3.).max(r[1]);
                    self.rect(
                        x,
                        y,
                        6.0f32.min(r[0] + r[2] - x),
                        6.0f32.min(r[1] + r[3] - y),
                        color,
                    );
                }
                let accessible_rect = if handle.handle == Handle::Heading {
                    let p = handle.points[0];
                    [p[0] - 5., p[1] - 5., 10., 10.]
                } else {
                    handle.bounds
                };
                self.accessible.push(AccessibleControl {
                    id: handle.handle.id(),
                    rect: accessible_rect,
                    label: handle.handle.label().into(),
                    value: None,
                    disabled: false,
                });
            }
            self.gizmo = Some(gizmo);
        }
    }
}
