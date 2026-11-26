# Changelog

## Unreleased — open-source repository and desktop fixes — 2026-11-26

- Automatically recover to the CPU editor after GPU startup failure; preserve
  the project and storage arguments and show the reason. Add `--cpu`.
- Restore CPU frame generation when leaving the GPU studio, including errors.
- Isolate graphics settings, downloaded caches, fleet data and credentials in
  `--test-storage` sessions; save graphics preferences atomically.
- Consolidate viewport controls into Environment and Navigate menus, shorten
  persistent hints, reduce the night notice and wrap Details values.
- Publish source, examples, tests and build/CI configuration under the existing
  Apache-2.0 license with asset/data notices and contributor/security guidance.
- Exclude local AI handover notes, credentials, build products and caches.

## Unreleased — fly anywhere, real coasts, real species, live vs forecast — 2026-11-24

- **Flying.**
  - Drag with either mouse button to look around; the pointer is hidden and confined while you do.
  - WASD or the arrow keys move without holding a button. With an actor selected, W/E/R choose tools and the arrows still move.
  - Q/E or PageUp/PageDown go up and down; Shift is 5× faster; + and − change the speed.
  - Movement accelerates and brakes smoothly, and the camera never enters the ground or seabed.
  - Speed runs from 0.5 m/s to 20 km/s, with a slider and presets.
- **Navigation display.**
  - Position, depth or altitude, speed in m/s, knots and km/h, heading and pitch.
  - A compass tape with the bearing and distance of the nearest land.
  - Go to land, Surface, Dive, back to the scene, and Build detailed scene here.
- **Reach any coast.** Real terrain and seabed stream around the camera in three rings (41 km at 160 m, 328 km at 1.28 km, 1,280 km at 10 km), with Sentinel-2 colour on land.
- **Rendering.**
  - The sea reaches the true horizon at any altitude: the grid's rays now meet the curved sea, which also removes the fold-over band near the horizon.
  - Terrain is drawn on the curved Earth.
  - Reversed, infinite depth replaces the 0.1 m–100 km range. This fixes distant coast and seabed flicker, and the 100 km draw-distance cut.
  - Haze follows the observed visibility.
- **Real fish and other animals** (`pelagic::fauna`).
  - Species come from the OBIS records around the camera.
  - Each is modelled from its family's body plan (111 families) with the species' own colouring where catalogued (268 species, 17 genera).
  - Animals swim with their real locomotion, drawn by GPU instancing with two levels of detail. Fish cast shadows.
  - They live in their habitat and depth range around the camera wherever you go, and replace the old procedural group shapes.
- **Live is live.**
  - ● Live shows the present: the nearest NOAA buoy and airport weather report first, else the model value for this 15-minute step. It refreshes every 10 minutes.
  - The real sun, and live AIS ships within 80 km drawn in 3D with wakes and name labels.
  - Forecast is a separate menu: +1 h to +5 days.
  - Details has a "Here and now" section: maritime zones, local time, sources and ages of every value, sun and moon times, nearest land, ships, species.
- **Trees on real land.** Coconut palms on low tropical land, broadleaf trees elsewhere, conifers at high latitude or altitude.
  - They grow only where the satellite image shows vegetation, out to 5 km from the camera, and cast shadows.
  - Atoll islands now read as palm lines on the horizon.
- **Night is honest.** With Live on at night the sea is dark, as it really is. A notice says so, and **See it by day** keeps the live sea and weather but sets daytime light.
- **Earth map.** New layers:
  - Satellite yesterday (NASA VIIRS true colour, live);
  - Sea surface temperature (GHRSST MUR, daily).
- **Fixes.**
  - Real-world scenes no longer carry the template ROV, test boat and simulated school; tools that need a reference vehicle add it.
  - Small islands and atolls had no satellite colour (dark land): the 2016 Sentinel-2 mosaic fills them with flat ocean blue.
    - Such tiles now fall back to the 2024 mosaic.
    - Scenes saved with the masked colours are refreshed once when opened.
    - Water-coloured pixels on land are ignored.
  - Trials on a thruster-steered underwater vehicle (the default ROV) no longer report meaningless turning-circle and zig-zag numbers (e.g. a 23,296° overshoot): those ship tests are skipped with the reason. Speed, stopping, decay and wave trials still run.

