//! Native UI Automation bridge. Only this application's semantic controls are
//! exposed; no desktop inspection or screen capture is involved.
use crate::view::{Model, Painter};
use accesskit::{
    Action, ActionData, ActionHandler, ActionRequest, ActivationHandler, Node, NodeId, Rect, Role,
    TreeId, TreeInfo, TreeUpdate,
};
use std::sync::{Arc, Mutex, mpsc};

const ROOT: NodeId = NodeId(1);
pub const MESSAGE: u32 = 0x8000 + 71;
pub struct Accessibility {
    adapter: accesskit_windows::SubclassingAdapter,
    tree: Arc<Mutex<TreeUpdate>>,
    pub requests: mpsc::Receiver<ActionRequest>,
}
struct Activate(Arc<Mutex<TreeUpdate>>);
impl ActivationHandler for Activate {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        Some(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}
struct Dispatch {
    window: usize,
    sender: mpsc::SyncSender<ActionRequest>,
}
impl ActionHandler for Dispatch {
    fn do_action(&mut self, request: ActionRequest) {
        if self.sender.try_send(request).is_ok() {
            // SAFETY: posts an integer-only notification to our own window.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    self.window as _,
                    MESSAGE,
                    0,
                    0,
                );
            }
        }
    }
}
pub fn tree(painter: &Painter, model: &Model) -> TreeUpdate {
    let mut root = Node::new(Role::Window);
    root.set_label("Pelagic — Ocean Engine");
    root.set_description(if model.workspace {
        "Ocean studio. Tab moves between controls. Enter activates. Fields accept text. Right drag orbits, wheel zooms, W A S D pans, Q E changes height, F frames the scene. Control S saves."
    } else { "Projects. Create, open or explore an ocean scene." });
    let scale = painter.ui_scale() as f64;
    let mut nodes = vec![];
    let mut children = vec![];
    for c in &painter.accessible {
        let id = NodeId(c.id as u64 + 2);
        // Some toolbar builders lay out a button twice while measuring it.
        if children.contains(&id) {
            continue;
        }
        let splitter = crate::layout::Splitter::from_id(c.id);
        let mut node = Node::new(if splitter.is_some() {
            // AccessKit 0.25 exposes Splitter as read-only on Windows. A slider
            // preserves adjustable numeric sizing through the UIA RangeValue API.
            Role::Slider
        } else if c.value.is_some() {
            Role::TextInput
        } else {
            Role::Button
        });
        node.set_label(c.label.clone());
        let [x, y, w, h] = c.rect;
        node.set_bounds(Rect::new(
            x as f64 * scale,
            y as f64 * scale,
            (x + w) as f64 * scale,
            (y + h) as f64 * scale,
        ));
        if c.disabled {
            node.set_disabled();
        } else {
            node.add_action(Action::Focus);
            if let Some(splitter) = splitter {
                if let Some(layout) = painter.editor_layout {
                    let (value, min, max) = layout.range(splitter);
                    node.set_numeric_value(value as f64);
                    node.set_min_numeric_value(min as f64);
                    node.set_max_numeric_value(max as f64);
                    node.set_numeric_value_step(20.0);
                    node.set_orientation(if splitter == crate::layout::Splitter::Sidebar {
                        accesskit::Orientation::Vertical
                    } else {
                        accesskit::Orientation::Horizontal
                    });
                    node.set_description("Drag the divider, use arrow keys when focused, or set its size in device-independent pixels.");
                    node.add_action(Action::SetValue);
                    node.add_action(Action::Increment);
                    node.add_action(Action::Decrement);
                }
            } else if let Some(value) = &c.value {
                let value = model
                    .editing
                    .as_ref()
                    .filter(|e| e.0 == c.id)
                    .map_or(value, |e| &e.1);
                node.set_value(value.clone());
                node.add_action(Action::SetValue);
            } else {
                node.add_action(Action::Click);
            }
        }
        children.push(id);
        nodes.push((id, node));
    }
    let status_id = NodeId(900);
    let mut status = Node::new(Role::Status);
    status.set_live(accesskit::Live::Polite);
    status.set_value(model.notice.clone());
    status.set_label("Status");
    children.push(status_id);
    nodes.push((status_id, status));
    if let Some(session) = &model.session {
        let id = NodeId(901);
        let mut viewport = Node::new(Role::Image);
        let displayed = session.view_snapshot();
        let position=displayed.vehicles.iter().find(|v|Some(v.id)==session.selected).or_else(||displayed.vehicles.first()).map(|v|format!("Displayed body position east {:.2}, north {:.2}, height {:.2} metres. Speed {:.2} metres per second.",v.position.x,v.position.y,v.position.z,v.velocity.length())).unwrap_or_default();
        let echoes = session
            .observations
            .first()
            .map(|o| {
                format!(
                    "Sonar has {} returns from {} beams. Sample time {:.2} seconds.",
                    o.beams.iter().filter(|b| b.range.is_some()).count(),
                    o.beams.len(),
                    o.time
                )
            })
            .unwrap_or_default();
        viewport.set_label(format!(
            "Ocean viewport. Simulation time {:.2} seconds. {position} {echoes}",
            session.world.time()
        ));
        children.push(id);
        nodes.push((id, viewport));
    }
    let focus = model
        .focus
        .map(|id| NodeId(id as u64 + 2))
        .filter(|id| children.contains(id))
        .unwrap_or(ROOT);
    root.set_children(children);
    nodes.push((ROOT, root));
    TreeUpdate {
        nodes,
        tree: Some(TreeInfo::new(ROOT)),
        tree_id: TreeId::ROOT,
        focus,
    }
}
impl Accessibility {
    pub fn new(window: usize, painter: &Painter, model: &Model) -> Self {
        let tree = Arc::new(Mutex::new(tree(painter, model)));
        let (sender, requests) = mpsc::sync_channel(128);
        let adapter = accesskit_windows::SubclassingAdapter::new(
            accesskit_windows::HWND(window as _),
            Activate(tree.clone()),
            Dispatch { window, sender },
        );
        Self {
            adapter,
            tree,
            requests,
        }
    }
    pub fn update(
        &mut self,
        painter: &Painter,
        model: &Model,
    ) -> Option<accesskit_windows::QueuedEvents> {
        let next = tree(painter, model);
        let mut cached = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        if *cached == next {
            return None;
        }
        *cached = next.clone();
        drop(cached);
        self.adapter.update_if_active(|| next)
    }
}
pub fn target(request: &ActionRequest) -> Option<usize> {
    (request.target_tree == TreeId::ROOT)
        .then_some(request.target_node.0)?
        .checked_sub(2)
        .filter(|id| *id < 512)
        .map(|id| id as usize)
}
pub fn value(request: ActionRequest) -> Option<String> {
    match request.data {
        Some(ActionData::Value(s)) if s.len() <= 8192 => Some(s.into()),
        _ => None,
    }
}
pub fn numeric_value(request: ActionRequest) -> Option<f64> {
    match request.data {
        Some(ActionData::NumericValue(value))
            if value.is_finite() && value.abs() <= f32::MAX as f64 =>
        {
            Some(value)
        }
        _ => None,
    }
}
