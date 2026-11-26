# Project, recording and dataset contracts

All lengths are metres, time seconds, mass kilograms, force newtons, torque N m.
World coordinates are right-handed X east, Y north, Z up. Quaternions rotate body
to world. UI headings are degrees; math/physics angular velocities are rad/s.
The C header and MAVLink guide document their own explicit boundary conversions.

## `.pelagic` project

UTF-8 JSON, `format: "pelagic-project"`, `version: 5`. A document is capped at
16 MiB; committed embedded assets are capped below this to leave write headroom.
`src/scene.rs` is the normative decoder and validator. `Scene::value` is the
canonical writer. Versions 1–4 migrate through this decoder; future versions fail
with a version error. Unknown optional JSON keys do not grant execution rights.
Projects contain data, never executable plugins or scripts.

Top-level groups:

| Key | Meaning |
|---|---|
| name, engine_build | Project display name and engine source fingerprint |
| scene | Density, nominal bottom depth, prescribed current, attenuation, reference visibility/depth and seed |
| camera | World position and target; saved view, never simulation state |
| environment | Wave height/period/direction/spread, terrain/material, depth profiles, shear/tide, sun, quality and optional embedded height grid |
| structure | Harbour box geometry enabled flag, position and scale |
| reference | Fully specified reference Entity; physical runtime ID is zero |
| body_model | Legacy mirror of reference.model; conflicting copies are rejected |
| entities | Additional unique positive-ID bodies; kind 0 ROV, 1 boat, 2 drifter; transform, physical/controller/actuator properties and optional `asset`/`material` library handles |
| library | (v5) Embedded mesh assets `{id, name, mesh, proportions}` and materials `{id, name, base_color (sRGB), roughness, metallic}`; see [asset library](ASSET-LIBRARY.md) |
| school, particles | Behaviour parameters/population and tracer count |
| sensors | Attached entity, range/fan/beams/rate, mount, tilt, latency, FOV and noise |
| flow_grid, provenance | Optional embedded XYZ/time flow lattice and explicit source/licence/geographic origin metadata |

Height grids have columns, rows, origin, spacing and row-major elevations (Z up).
CSV bathymetry and ESRI inputs are converted on import; flow velocities are SI.
Full budgets and validation ranges are in `Scene::validate`: at most 16 bodies,
256 fish, 512 tracers, 257×257 terrain samples and 131,072 flow samples. OBJ import
is geometry-only (20,000 vertices / 40,000 triangles per mesh; library: 32 meshes,
24 materials, 160,000 vertices), with contact represented by the entity's declared
physical box, not an arbitrary triangle collider. Handles that do not resolve in
the library are rejected. v4 per-entity `mesh` objects migrate into the library on
load (identical meshes share one asset) and v5 is written on save.
No project depends on an external absolute asset path after successful import.

Document saves serialize to a synced sibling temporary file and replace the
destination atomically. The old valid document remains if replacement fails.
Undo/redo retains at most 100 committed scenes; large immutable grids/meshes share
storage. Recovery and preferences are under `%LOCALAPPDATA%\Pelagic`.

## `.pelagic-run` recording

UTF-8 JSON, `format: "pelagic-run"`, `version: 1`; maximum 64 MiB and 1,200 samples.
Includes engine version/build, units, coordinates, original project JSON, fixed dt,
optional starting world checkpoint, optional sensor checkpoint, command events and
ordered frames. The optional sensor checkpoint was added compatibly during Phase 5.
It retains previous velocity, pending latency queue and last delivered measurement.

Frames store full truth snapshots, world velocities and delivered observations;
sample and available times remain distinct. A command applies at its integer tick
before the next integration step. Playback selects saved frames without stepping
physics. Verification restores the initial state, reapplies commands and compares
truth, velocity and (when a sensor checkpoint exists) observations exactly.
Malformed, oversized, unordered or non-finite records fail validation. Build
differences can invalidate re-simulation without invalidating saved truth playback.

## Dataset bundle

A new directory contains `manifest.json`, `scene.pelagic`, `experiment.pelagic-run`,
`truth.csv`, `sonar.csv`, `navigation.csv`, `fish.csv` and `commands.csv`. Camera
export adds at most 120 binary PPM frames plus calibration/pose/time metadata at
320×200; it is a deliberately bounded export, not a video recorder. Pixel centres
are at n+0.5 from the upper-left image corner. A failed/cancelled export keeps an
`INCOMPLETE.txt` marker. Choose a new destination; exports do not overwrite a
previous dataset. Source provenance, coordinate conventions and illustrative-model
status travel with the bundle. Manifests describe files; distribution SHA-256
manifests are a separate package-integrity layer.
