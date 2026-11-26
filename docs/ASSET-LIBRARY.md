# Project asset library and materials

Status: first working slice (November 2026). It covers mesh import, a per-project
library with stable handles, placement, material assignment and editing, rendered
thumbnails, undo/recovery/save and CPU/GPU display. It is not yet a full asset
database, texture pipeline or material graph. See "Limits" below.

## Workflow

1. **Import** (Content Browser or toolbar) reads a Wavefront OBJ (geometry only)
   on a background task. The mesh enters the project library. With an actor
   selected in the Outliner the mesh is assigned to that actor; otherwise a new
   body instance is placed. Importing identical geometry again reuses the asset.
2. **Content Browser → Meshes / Materials** shows library cards with rendered
   thumbnails, triangle counts and use counts. Clicking a card selects it; the
   header shows the selection.
3. **Place** adds a new body that uses the selected mesh at the view target
   (clamped between the surface and seabed and nudged off occupied spots). With a
   material card selected and a mesh-bodied actor selected, Place adds a painted
   copy of that actor's mesh. The new actor is selected. One undo entry.
4. **Assign** sets the selected mesh or material on the selected actor (reference
   vehicle included). Assigning a built-in preset copies it into the project once.
5. **Details → Appearance** (category menu) shows `Mesh:` and `Material:` pickers
   and a thumbnail of the actor's current look. "Procedural (built-in)" and
   "Default look" clear the handle.
6. **Details → Material surface** edits the actor's material: sRGB base colour
   (0–255 per channel), roughness (0.02–1) and metallic (0–1). The material is
   shared, so every actor using it changes.
7. **Remove** deletes the selected library item only when no actor uses it. Built-in
   presets cannot be removed.

Every change is an ordinary scene edit: validated, one undo/redo entry, recovery
snapshot, dirty marker and atomic save. Appearance edits reset the preview run like
other scene edits. Edits are refused while a transform gesture or background task
is active.

## Data model

- `pelagic::assets::Library { meshes: Vec<MeshAsset>, materials: Vec<Material> }`
  is embedded in the project (`library` key, schema v5). No external path is
  needed after import.
- `MeshAsset { id, name, mesh, proportions }`. IDs are positive, unique and never
  reused for a different asset while it exists; removing one does not renumber the
  rest. Imported meshes are centred and stretched to fill the unit box; the
  original proportions (largest axis 1) are kept so Place sizes the body box to the
  shape. Bodies draw the mesh scaled by their half sizes, so a resized body keeps
  the mesh exactly inside its physical box.
- `Material { id, name, base_color: [u8; 3] (sRGB), roughness, metallic }`.
- `Entity.asset` / `Entity.material` (and `Scene.reference_asset` /
  `reference_material`) are optional handles. Dangling handles are rejected by
  `Scene::validate`.
- Budgets: 32 meshes, 24 materials, 160,000 library vertices in total, each mesh
  20,000 vertices / 40,000 triangles, and the existing 16 MB document limit.
- Migration: v4 projects that embedded a mesh per entity load into the library;
  identical meshes share one asset and keep their v4 appearance (proportions
  1:1:1, mesh not re-stretched). Saving writes v5. Unknown future versions fail.

## Rendering and physics contract

- Materials are visual only. `World` creation copies them into
  `Ocean::materials` keyed by runtime body ID (reference = 0); mass, volume,
  added mass, controllers, contact boxes, sensors and seeds are unchanged.
- GPU preview uses base colour (linear), roughness and metallic per vertex.
- CPU reference uses base colour only. CPU and GPU share handles and geometry, not
  identical shading.
- For procedural bodies the material replaces the primary surface (ROV floats,
  vessel hull, drifter body); frames and fittings keep their built-in colours.
- Picking still uses mesh triangles inside the body box, as before.

## Thumbnails

`pelagic::thumbnail::render` is a deterministic 2× supersampled software
rasteriser with fixed studio lighting, independent of scene lighting and renderer
choice. Materials preview on a sphere. The editor caches thumbnails in memory (up
to 96 entries, keyed by mesh identity/shape, surface values, proportions and pixel
size); they are recomputed after restart rather than stored on disk.

## Demonstration scene

`pelagic-desktop --export-reference-project <new path> --library` writes the
asset-library demo (also supplied as `examples/scenes/asset-library.pelagic`): a
15 m survey site with a procedurally generated torpedo AUV, mooring buoy and seabed
lander, all imported through `Mesh::parse_obj`, placed with `library::place` and
painted with presets. `--capture-workspace <png> --library` renders it from
Pelagic's own buffers; add `--content=1|4`, `--select=<entity index>` and
`--tool=4|5` to show the library cards and Appearance/Material tabs.

## Limits

- OBJ geometry only: no textures, UVs, normals, vertex colours, MTL files, glTF,
  skeletons or animation. glTF and texture maps are the next importer step.
- One material per actor; no per-face materials, texture maps, emissive or
  transparency, and no material graph.
- The library is per project; there is no shared on-disk asset database, folder
  hierarchy, drag-and-drop placement, rename or reimport-from-source yet.
- Placement uses ROV physical defaults scaled to the box; review mass, volume and
  controller values in Details for each new body.
- Thumbnails use fixed lighting and do not show underwater attenuation.

## Verification

- Core: `tests/asset_library.rs` (migration, handles, budgets, renderer materials)
  and `src/thumbnail.rs` unit tests.
- Desktop: `library.rs` tests (import/place/assign/remove, proportions, demo
  scene) and `accessibility_tests.rs` (cards and tabs in bounds, button roles,
  thumbnails drawn, material field commit, undo, save/reopen).
- Native: `scripts/Test-AssetLibrary.ps1 -Executable <exe> -Project
  examples/scenes/asset-library.pelagic -Report <new json> [-GpuPreview]` starts
  its own hidden app with isolated storage, drives only that window's UI
  Automation tree, and checks preset assignment, placement, one-step undo/redo,
  refusal to remove a used mesh, v5 save and reopen.