## Unreleased — real sea at startup, touch-anywhere Earth, earthquakes, graphics settings — 2026-11-24

- **Startup on a real sea.** With nothing opened, Pelagic opens:
  1. the last real-world scene;
  2. otherwise the cached default real sea, Kavaratti in Lakshadweep;
  3. otherwise it builds that scene live.

  Real-world scenes refresh to the live sea when opened.
- **Click anywhere on the Earth** for a place card with:
  - a Sentinel-2 satellite view;
  - depth or height;
  - live waves, swell, wind, current and sea temperature;
  - the nearest port;
  - Explore in 3D.

  Scenes now build from any point. On land, in a port or in shallow water, they move to the nearest open water and look back at the clicked place.
- **Earth base layers.** Satellite (Sentinel-2 cloudless 2016, EOX, CC BY 4.0) and the new default, Satellite + seafloor: imagery on land and in shallows, bathymetry relief in deep water.
- **Satellite colour on land in 3D.** Applied to the scene area and the 82 km backdrop (`habitat::Imagery`; scene keys `imagery` and `backdrop_imagery`).
- **Marine life in 3D from the place.**
  - The OBIS checklist (within ~50 km) picks which animal groups swim in the viewport: sharks, rays, sea turtles, whales and dolphins, reef-fish shoals (`landscape::creatures`, `creature_mesh`).
  - The viewport Life here menu lists the species recorded.
- **Earthquakes.** Live USGS layer for M2.5+ over the past week, coloured by age and sized by magnitude, with tsunami flags. A Seismic activity list jumps to any event.
- **Graphics menu.**
  - Low, Medium, High or Ultra (Ultra by default on dedicated GPUs).
  - V-Sync on/off (off removes the frame-rate cap) and render scale 50–200%.
  - Up to 8.8 MP viewport (16 MP renderer limit).
  - The sea keeps moving while paused, only while the viewport is visible.
- **Tool tabs.** Each starts with a plain-language guide: what it answers, three steps, and Show me an example, which runs it with defaults. The guide collapses to one line on small screens.
- **Fixes.**
  - Cached all-zero NOAA rasters are rejected and re-fetched; they had stopped Kavaratti from building.
  - GPU out-of-memory in long offscreen runs: the renderer now keeps at most two frames in flight, a hidden viewport no longer animates, and the device is polled every frame.
  - The template scene is no longer treated as a real place.
  - The viewport bar no longer covers the view label.
  - Ship Handling's example selects a surface vessel first.
  - The main area gets more of the window by default.
- **Worldwide ships.** aisstream.io works with a user key stored only in `%LOCALAPPDATA%\Pelagic` (verified: 857 ships in 30 s).

## Unreleased — realistic ocean, real seafloor, marine layers and live data — 2026-11-24

- **GPU ocean.**
  - The physics waves are drawn exactly. FFT short waves (three 256² cascades) are added above the physics band.
  - Whitecaps are calibrated to Monahan's coverage, with persistent streaky foam on the crests and Kelvin wakes.
  - The surface is a camera-projected grid that reaches a curved horizon.
  - Shading: Beckmann sun glitter from the slope variance, light through crests, and spectral absorption.
  - Tests: GPU FFT against a direct sum; swell silhouettes.
- **Sky.**
  - Single-scattering atmosphere (Rayleigh, Mie, ozone) with real solar position.
  - Clouds with cloud shadows, bloom and auto exposure.
  - Underwater: volumetric light shafts, caustics and Snell's window by refraction.
- **Landscape (`pelagic::landscape`, `pelagic::habitat`).**
  - Full-resolution terrain materials.
  - A backdrop stitched to the scene grid without cracks.
  - Procedural corals, fans, sponges, rocks and swaying seagrass, placed from habitat maps.
  - Scene viewpoints.
  - The scene format gains `sun_azimuth`, `clouds`, `backdrop` and `habitat`; sun elevation now ranges from −6 to 90°.
- **Real-world scenes.**
  - Sources: NOAA NCEI DEM Global Mosaic seafloor, terrain-tile land, UNEP-WCMC reef and seagrass masks, and an 82 km backdrop.
  - The live sea and real sun are applied.
  - The scene opens on the nearest well-lit mapped reef.
