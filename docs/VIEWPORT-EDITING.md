# Viewport selection and transforms

Open a project and choose Surface or Underwater. Click a vehicle or harbour
structure in the viewport to select it. The Outliner and Details follow the
selection; viewport picking clears an Outliner search so the selected row can be
shown. Click empty space to clear selection. **F** or **Frame selected** brings a
selected body into view. F also frames harbour structures.

Use the tool dropdown or these shortcuts:

| Shortcut | Tool | Drag target |
|---|---|---|
| 1 | Move | Red X, green Y, blue Z arrow |
| 2 | Rotate Z | Heading ring, for vehicle/body actors |
| 3 | Size | Diagonal uniform-size handle |

Heading rotation matches the existing authored heading parameter. Full initial
pitch/roll and arbitrary transform hierarchies are not implemented. Harbour
structures currently support movement and uniform size.

The Snap menu applies increments to the change from the beginning of a drag:

| Preset | Move | Heading | Size factor |
|---|---:|---:|---:|
| Off | Continuous | Continuous | Continuous |
| Fine | 0.1 m | 5° | 0.05 |
| Medium | 0.5 m | 15° | 0.10 |
| Coarse | 1 m | 45° | 0.25 |

Tab can focus handles; arrow keys nudge them. Without snapping, keyboard steps are
0.1 m, 5° and 5% respectively. Position and heading remain editable in Details;
the Dimensions category exposes the body's full X/Y/Z dimensions in metres.
RMB orbit and wheel zoom retain their existing behavior.

## Edits and physical state

Transforms edit the initial scene. Reset the preview before manipulating a scene
that has already stepped; handles are hidden during simulation, paused runs after
tick zero, recorded playback and mounted-camera views. Selection remains available.
Finish or cancel an active import/background task before starting a transform.
Controller targets are separate authored settings and are not rewritten by moving
or rotating a body.

While dragging, a temporary visual world supplies the preview. The authoritative
world, sensors and project document remain unchanged. Release commits one history
entry and resets the preview to the edited initial scene. Escape or capture/focus
loss cancels the drag. No-op gestures do not create an undo entry. Save, recovery,
undo and redo use the existing project format and editor paths.
A changed scene invalidates an outstanding gesture instead of overwriting the
newer scene at commit.

Uniform body resizing changes the rendered/contact dimensions and the relative
positions of buoyancy centers, thrusters and the selected body's mounted sensors.
The new dimensions also feed the existing inertia calculation on world creation.
Mass, displaced volume, added mass, thrust limits and controller settings remain
explicit authored values. Review Body settings when changing a physical design;
resizing does not establish a physically calibrated model. Imported mesh vertices
retain their normalized local geometry and use the body's dimensions.

## Picking and display

The editor maps through the actual fitted frame rectangle, FOV and camera basis,
including DPI scaling, letterboxing and mounted views. Vehicles use oriented body
proxies; imported meshes additionally use their triangles. Static harbour boxes
and the prescribed seabed occlude picks. The seabed test uses a bounded straight-ray
march and can miss sub-sample terrain features. Transparent water, fish and tracers
do not block actor selection. Non-body fields such as marine-life settings remain
selectable through the Outliner.

The selection overlay and handles are drawn in the native editor over either CPU
or GPU frames. They are not included in camera sensor exports. Near-parallel axes
and edge-on heading rings can be unavailable; use Details or orbit to another
angle. Overlay boxes may be visible through geometry for an actor selected in the
Outliner, while viewport picking still respects solid occlusion.

## Verification

Unit tests cover projection/ray agreement, nearest hits, mesh holes, solid
occlusion, axis snapping, angle wrap, zero rotation, read-only previews, single-edit
history and saved transforms. `scripts/Test-ViewportEdit.ps1` drives only its own
isolated native HWND and accessibility tree. CPU/GPU runs exercise viewport mouse
selection, multi-move drags, single undo/redo, Escape and capture-loss cancellation,
rotation, uniform size, keyboard nudges, simulation guards and reload after a real
process restart. It also checks that authored mass/volume survive resizing.
Local evidence is under `output/viewport-editing`; no desktop capture is used.
