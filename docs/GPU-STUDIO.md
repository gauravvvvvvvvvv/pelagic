# GPU editor shell and Earth view

Status: first version, 22 November 2026. `Pelagic.exe` now opens the GPU editor by
default. The previous CPU-painted editor remains available with `--legacy-ui`.

GPU startup failures now open the CPU editor automatically, preserving `--open`
and storage arguments. A visible notice and `startup.json` in the selected app
storage explain the fallback. `--cpu` chooses the CPU editor directly;
`PELAGIC_DISABLE_GPU=1` exercises the same automatic recovery path without
changing display drivers. Offscreen GPU captures still return a failure when
GPU initialization fails, so automation cannot mistake a CPU view for a capture.

The viewport's **Environment** menu contains views, marine-life records, sea
state, live/forecast controls, time, clouds and clarity. **Navigate** contains
speed and travel controls. Long Details values wrap to the panel width. These
controls remain named and keyboard accessible through AccessKit.

`--test-storage=<directory>` isolates preferences, startup scenes, tile/feed
caches, fleet data and credentials from normal app storage.

## What changed and why

The old editor painted every pixel on the CPU, rendered the 3D view at about
512 px wide and stretched it, and in GPU mode copied each frame back to the CPU
before showing it through GDI. Camera motion therefore waited on CPU geometry
preparation, GPU work and a synchronous readback.

The new shell:

- draws the whole window on the GPU (wgpu swapchain, vsync, DX12/Vulkan);
- renders the 3D viewport at the panel's real pixel size (up to a 4-megapixel
  budget) straight into a texture the UI samples, with no readback;
- rebuilds scene geometry only when the world changes; camera moves only
  update a uniform buffer. Geometry is split into a static layer
  (`RenderScene::build_static`: seabed, structures), re-uploaded only when
  `Session::world_revision`, quality or terrain material change, and a
  dynamic layer (`build_bodies`: bodies, fish, particles) rebuilt per
  simulation tick (`Session::revision`);
- uploads the water surface once as a flat mesh (`RenderScene::flat_water`)
  and evaluates the linear wave model in the vertex shader every frame from
  the sea state's components (phases reduced in f64 on the CPU). Inside the
  1 km fine grid it is the same sum as the CPU model; on the horizon rings,
  components shorter than two vertex spacings are faded out instead of
  aliasing;
- applies FXAA after tone mapping;
- casts sun shadows from a 2048² shadow map that follows the camera focus,
  cast along the refracted sun (which lights everything below the surface),
  with 3×3 filtering, normal-offset bias and texel snapping against shimmer;
- uses egui widgets and `egui_dock` for an Unreal-style docked layout:
  Viewport / Earth tabs, Outliner, Details, Content Browser, Output Log,
  menu bar, toolbar and status bar. Tabs can be dragged, split and re-docked;
  Window > Reset layout restores the default.

All scene edits still go through `editor::edit` (validation, undo/redo,
recovery file, dirty marker, atomic save). The simulation, project format and
asset library are unchanged.

## Viewport controls (Unreal conventions)

| Input | Action |
|---|---|
| RMB + WASD, Q/E | Fly; wheel while flying changes speed, Shift = fast |
| Alt + LMB | Orbit around the focus point |
| MMB | Pan |
| Wheel | Dolly |
| LMB | Select (click) or drag a gizmo handle |
| W / E / R, Space | Move / rotate (heading) / uniform scale tools |
| F | Frame selection |
| Del, Ctrl+D | Delete, duplicate |
| Ctrl+Z / Ctrl+Y, Ctrl+S, Ctrl+O | Undo / redo, save, open |

Initial transforms can be edited only when the simulation is reset (tick 0),
as before. Details fields preview live while dragging and commit one undo
entry on release.

## Experiments, sensors and campaigns

See [CAMPAIGNS.md](CAMPAIGNS.md). The Experiments tab holds the recording,
batch, replay and export tools ported from the legacy editor plus test
campaigns; the Sensor Monitor tab shows live or recorded navigation and sonar
observations and can switch the viewport to the mounted sensor camera.

## Accessibility

The shell exposes egui's widget tree through AccessKit to UI Automation
(Narrator, NVDA, Accessibility Insights). Icon glyphs and tab padding are
stripped from accessible names each frame, icon-only and unlabelled controls
have explicit names, and Details values are named after their labels.
`scripts/Test-StudioAccessibility.ps1 -Executable <exe> -Report <json>`
attaches only to the process it starts (via `--automation-handle=<file>`),
uses isolated storage and checks names, actions (play/pause, select, search,
tab switching) and the Experiments and Sensor Monitor panels.

## Underwater rendering