- **Viewport quick bar.** Views, sea presets, Live sea now, time of day, clouds and water clarity. New scripts: `cam:`, `sun:`, `sea:`, `clouds:`, `quality:`, `view:`.
- **Earth Layers menu** (every layer hideable):
  - Habitat and seabed: reefs, seagrass, islands, GEBCO undersea feature names (searchable).
  - Navigation: shipping lanes, ports, OpenSeaMap seamarks, Marine Regions boundaries, sea names.
  - Live: a finer live-wave grid when zoomed in, NOAA NDBC buoys, and AIS ships (Digitraffic, plus aisstream.io with a user key).
  - Also OBIS marine-life records and My fleet (tracks, dead-reckoned predictions, CPA warnings, saved to `fleet.json`).
  - Click any object for details.
- **Fixes.**
  - The Earth right-click menu emptied as the pointer moved onto it, so "Create ocean scene at…" could not be clicked. It now acts on the right-clicked point.
  - The coordinate readout overlapped the "Create ocean scene here" button.
  - The toolbar now wraps.
  - The status and credits text on the globe no longer overlap.
  - GPU tests skipped silently on shader errors; they now fail.
- **Not included.** Military submarine or warship deployment tracking (out of scope). TeleGeography cable data is commercially licensed. Public AIS has no people-on-board count.

## Unreleased — hull and harbour inspection, oil spill drift — 2026-11-24

- `pelagic::inspection`: targets from the harbour pilings or a vessel's hull
  below its waterline; stand-off from the required GSD and water clarity
  (contrast exp(−c·range)) outside a clearance sized to the vehicle; passes
  along each face with usable-frame spacing; routes around the structure,
  underwater and above the seabed; per-patch coverage (frame, incidence,
  occlusion, GSD, contrast); a hovering vehicle flies it (PID on a target
  point that waits but skips after 30 s, heading to the surface,
  stabilised camera); exports.
- `pelagic::spill`: oil parcels drift with current + wind drift (3 %,
  optional deflection) + eddy diffusion; Fay gravity–viscous initial
  spreading; Fingas empirical evaporation for five crude oils or by
  distillation; stranding on a real-world scene's coastline; mass balance,
  slick area and thickness, first landfall; CSV and GeoJSON exports.
- Inspection and Spill tabs; Earth right-click **Oil spill here** and the
  spill drawn on the globe; `Pelagic.exe --inspect` and `--spill`; scripts
  `inspect`, `inspect-run`, `inspect-physics`, `spill`, `spill-here:`,
  `spill-steady:`, `spill-run`, `spill-hour:`, `select-reference`.
  Scripted captures wait for real-world scene builds; capture readback
  waits up to 60 s.
- README section listing every vehicle-testing and marine-operations tool.
- Found while testing: passes planned below a rising seabed stalled the
  ROV (fixed: viewpoints kept above the local seabed); a wound-up integral
  pushed it into a piling (fixed: integral only near the target point).

## Unreleased — ship handling and COLREGs training — 2026-11-24

- `pelagic::colregs`: CPA/TCPA, relative bearings, roles under Rules 13,
  14, 15 and 18 (fixed when risk first arises), configurable thresholds,
  eight standard encounters (head-on, crossing give way / stand on, a
  give-way vessel that does not act, overtaking, being overtaken, a
  fishing vessel from port, two vessels together) and seeded random ones;
  `Session` with own ship on its physics, helm and engine orders, a
  reference COLREGs autopilot (velocity-obstacle search with rule
  penalties, Rule 17 conduct, safe return to route), and a rule-by-rule
  debrief (passing distance, early and substantial action, Rules 14, 15,
  17(a)–(c), multi-vessel duties); batch runs and CSV/JSON export.
- `helm` (shared by survey and encounter runs): authority-sized heading
  loop and speed control with optional astern thrust.
- Ship Handling tab: radar plot (rings, vectors, trails, safe ring, CPA
  lines), bridge controls with keys and time acceleration, CPA/TCPA table,
  autopilot hand-over, debrief, autopilot batch; `Pelagic.exe --colregs`.
