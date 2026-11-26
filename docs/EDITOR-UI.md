# Native editor layout

The workspace has been rebuilt around the arrangement in
[Epic's editor interface reference](https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-editor-interface).
Pelagic uses its own Rust painter, embedded Geist font and brand assets.

- Menu bar: File, Edit, Window, Tools, Build, Select, Actor and Help. Entries invoke
  existing project, history, import, actor and experiment commands. Build currently
  offers scene validation and experiment actions.
- Main toolbar: save/history, add/import, experiments, play/step/reset, renderer
  selection and theme.
- Viewport toolbar: view selection, Lit/Wireframe, authoring tool, snapping and scalability.
  The viewport uses the available center/left region. Controls retain their DPI
  size when maximizing the window.
- World Outliner: actual scene entries with selection, actor/type search and
  scrolling. Enter commits a search; clear it to show the full scene again.
  Selection updates the Details panel. Wheel and arrow buttons scroll the list.
- Details: compact labeled property rows, working text edits and context actions.
  Existing scene, entity, marine-life, sensor, experiment and geographic settings
  remain available. Enter commits; Escape cancels; Tab moves focus.
- Content Browser: the project asset library (meshes and materials with rendered
  thumbnails and use counts) plus current-level actors/configurations with type
  icons. Filters: All, Meshes, Materials, World data, Marine life. Place, Assign
  and Remove act on the selected library card; see [asset library](ASSET-LIBRARY.md).
- Details → Appearance / Material surface: mesh and material pickers, an
  appearance thumbnail, and shared material colour/roughness/metallic fields.
- Output Log: bounded status history plus current renderer/world/frame information.
  The bottom status bar also exposes the latest notice or task progress.

Ctrl+Space toggles the Content Drawer. File > Project home returns to the launcher.
The Window menu opens the content/log tabs and existing tool panels. Native
`--dark` and `--light` flags can select a theme at launch.

Drag the three dividers to resize the right sidebar, Outliner/Details split and
Content Browser. They show resize cursors and blue focus feedback. Sizes are
measured in device-independent pixels, with minimum sizes that keep controls
reachable. During dragging the existing frame is fitted to the new bounds; the
render target is resized on release. Escape cancels an unfinished drag.

The divider controls are keyboard focusable: Left/Right resize the sidebar,
Up/Down resize the horizontal splits. Assistive clients can also set their numeric
sizes. Layout sizes and drawer visibility save automatically to local preferences
and restore at the next launch. They do not enter scene files or undo history.
Window > Layout provides explicit Save/Reset commands and keyboard resize actions.

F11 or the viewport's Maximize/Restore button expands the viewport and restores
the previous panels. This transient mode is not saved across restarts. Opening
the Content Browser or Output Log restores the panel layout. The native
`--maximize-viewport` flag can start a session in the expanded view.

The section view now uses contiguous opaque fills instead of antialiasing each
individual strip. It marks body positions in an east/depth projection. It does
not capture 3D orbit/zoom gestures or show a working Lit toggle.

The scene viewport now supports actor picking, synchronized Outliner selection,
selection boxes, XYZ movement, Z-heading rotation and uniform body/structure size.
Use **1**, **2**, **3** for Move/Rotate Z/Size; **F** frames the selected actor.
Each drag is one undoable edit. See [viewport editing](VIEWPORT-EDITING.md) for
snapping, cancellation and the distinction between authored state and simulation.

## Current limits

This is an editor layout and interaction rebuild, not full Unreal functionality.
Panels resize within their current arrangement; rearranging tabs, floating windows
and drag docking remain open. The Content Browser shows the per-project embedded library,
not a shared filesystem asset database. Arbitrary pitch/roll authoring, executable visual scripting,
material graphs, texture maps, glTF import and standalone game export remain open.

The graphics pipeline and prototype assets have not been replaced by this UI work.
CPU and GPU rendering retain their existing simulation contracts.

## Local checks

Thirty-one desktop tests cover existing workflows plus DPI density, camera hit
regions, fractional-scale section fill and reachability of the full actor/asset
inventory. Existing overlap/bounds and accessibility tests cover all tool pages.
Formatting and strict workspace Clippy pass.

`scripts/Test-Workspace.ps1` checks numeric resizing, actual mouse drags, keyboard
resizing, Escape cancellation, F11 selection preservation and restoration after a
real process restart. The same checks run on CPU and GPU previews. Evidence is
stored under `output/workspace-layout`. The native launch size now accounts for
Windows display scaling and the available work area.

`scripts/Test-Accessibility.ps1 -EditorLayout -GpuPreview` exercises the real native
app in an isolated session, using only its own HWND. It checks menu accessibility,
tab switches, actor filtering/selection, asset inspection, property edit/restore,
GPU retention, a 1,000-paint resource check and paused idle CPU. No desktop screen
capture is used. Reports and images rendered directly from the app's buffers are
under `output/editor-redesign`.
