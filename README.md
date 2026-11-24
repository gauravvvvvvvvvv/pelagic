# Pelagic — development build

**Release status:** the current CPU implementation is a development foundation.
The Unreal-inspired editor, production GPU rendering, game runtime and experimental
learning extensions are not complete. A native GPU preview now shares scene geometry
with a CPU preview; see [GPU preview](docs/GPU-PREVIEW.md). The existing `1.0.0` Cargo identifier is an unreleased
candidate label, not product acceptance. See [product direction](docs/PRODUCT-DIRECTION.md).

A pure Rust ocean engine and native Windows studio, including its CPU renderer,
interface and synthesized startup sound. It runs offline without a browser,
WebView, Python or JavaScript runtime. The Rust library and versioned C ABI also
support independent headless hosts.

Open **Pelagic.exe** in this repository or the portable package. In a release ZIP,
**PelagicSetup.exe** adds verified per-user installation, a stable launcher, updates
and rollback. The build is unsigned; see [distribution](docs/DISTRIBUTION.md).

The native editor now uses an Unreal-style menu/toolbar, central viewport,
World Outliner, compact Details and Content Browser/Output Log layout, with
resizable panels, saved layouts and viewport maximize/restore.
Viewport actors can be selected and moved, rotated in heading or resized with
undoable handles; see [viewport editing](docs/VIEWPORT-EDITING.md). Imported meshes
and materials live in a per-project library with rendered thumbnails, placement and
assignment; see [asset library](docs/ASSET-LIBRARY.md). The default editor now runs
entirely on the GPU with a docked Unreal-style layout and an Earth tab that streams
real global bathymetry; see [GPU editor and Earth view](docs/GPU-STUDIO.md).
See [editor controls and current limits](docs/EDITOR-UI.md).

Five supplied projects introduce the workflows:

| Project | Start here for |
|---|---|
| examples/scenes/v5-surface.pelagic | Waves, sea state, floating vessel and harbour |
| examples/scenes/v5-robotics.pelagic | ROV, boat, drifter, fish, tracers and controls |
| examples/scenes/v5-ecology.pelagic | Configurable animated schooling |
| examples/scenes/v5-sensors.pelagic | Sonar, motion/navigation, latency, recording and camera export |
| examples/scenes/v5-goa.pelagic | NOAA ETOPO1 regional terrain, atlas and provenance |


## Realistic ocean and the real Earth

- **Sea and sky.** The viewport draws the physics waves exactly, adds FFT short waves and whitecaps calibrated to observed coverage, and uses a physically based sky, clouds and the real sun position. It has vessel wakes, underwater light shafts and Snell's window. See [Ocean rendering](docs/OCEAN-RENDERING.md).
- **Real places.** **Earth → Create ocean scene here** (or right-click) builds a scene from NOAA's seafloor mosaic, the terrain tiles and UNEP-WCMC reef and seagrass maps. It adds an 82 km backdrop of real coast and mountains, the live sea and the sun for that hour.
- **Marine layers.** **Earth → Layers** switches each layer on or off:
  - reference: shipping lanes, ports, lighthouses and seamarks, maritime boundaries, islands, sea names and undersea feature names;
  - live: waves, wave buoys and AIS ships;
  - other: marine-life records and your own fleet, with tracks, predictions and closest-approach warnings.

  See [Marine layers](docs/MARINE-LAYERS.md).
## Vehicle testing and marine operations

Tabs beside the viewport and Earth view, each with a command-line twin for
batch and CI use. They are planning, training and test aids built on the same
physics as the simulation; each guide states its method, sources and limits.

| Tab | What it answers | Command line | Guide |
|---|---|---|---|
| Trials | Free decay, speed, turning circle, zig-zag, stopping, wave response; IMO MSC.137(76) checks | `--trials` | [Trials](docs/TRIALS.md) |
| Seakeeping | Limiting wave heights against motion criteria; go/no-go outlook from live forecasts | `--seakeeping`, `--forecast` | [Seakeeping and live seas](docs/SEAKEEPING-AND-LIVE-SEAS.md) |
| Search & Rescue | Leeway drift of a person or craft; search plans ranked by probability of success | `--sar` | [Search and rescue](docs/SEARCH-AND-RESCUE.md) |
| Survey | Multibeam or side-scan lines, coverage and IHO S-44 checks, flown with vehicle physics | `--survey` | [Survey](docs/SURVEY.md) |
| Ship Handling | COLREGs encounters at the helm or with a reference autopilot; rule-by-rule debrief | `--colregs` | [COLREGs](docs/COLREGS.md) |
| Inspection | Camera passes over pilings or a hull, imaged to a sharpness requirement, flown by an ROV | `--inspect` | [Inspection](docs/INSPECTION.md) |
| Spill | Oil spill drift, evaporation and landfall under live wind and current | `--spill` | [Spill drift](docs/SPILL.md) |

Vehicles are defined or imported as `.pelagic-vehicle.json`
([vehicle definitions](docs/VEHICLE-DEFINITIONS.md)). Weapon and targeting
functions are outside the product's scope.

[Worked tutorials](docs/TUTORIALS.md) cover editing, navigation, saved runs, replay,
batch execution and portable dataset export. [SDK](docs/SDK.md) documents Rust,
the C ABI and the MAVLink 1 subset. [Project formats](docs/PROJECT-FORMAT.md) define
units, schemas, bounds and checkpoint semantics.

The engine supplies queryable spectral/regular waves, prescribed currents,
depth-dependent water properties, regional bathymetry, approximate hydrodynamics,
actuators/controllers, bounded contact proxies, schooling and synthetic sensors.
The CPU renderer shares this world and depicts geometry, water, lighting and
visibility at three quality tiers. These are **interactive reference models**,
not a globally resolved ocean, calibrated CFD solver, validated fish species or
certified engineering predictor. Read [model validation](docs/MODEL-VALIDATION.md)
before interpreting results quantitatively.

Native UI Automation exposes named controls, focus, invocation, editable values
and status; keyboard workflows, light/dark themes and reduced motion are included.
Projects use atomic saves, bounded history and recovery. Recordings capture world
and sensor checkpoints and a command timeline; verification compares truth and
observations. A bounded writer preserves failed recording backups for retry.
Minimizing pauses interactive simulation/rendering; headless jobs continue.

The [Phase 5 report](docs/PHASE-5.md) and machine-readable evidence contain the
release checks, CPU/no-GPU timings, long-session allocation measurements and known
limits. Historical phase reports remain earlier baselines.

Build/check/package instructions are in [BUILDING.md](docs/BUILDING.md). The CI
workflow runs formatting, strict Clippy, release tests, packaging and native smoke
checks. Hosted CI results are not implied by the existence of the workflow.
See [contributing](CONTRIBUTING.md), [changelog](CHANGELOG.md),
[maintenance](docs/MAINTENANCE.md) and [credits](docs/CREDITS.md).

Engine/studio source: Apache-2.0. Dependencies retain their upstream licence texts.
The supplied Pelagic brand artwork remains the project owner's artwork.
# pelagic