- Found while testing: the autopilot judged a vessel "past" at CPA and
  turned back towards it (fixed: past and clear needs opening, abaft the
  beam and well off, and the return checks for risk); alterations measured
  from the current heading froze turns half way (fixed: measured from the
  start of the avoidance); the helm started at zero thrust and lost a
  third of the speed (fixed: primed at the starting speed).

## Unreleased — seabed survey planning and coverage — 2026-11-24

- `pelagic::survey`: IHO S-44 Ed. 6.2.0 Table 1 orders (TVU, THU, feature
  size, coverage); multibeam (swath, range, beams, three-term sounding
  uncertainty) and side-scan (range, blind zone) sensors; surface or
  seabed-following platforms; adaptive line spacing from the narrowest
  swath, 200 % planning, lines cut around land and shoals, contour-following
  heading; coverage by ray-cast swaths on an area-aligned grid (passes, best
  TVU, gaps, land excluded); sounding-density check against the feature
  size; physics run of the lines with any vehicle (speed control, integral
  line-of-sight steering, survey-grade heading loop, terrain following,
  swath from the real pose; heading loop sized from the vehicle's yaw
  authority; 80 % thrust ceiling and slower turns).
- Survey tab (sensor, platform, order, area and lines; coverage, depth and
  uncertainty maps with lines and vehicle track; cross-track chart;
  export) and `Pelagic.exe --survey` (exit 2 when coverage falls short).
  Exports ESRI ASCII grids, GPX and a QGC WPL 110 mission.
- Trials: `Bench::rig_in` for reuse in scene worlds (trial results
  unchanged).
- Found while testing: an autopilot tuned from rigid-body inertia alone
  turned the example AUV wide (80 % coverage) and could not hold the
  unstable legacy demo boat (74 %); the authority-sized loop holds all
  three example vehicles to 0.05–0.2 m RMS.

## Unreleased — search and rescue planning — 2026-11-24

- `pelagic::sar`: Monte Carlo leeway drift (Allen & Plourde 1999 / Allen
  2005 coefficients for 21 object classes, per-particle leeway, wind and
  current errors, jibing, incident time window, LKP uncertainty), wind and
  current from forecasts at several locations (inverse-distance blend) or
  steady; expanding square, sector, parallel track and creeping line plans
  on the drift's axes, optionally moving with the datum; probability of
  success by flying the track through the drifting particles (inverse cube
  law or definite range; parallel-sweep POD matches erf(√π·C/2)); POC, POD,
  coverage; GPX route and QGC WPL 110 mission export.
- Search & Rescue tab: incident and search-unit setup, camera sweep-width
  estimate, plans ranked by POS, drift map with time slider and playback,
  POS chart; drift, track and area drawn on the Earth globe; Earth
  right-click **Search and rescue here**.
- `Pelagic.exe --sar` (live forecast or `--wind`/`--current`) and
  `--sar --objects`; Open-Meteo drift-area download (25 locations, past two
  days to three days ahead, cached per cycle).
- `metocean::{parse_utc, format_utc}`.

## Unreleased — live seas, swell, seakeeping and operability — 2026-11-23

- Live seas from Open-Meteo global wave models (`apps/desktop/src/forecast.rs`):
  Earth tab **Live waves** map (wave height and travel direction, 3-day
  hourly slider and animation), 7-day **Sea forecast** at any point (wind
  sea, swell, wind, current, sea temperature) with **Apply to current
  scene** and **Create scene here**; new globe scenes start with the live
  sea of the hour. Cached per 6-hour cycle, paced under the free tier,
  attribution shown; `PELAGIC_OPEN_METEO_KEY` for commercial use.
- Realistic seas: scenes combine a wind sea with a separate swell
  (`SeaState::swell`, `SeaState::with`), carry wind speed/direction and the
  sea's source; whitecaps follow the wind when known; wave height up to 15 m.
  Older project files load unchanged.
- `pelagic::metocean` (forecast parsing, compass → scene directions,
  `apply` with depth/steepness clamping) and `pelagic::seakeeping` (response
  tables in irregular seas, limiting Hs against NORDFORSK 1987 criteria,
  Ochi slamming, deck wetness, forecast outlook for a course).
