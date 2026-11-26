# Engine and studio contract

Baseline: 15 November 2026. Scope authorised by the request to build through Phase 2. The five-phase v1 scope remains in ROADMAP.md. This document freezes implementation boundaries; user acceptance of final visual quality remains a review of the actual product.

## Ownership and extension

The Rust core owns time, typed scenes, environment queries, physical entities, sensors, recordings and rendering. Serde handles versioned data. The native Windows host owns dialogs/audio, input, document history/recovery and presentation. No native window types cross into the simulation API. Headless examples and desktop sessions use the same `Scene`, `World`, `Ocean`, `SeaState` and `SoftwareRenderer`. See PHASES-3-4.md and SDK.md for the current 0.4 additions; this document retains the original foundation contract below.

The UI thread owns one model/document/session. Win32 callbacks access a `RefCell<App>`; reentrant callbacks do not create simultaneous mutable references. Environment imports run in bounded background jobs, return owned grids over a channel, and install only on the UI thread. Cancel drops the receiver; bounded parsing may finish in the worker but cannot install stale results. Project replacement cancels its pending import. Renderer and physics are currently serial on the UI thread: higher quality can delay interaction, so real-time throughput is not implied at every tier.

Imported grids use `Arc` in edit history; changing spacing uses copy-on-write. History is capped at 100 entries. Project/recovery input is capped at 16 MB; terrain import at 8 MB and 257² samples. Current-grid adapters cap at 4,194,304 vector samples. Core render allocation is bounded; desktop frame sizes cap at 1120 px wide / 900 px high. Typical renderer state is tens of MB, not a guaranteed total process-memory ceiling. Benchmarks must report resolution and warm/cold state.

Rust traits (`Bathymetry`, `CurrentField`, `Renderer`) are extension boundaries, with `Send + Sync` environment sampling. In-process extensions compile against the same source/API version; no stable binary Rust plugin ABI is promised. Public numerical data must remain finite and constructor invariants must be preserved. Build 0.4 delivers timestamped records, a tested C ABI host and a MAVLink subset/UDP check. MAVLink was selected for the first lightweight robotics transport; ROS 2 remains an additional adapter, not a core dependency.

## Time and geometry

Core time is integer ticks times fixed dt. Desktop dt is 1/60 s. Play uses elapsed wall time, capped at 250 ms catch-up per UI event; a suspended window does not promise real-time experiment execution. A paused Step advances one tick. Model-specific sensor/controller schedules and durable run replay arrive in Phase 4. Snapshots capture truth, not a saved experiment.

Terrain queries use bilinear interpolation and clamp outside the grid. Rendering resamples that field at quality-dependent mesh density; this introduces mesh approximation error. Static boxes are shared geometric objects with a ray intersection API; contact resolution is Phase 3. Georeferencing is local spherical/equirectangular and unsuitable near the poles or over large regions. There is no atlas or datum conversion in this build.

JSON schema v3 includes seed, units, coordinates, model labels and terrain source category. Imported terrain is bundled in the document, avoiding missing external file references. Detailed observation provenance (time, calibration, uncertainty, licence/source identifier) and portable experiment assets are reserved for Phase 4. No silent load of unknown future schemas.

## Screens and design system

Geist is the app typeface; the supplied logo remains artwork, since no original logo font file was supplied. Light uses white/near-white surfaces and black primary actions; dark uses black surfaces and white primary actions. Borders are thin neutral greys, focus is visible, disabled controls remain legible, and hover/press easing settles and stops its timer. Native chrome remains black with a white icon. The Windows client-animation preference disables startup/hover motion and updates on settings changes. UI Automation/screen-reader support remains release-hardening work, rather than being claimed as complete accessibility.

The layout uses three stable regions: scene hierarchy left, largest available viewport centre, context inspector right, with a compact action bar and status footer. The supported minimum client size is 900×620. Hit-target bounds and overlap are checked at that size, 1100×720 and 1920×990, across environment tabs/views.

| Screen | Interaction contract |
|---|---|
| Startup | Supplied logo animation and four-second fading cue; transition to hub; sound preference persists |
| Hub | New/Open, current session, up to four recent paths, recovery, reference-scene preview; explicit empty state |
| New Project | Name, absolute parent folder, empty/entity starting option, clear validation; creates one project directory |
| Editor | Hierarchy selection, context inspector, surface/underwater/section views, camera navigation, Play/Step/Reset, save/history |
| Environment tools | Menu chooses water/waves/flow/terrain/profile; validated numeric edits; presets/import and clear pending/cancel states |
| Entity tools (Phase 3) | Keep the same hierarchy/inspector; add asset browser, placement, transforms, duplication and gizmos |
| Sensors (Phase 4) | Select attached sensor beneath entity, inspect mount/rate/model; viewport overlays distinguish truth from observations |
| Experiments/results (Phase 4) | Dedicated run/results workspace with configuration, recording, timeline and plots; never confuse temporary Play with saved runs |
| Settings | Hub theme/sound persist now; full preferences, bindings, accessibility and diagnostics expand in Phase 5 |

The first ten minutes: create a coastal project, orbit and inspect it, change Hs/Tp, play/pause/step, inspect the section/profile, switch underwater, save, return home and reopen. A CSV import is optional. Undo and recovery protect committed scene edits. Sun/material/quality/camera changes are visual; physical environment changes reset the preview. All changes remain undoable and persistent where appropriate.

## Numerical acceptance

Wave dispersion compares against ω²=gk tanh(kd) across 0.5–1000 m depths; relative residual below 1e−12. Analytic spatial/time derivatives compare against centred finite differences. Discrete variance sums to Hs²/16; long sampling has a separate finite-record tolerance. Current grids reconstruct affine XYZ/time fields with 1e−12 tolerance and clamp boundaries. Terrain and coordinate tests check interpolation, bounds and round trips. Repeatability checks use one build/platform. Field accuracy requires independent datasets and model-specific validation in later phases.
