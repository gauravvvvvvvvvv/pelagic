# Current product direction — development, not release acceptance

Pelagic is a native Rust ocean simulation and game-development engine. Authoring,
standalone games and headless experiments share project assets and runtime state.
The current CPU studio is a foundation, not an Unreal-equivalent finished product.

## Architecture decisions

- Pure Rust application/engine/renderer implementation, with native OS interfaces.
  No browser, WebView, Python or JavaScript runtime is required.
- One fixed-step simulation runtime with explicit physics/behaviour/sensor budgets.
  Rendering consumes immutable snapshots; it never owns simulation time.
- A CPU renderer remains supported. An experimental native GPU preview uses wgpu
  through DirectX 12/Vulkan and shares geometry with a CPU preview. Production GPU
  rendering and automatic selection of a measured tier remain planned; current
  selection is explicit. See [preview status](GPU-PREVIEW.md).
- The same asset identifiers, scene hierarchy and material descriptions feed both
  backends. Each material/effect declares its CPU implementation, approximation or
  unsupported status. Fallback must be visible and exportable in diagnostics.
- Display resolution, mesh LOD, cosmetic particles, shadows and reflection quality
  may vary by graphics tier. Physical populations, collision models, sensors and
  solver dt must not change silently. Camera sensor render settings are provenance.
- Native editor panels include viewport, outliner, properties, content browser,
  node graphs, animation/timeline, console and profiler. Layouts are dockable and
  persistent. Rust UI libraries may be used without adopting a web frontend.
- Stable entity IDs, components, transform hierarchy, asset handles and a command
  history underpin editor tools. The UI does not mutate physics objects directly.
- A shared typed graph infrastructure serves several distinct graph domains:
  gameplay, materials, ocean generation, behaviour and experimental learning.
  These domains have different validators, execution rules and capability limits.
- Games export to a standalone native player; editor code is excluded. Headless
  experiment runners remain separately available. Windows x64 is the first target.
- Optional HDC/VSA experiments observe recorded/live features through an explicit
  interface. They are not demonstrated substitutes for rendering or physical
  integration. Any claimed advantage requires a held-out benchmark and baselines.

## Present versus planned

Existing foundation: CPU geometry renderer, experimental native GPU preview with
shared CPU/GPU geometry, prescribed ocean fields, approximate
body dynamics, schooling, sensors, projects/history/recovery, recordings/datasets,
C ABI/MAVLink subset, native accessibility bridge and validation/benchmark tools.
The native workspace now has compact editor menus/toolbars, an actor Outliner,
Details rows and current-level Content Browser/Output Log. Actor search,
selection, asset inspection and existing property edits work in that layout.
Panel dividers resize by mouse, keyboard and accessibility APIs; local layouts
restore across restarts, with reset and viewport maximize/restore commands.
Viewport picking and initial-scene XYZ/heading/size handles now share a native
CPU/GPU overlay and the existing history/save paths. Full transform hierarchies,
pitch/roll authoring and production content tools remain open.
A per-project mesh/material library with stable handles, thumbnails, placement and
material assignment is the first asset-workflow slice ([asset library](ASSET-LIBRARY.md)).
See [editor status](EDITOR-UI.md); drag docking, textures/glTF and a shared asset
database remain open.

Major planned systems: production asset/material pipeline and GPU rendering, editor
docking/content tools/gizmos, graph compiler/runtime, general game runtime, game
UI/audio/animation tooling, multiplayer, streamed worlds and learning extensions.
No screenshot, test count or version number signs off visual quality by itself.

## Delivery sequence

1. **Shared foundations and visual proof:** asset/component/render contracts; one
   authored ocean scene on CPU and native GPU; measured tiers on Iris Xe and a
   declared discrete-GPU reference when available. Visually review the actual app.
2. **Professional authoring:** docking, hierarchy, content browser, working gizmos,
   materials and typed graphs; graph changes participate in undo/save/recovery.
3. **Playable vertical slice:** character/vehicle input, game cameras, interactions,
   animation, audio, game UI, save/load and a standalone exported ocean game.
4. **Expanded systems:** richer ocean/life/sensors, streaming, procedural tools,
   networking and optional learning experiments with evidence.
5. **Qualification:** finish every accepted backlog item in its declared envelope,
   cross-backend checks, accessibility, migration, profiling, distribution and docs.

These are dependency milestones, not dates or a promise of complete Unreal parity.
The expanded backlog is a substantial development program. Estimates follow the
first representative vertical slice; an unimplemented requirement remains open.

## Reference sources

- [Epic editor interface](https://dev.epicgames.com/documentation/en-us/unreal-engine/unreal-editor-interface): workflow reference, not a copied implementation.
- [Epic visual scripting](https://dev.epicgames.com/documentation/en-us/unreal-engine/overview-of-blueprints-visual-scripting-in-unreal-engine): executable node networks are the capability reference.
- [wgpu](https://wgpu.rs/): native Rust graphics backends, independently of its browser support.
- [Khronos glTF](https://registry.khronos.org/glTF/): interchange for meshes, PBR material parameters, skins and animation data; a full editor/runtime is still required.