- Seakeeping tab (limiting-height table, go/no-go now, hour strip, motions
  against limits, export with `outlook.csv`); CLI `--forecast` and
  `--seakeeping`.
- Physics: a hull partly or fully out of the water outside the waterline
  model now carries added-mass forces in proportion to its immersion (none
  in air), and keeps only their yaw parts.
- Shared chart module; Earth toolbar second row; scripted captures wait for
  live-sea downloads.

## Unreleased — standard trials, fins and rudders, physics corrections — 2026-11-23

- Standard trials (`pelagic::trials`, Trials tab, `Pelagic.exe --trials`):
  free decay (period, damping, closed-form estimate, implied GM), speed
  against thrust, port and starboard turning circles, 10°/10° and 20°/20°
  zig-zags, crash stop and regular-wave response (heave, roll, pitch), in a
  still deep basin or the scene's current and sea. IMO MSC.137(76)
  reference limits, custom limits, warnings, charts, JSON/CSV reports and
  CI exit codes. Deterministic and multi-threaded (about 0.3 s for a full
  suite).
- Fins, keels, rudders and hydroplanes (`pelagic::fins`, `BodyModel.fins`):
  flat-plate lift with stall, control channels (yaw, pitch, roll) commanded
  through `Vehicle::fin_command`; Details section to add and edit them.
- Added inertia (`BodyModel.added_inertia`) and its strip-theory estimate.
- Physics corrections (change results for existing scenes; the reference
  recording was re-recorded): thruster allocation no longer throttles
  propellers below the centre of mass to cancel their pitch moment; the
  added-mass Coriolis force now accompanies the Munk moment (energy is
  conserved; surface vessels keep the yaw parts only); waterline damping
  uses the same effective waterplane as the buoyancy.
- Example AUV and workboat gain tails, rudders, a lateral-plane fin and added
  inertia; both pass the IMO reference limits in trials.
- Trials tab sits beside Viewport and Earth; scene overview gets a "Run
  standard trials" action; script commands `trials`, `trials-run`,
  `trials-chart:<name>`, `select:<name>`, `import:<path>`.

## Unreleased — vehicle prototypes and editor redesign — 2026-11-23

- Vehicle definitions (`pelagic::definition`, `*.pelagic-vehicle.json`):
  import with an optional OBJ hull, export with the hull in metres; Details
  sections for hydrodynamics, thrusters and controllers. Example survey AUV
  and workboat USV.
- `BodyModel` inertia, drag and rotational-drag overrides (optional; earlier
  scenes and recordings unchanged) and hull-based estimates (box inertia,
  0.5·ρ·Cd·A drag, strip-theory rotational drag and added mass).
- Editor redesign: ocean-slate theme with one teal accent, larger type and
  spacing, grouped toolbar with an accent Play button, styled tabs, card
  sections in Details, status chips, scene overview with sea-state, contents
  and quick actions when nothing is selected, wider Details column.
- Accessibility: decorative glyphs are hidden from screen readers; Details
  values for hydrodynamics and thrusters are named.
## Unreleased — experiments, campaigns, reproducibility, vessel physics — 2026-11-23

- Sun shadows in the 3D viewport: 2048² shadow map along the refracted sun,
  3×3 filtered, texel-snapped; captures use the same path.
- Screen-reader support for the GPU editor (AccessKit / UI Automation):
  plain names without icon glyphs, named icon-only buttons, combo boxes,
  fields, viewport and globe; `scripts/Test-StudioAccessibility.ps1`.
- Experiments tab (ported legacy Run actions): live recording, headless
  batch, replay and scrubbing, save/open, verify, dataset and camera exports,
  backup polling and retry.
- Sensor Monitor tab: navigation readouts, auto-scaling sonar fan, view from
  the mounted camera. Details gained the missing sensor and school fields;
  ranges now match the core validation.
- Test campaigns (`pelagic::campaign`): seeded multi-run testing with sea-state
  and current jitter, acceptance criteria, per-run metrics, exact
  reproduction, JSON/CSV reports; editor panel and `--campaign` CLI with CI
  exit codes. `--rerecord` migrates old recordings.