The GPU shader models spectral absorption (red is lost within metres, blue
travels furthest, scaled by the scene's optical attenuation), depth-dependent
in-scattered light, Snell's window and total internal reflection seen from
below, animated god rays, caustic filaments on upward-facing surfaces and
procedural sand detail. These are visual approximations, not a calibrated
optical model; camera-sensor exports still use the CPU reference renderer.

## Earth tab

A 3D globe with level-of-detail tile streaming, drawn as textured GPU meshes
with f64 CPU projection (no precision jitter at a few hundred metres altitude).

- Drag to pan, wheel to zoom about the cursor, double-click to dive, right-click
  for "Create ocean scene here". **Places** has preset ocean sites; the text box
  accepts `lat, lon`.
- **Tilt and heading**: Shift+drag, Ctrl+drag or middle-drag orbits the view
  centre (horizontal = heading, vertical = tilt, up to 78° near the surface,
  phased out from orbit). The **3D tilt** button toggles a 60° view; the
  compass resets to north-up and straight down.
- **3D relief**: tiles are displaced by the real elevation/depth times the
  **relief ×** exaggeration (default 2, 0 = flat). Tile detail follows each
  tile's distance from the camera, tiles are drawn far-to-near, and short edge
  skirts hide cracks between detail levels. Below about 250 km the sky turns
  from space to daylight, with a limb glow and horizon haze.
- Picking still intersects the smooth sphere, so hover depth and
  scene-creation points can be offset slightly on steep exaggerated relief
  seen at a grazing angle.
- **Bathymetry relief** (default): AWS Terrain Tiles "terrarium" elevation
  (Mapzen; GEBCO, ETOPO1, SRTM and other sources, open data, zoom 0–15),
  hillshaded and depth-tinted locally. Above zoom 8 the dataset leaves much of
  the open ocean at exactly 0 m; those pixels are filled from coarser zooms so
  the sea floor stays real instead of looking like land.
- **Satellite (Blue Marble)**: NASA GIBS Blue Marble shaded relief and
  bathymetry, public domain, zoom 0–8 (upsampled when closer).
- **Create ocean scene here** samples a 257 × 257 seabed grid at 20 m spacing
  (≈5.1 km square) around the view centre and opens it as an unsaved Pelagic
  scene with provenance (source, licence, resolution, origin latitude and
  longitude). Points on land or shallower than 6 m are refused.

Network and privacy: the Earth tab and scene creation download tiles over HTTPS
from `s3.amazonaws.com` and `gibs.earthdata.nasa.gov`. Requests contain only tile
coordinates and a generic user agent. Tiles are cached in
`%LOCALAPPDATA%\Pelagic\tiles`; cached areas work offline. Delete that folder to
clear the cache.

Accuracy: ocean depths come from global compilations (typically 450 m–1.8 km
source resolution in deep water), so local scenes have real large-scale seabed
shape but not survey-grade detail. Google Earth's own satellite photography is
proprietary and is not used.

## Captures and automation

`pelagic-desktop --capture-studio <png> [--frames=N] [--size=WxH] [--library]
[--open <project>] [--test-storage=<dir>] [--script=<cmd>]...` renders the real
editor offscreen from its own GPU buffers and exits. Script commands:
`select-auv`, `frame`, `play`, `globe`, `satellite`, `viewport`,
`goto:<lat>,<lon>,<distance km>[,<tilt>[,<heading>]]`, `create`, `experiments`,
`sensors`, `sensor-camera`, `record`, `stop-recording`, `batch:<s>`,
`replay`, `scrub:<sample>`, `campaign:<runs>`, `import:<vehicle.json>`,
`select:<actor name>`, `trials`, `trials-run` (quick trials on the selected
vehicle), `trials-chart:<decay|speed|turning|zigzag|waves>`, `seakeeping`,
`seakeeping-run`, `live-waves`, `sea-forecast:<lat>,<lon>`,
`apply-forecast`, `sar`, `sar-here:<lat>,<lon>`,
`sar-steady:<wind m/s>,<from °>,<current m/s>,<towards °>`, `sar-run`,
`sar-minute:<minutes into the search>`, `survey`, `survey-run`,
`survey-physics` (selected vehicle), `survey-set:<fit|mbes|sss|1a|special|exclusive|area=CX,CY,L,W,HEADING>`,
`ship`, `ship-start:<0-8>` (8 = random), `ship-autopilot`, `ship-run:<seconds>`, `ship-suite`,
`select-reference`, `inspect`, `inspect-run`, `inspect-physics` (selected ROV), `spill`,
`spill-here:<lat>,<lon>`, `spill-steady:<wind m/s>,<from °>,<current m/s>,<towards °>`,
`spill-run`, `spill-hour:<hours after release>`,
`cam:<x,y,z,tx,ty,tz>`, `sun:<elevation°>,<azimuth°>`, `clouds:<0-1>`,
`sea:<Hs m>,<Tp s>[,<wind m/s>[,<direction°>]]`, `quality:<0-2>`,
`view:<aerial|surface|reef|shore|seabed>`, `reefs`, `seagrass`, `names`,
`find:<undersea feature>`, `layer:<lanes|ports|seas|buoys|ships|life|fleet|waves|seamarks|boundaries|islands>`,
`vessel:<lat>,<lon>,<knots>,<course>,<name>`
(live seas and live drift need
network access). Commands run in
order and each
waits for the previous background task; captures count frames only after
scripted work finishes. Capture mode never shows dialogs.

## Known limits

- Bodies, fish and particles are still rebuilt on the CPU each tick (under
  1 ms for the demo scenes). Captures through `GpuRenderer::render` and the
  CPU renderer still sample the water on the CPU.
- No texture maps yet in the 3D viewport; one shadow cascade (about 16–400 m
  around the focus), so distant objects are unshadowed.
- Whitecaps use an empirical coverage estimate placed on crests, not a
  breaking-wave simulation; foam does not persist or advect.
- The Earth globe is drawn with the painter's algorithm (no depth buffer), so
  near relief can be overdrawn by a neighbouring tile at extreme
  exaggeration; the sky and haze are artistic gradients, not a scattering
  model.