- Bit-reproducible simulation across executables and machines: simulation
  maths uses portable `libm` (`math::Det`). The shipped reference recording
  was re-recorded and now verifies in other executables.
- Vessel hydrostatics: bodies crossing the surface integrate buoyancy over a
  4×3 column grid against the local wave surface (heave, roll and pitch with
  waves, waterplane righting moment) with linear radiation-style damping.
  Submerged bodies keep the previous model.
- Water: crest light transmission and empirical whitecap coverage
  (Pierson-Moskowitz wind, Monahan coverage).
- Licence notices: AccessKit winit adapter added to the vendored texts.
## Unreleased — GPU editor shell and Earth view — 2026-11-22

- New default editor: GPU swapchain, egui + egui_dock Unreal-style docked layout,
  full-resolution viewport without CPU readback, geometry rebuilt only on change.
- Unreal camera controls, picking, gizmos, live-preview Details, thumbnail
  Content Browser, Output Log; legacy UI kept behind `--legacy-ui`.
- Underwater optics: spectral absorption, depth-dependent scattering, Snell's
  window, god rays, caustics and sand detail.
- Earth tab: streamed 3D globe from real open bathymetry/elevation and NASA
  Blue Marble imagery; "Create ocean scene here" builds a scene from the real
  seabed. Tiles cached locally.
- Playback: seabed and structures are cached on the GPU as a static layer;
  only bodies, marine life, particles and water are rebuilt per tick
  (about 13 ms -> 5 ms on the reference laptop). FXAA anti-aliasing.
- Earth: camera tilt and heading (Shift/middle-drag, "3D tilt", compass),
  3D seabed and terrain relief with adjustable exaggeration, distance-based
  tile detail, sky, limb glow and horizon haze, edge skirts against seams,
  coarse placeholder tiles, and re-requests for tiles dropped from the queue.
- Waves on the GPU: the editor uploads a flat water mesh once and the vertex
  shader evaluates the same linear wave sum per frame (phases reduced in f64
  on the CPU). Per-tick geometry fell from about 5-8 ms to under 1 ms.
  Captures and the CPU renderer still use the CPU-sampled surface.
- Licence notices: texts vendored for crates whose archives omit them (egui
  family, clipboard-win, profiling, spirv, zune-core, zune-jpeg); the
  generator now lists every missing text and inventories the shipped Windows
  target.
## Unreleased — asset library and materials — 2026-11-22

- Project schema v5: embedded mesh/material library with stable handles;
  v4 per-entity meshes migrate (deduplicated) on load.
- Materials (sRGB base colour, roughness, metallic) drive GPU preview shading and
  CPU base colour without changing physics; seven built-in presets.
- Content Browser library cards with rendered, cached thumbnails; Materials filter;
  Place, Assign and Remove; Details Appearance and Material surface tabs.
- Imported meshes keep their proportions; placed bodies size their box to match.
- Bundled procedurally generated AUV, buoy and lander demo (`asset-library.pelagic`).
- New core, desktop and accessibility tests for migration, handles, budgets,
  thumbnails, placement and save/undo round trips.
## Unreleased development foundations — 2026-11-15

The earlier `1.0.0` candidate identifier does not signify acceptance of the
expanded CPU/GPU editor, node-graph and game-development requirements.

- Native UI Automation names, roles, focus, button invocation, value editing and status.
- Faster bilinear viewport composition and distance-filtered terrain detail.
- Sensor-state checkpoints and verification of motion plus observations.
- Static-obstacle escape for fish, long-session and timestep acceptance tests.
- One bounded recording writer, retained failed saves and explicit retry.
- Bounded background progress queue, minimized pause policy and overload indication.
- Verified native installation, stable launcher, version switching and rollback.
- Five curated projects, worked tutorials, format/model/build/distribution guides,
  dependency notices, issue templates and automated release checks.
## 0.4.0 — 2026-11-15

Integrated physical entities, schooling, sensors, recording/replay, datasets,
regional data, OBJ/current imports, C ABI and MAVLink subset. 42 automated checks.
## Earlier phases

Phase 1 established native startup, project workflows and UI contracts. Phase 2
introduced the shared ocean/environment, CPU geometry renderer and basic physics.
Historical benchmark documents remain available alongside current release evidence.
