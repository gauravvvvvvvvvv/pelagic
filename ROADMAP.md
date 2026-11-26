# Pelagic: five-phase product plan

**Superseded scope, 15 November 2026:** the user has expanded the product to a
professional simulation/game editor with CPU/GPU tiers, executable node graphs
and experimental learning extensions. This document retains the earlier baseline;
it does not establish completion of the expanded product. Current direction is in
[PRODUCT-DIRECTION.md](docs/PRODUCT-DIRECTION.md). The detailed local feature and
acceptance backlog is maintained separately from release payloads.

Planning date: 15 November 2026. Status: five-phase delivery baseline; bounded implementation through Phase 4 available in build 0.4. See [Phase 3/4 delivery and open acceptance items](docs/PHASES-3-4.md). Implementation does not automatically sign off aspirational phase gates or scientific/visual acceptance.

## Outcome

By the end of Phase 5, Pelagic v1 is a complete, installable native Rust ocean simulation engine and desktop studio. A user can explore an ocean environment, build a scene, simulate waves and marine entities, operate vehicles and sensors, record experiments, and reopen or share a reproducible project. The application has a coherent editor and convincing ocean graphics, and works through a CPU rendering path without requiring a GPU.

Phase 5 is the release and acceptance phase, not the point where essential functionality starts. The end-to-end feature set is integrated by Phase 4. Phase 5 makes that integrated product reliable, measured, documented and distributable.

The long-term audiences include marine robotics, research, education, maritime testing, interactive applications and exploration. A common engine supports them; application-specific integrations do not dictate core physics or rendering.

## What exists today

The native studio now has project creation, versioned environment documents, camera navigation, history and recovery. The shared engine includes linear/spectral waves, terrain grids, current/profile queries and a depth-tested CPU mesh renderer with surface/underwater/section inspection. The old sphere/checkerboard presentation has been replaced by procedural terrain and a primitive ROV visual model.

Build 0.4 adds proxy contacts, partial buoyancy, diagonal hydrodynamic/actuator models, controllers, procedural vehicle/fish geometry, schooling/tracers, OBJ import, mounted camera/sonar/navigation/IMU outputs, recorded experiments, playback, dataset export, bounded regional/current loaders, a coarse NOAA overview, a Rust SDK/C ABI and a tested MAVLink subset. Production asset quality, model qualification, broader accessibility and large-world release engineering remain open. Implementation and mathematical tests do not constitute field validation or user acceptance of graphics.

## Product contract

### Mandatory architecture

- Rust implementation for the engine, renderer and desktop application. Native operating-system APIs are allowed for windows, audio, file dialogs and packaging.
- No Python runtime, web frontend, browser or WebView is required to run the product.
- Headless simulation, desktop authoring and rendering use the same scene and engine models.
- Physics, rendering, environment sampling, entity behaviour, sensors, project storage and application UI have explicit module boundaries.
- CPU operation is a tested baseline, including visible rendering. Optional acceleration may improve throughput; it must not be required for the supported reference workflow.
- Simulation time and numerical fidelity are independent of UI frame rate. Visual quality settings never silently alter an experiment's physical model.
- Every saved run identifies its scene, model versions, units, coordinate conventions, seed and relevant data sources.

### Meaning of realistic

Realism has three separate acceptance tracks:

1. Visual: recognisable ocean surfaces, underwater lighting, terrain, vehicles and marine life with coherent scale and motion.
2. Physical: stated equations and approximations, numerical checks, benchmark comparisons and recorded error under specified conditions.
3. Behavioural: fish and other entities respond plausibly to currents, obstacles and nearby entities, with controllable parameters and documented limits.

Visual quality does not establish physical validity. Realistic fish animation does not establish that population ecology is predictive. Each model must be labelled as illustrative, approximate or validated for a stated operating range.

"Everything realistically" is a product vision, not a measurable release criterion. V1 covers the systems and scenarios listed here. It does not promise a resolved global ocean, all species, full CFD, or guaranteed accuracy for every environment.

### Supported v1 envelope

- Local and regional surface/underwater scenes, with explicit scene bounds and data resolution.
- Surface waves, spatial current fields, depth-dependent water properties and configurable visibility.
- Seabed terrain and placed static structures.
- Surface and submerged reference vehicles, passive drifters/particles, and representative marine-life entities.
- A documented reference fish species/model with schooling, individual movement and environmental interaction; further species share the extension interface.
- Camera, basic sonar and motion/navigation sensors with documented limits.
- Scene authoring, simulation control, recording, replay, export and a Rust SDK.
- Geographic exploration using available map/bathymetry data with resolution and source information. Global overview coverage, where suitable data is available, is distinct from locally surveyed detail.

At the start of Phase 1, freeze a reference computer and three benchmark scenes: a small learning scene, a vehicle experiment and a larger ecological/environment scene. Publish scene dimensions, entity counts, terrain resolution, sensor rates and quality settings. Feature/performance acceptance applies to those declared scenarios; it is not an unlimited-world guarantee.

## Phase 1 — Product definition and durable engine/editor foundations

### Purpose

Agree on the finished experience and establish an architecture that can support the entire product without repeatedly redesigning the application.

### Deliverables

**Design and workflows**

- Complete screen and interaction specification: startup, project hub, project creation, editor, environment tools, entity tools, sensors, experiment results and settings.
- A small consistent design system: Geist typography, light/dark palettes, subtle borders, button sizes, inputs, menus, tabs, dialogs, validation, focus, hover and press motion.
- Desktop layouts that use window size deliberately; no stretched low-resolution UI, scattered controls or decorative filler.
- Decide the dock/panel layout, viewport priority, scene hierarchy and inspector interaction before building further panels.
- Define onboarding and the first successful ten-minute workflow.
- Retain the supplied logo and four-second startup cue. Confirm a reusable UI typeface separately if the original logo font becomes available.

**Application foundations**

- Audit the existing shell and keep or replace components based on layout, accessibility, maintainability and platform needs.
- Reliable keyboard/text input, clipboard, selection, focus, standard shortcuts, context actions and readable disabled states.
- New/Open/Save/Save As, project asset folders, schema migrations, recent projects, preferences and clear unsaved-state handling.
- Undo/redo as reusable commands; recovery that does not overwrite unrelated work.
- Documented distinction between scene authoring, temporary simulation runs and saved run results.
- A single packaged executable/launch path, visible build identification, useful errors and diagnostics.

**Engine foundations**

- Typed scenes and entities, stable identifiers, transforms and units/coordinate conventions.
- Deterministic fixed-step scheduling, model-specific update rates and explicit initial conditions.
- Shared terrain/entity geometry interfaces for renderer, physics and sensors.
- Rendering snapshots and query interfaces that do not couple simulation speed to presentation.
- Interfaces for environment fields, entities, controllers, sensors and data adapters.
- Establish thread ownership, background jobs, cancellation, memory budgets and asset lifetime rules.
- Define safe extension boundaries. In-process Rust extensions must have an explicit compatibility policy; a stable cross-version dynamic ABI must not be assumed.

**Measurement**

- Select reference hardware and freeze benchmark scene specifications.
- Record startup time, UI latency, rendering frame time, simulation-step time, memory and idle CPU behaviour.
- Define numerical tolerances, reproducibility scope and the validation dataset/benchmark needed for each later model.

### Exit gate

A user can create, configure, save, close and reopen a reference project without losing scene or camera settings; navigate it without input/layout failures; undo/redo edits; and recover committed unsaved changes. Headless and desktop runs use the same scene. The five-phase scope, reference scenes and UI specification are reviewed and frozen.

### Explicit exclusions from this gate

The debug renderer and current vehicle proxy do not qualify as finished graphics. Prototype tests alone do not qualify the application as a scientifically validated simulator.

## Phase 2 — Ocean world and production rendering foundation

### Purpose

Make the environment look and behave like an ocean, with shared physical sampling rather than a decorated empty viewport.

### Deliverables

**World representation**

- Tileable/gridded bathymetry, terrain bounds, interpolation and spatial acceleration.
- Terrain materials for sand, rock and representative seafloor variation.
- Static geometry for coastlines, simple harbour/underwater structures and obstacles.
- Defined local/geographic coordinate conversion and origin handling.
- Spatial/time-varying currents and temperature/salinity/density profiles.
- Environment queries for surface height, normal, flow velocity, water properties and seabed depth.

**Waves and sea states**

- A bounded analytical wave model first, followed by an appropriate spectral sea-state implementation within this phase.
- Controllable wave height, period, direction, spreading and wind-driven sea-state parameters.
- Consistent surface displacement, normals and water velocity sampling for physics and rendering.
- Deterministic seeded sea states, boundary behaviour and documented depth/dispersion assumptions.
- Compare generated wave statistics and dispersion against declared reference cases.
- Define approximations for shallow-water effects, shoreline transitions, breaking and foam; implement only the agreed v1 range and show its limits.

**Rendering**

- Replace the sphere/checkerboard reference presentation with geometry, materials, proper depth testing and lighting.
- Surface and underwater camera transitions, sun/sky environment, water colour, absorption, scattering approximations and visibility.
- Surface reflections/refraction approximations, coherent highlights and restrained foam where supported.
- Terrain/entity visibility, contact/depth cues, basic shadows and deliberate anti-aliasing/upscaling.
- Efficient CPU rendering with quality tiers, spatial culling, level of detail and reusable buffers.
- Render resolution follows the viewport/quality budget; it is no longer a fixed 320-by-200 image stretched across the screen.
- Visual effects must remain stable in motion. Optional caustic/volumetric effects need an explicit CPU cost budget.
- Separate physical environment settings from display-only settings.

**Authoring**

- Environment inspector for water profiles, current fields, visibility and sea state.
- Terrain import/generation workflow, material controls and viewport overlays.
- Useful surface, underwater and section/inspection views with consistent navigation.

### Exit gate

A saved scene can show a convincing surface and underwater environment on the CPU baseline. Waves, flow and terrain are queryable through the Rust API and match the rendered world within documented tolerances. Visual and numerical wave tests pass. The small benchmark scene meets its declared performance budget.

## Phase 3 — Physical entities, vehicles and marine life

### Purpose

Populate the ocean with entities that interact meaningfully with the environment and each other.

### Deliverables

**Physical simulation**

- Robust rigid-body transforms and integration, collision shapes, broad/narrow-phase contact and seabed/structure interaction.
- Buoyancy based on geometry or documented sampling, centres of mass/buoyancy and surface-crossing behaviour.
- Relative-water drag and lift approximations where needed.
- Submerged-vehicle hydrodynamics including added-mass and relevant coupling terms within the selected model.
- Thruster allocation, actuator limits, response lag and configurable mass/inertia/drag parameters.
- Reference underwater vehicle and a simple surface vessel with actual visible geometry.
- Appropriate wake/disturbance approximations, with cosmetic wakes separated from physical effects.
- Passive floats, drifters and bounded particle advection/dispersion.
- Controller interfaces and simple reference controllers for depth and heading/station tests.

**Marine life**

- Reference fish geometry, materials, scale and articulated/swimming animation.
- Individual motion and schooling: separation, alignment, cohesion and bounded turning/acceleration.
- Flow response, terrain/obstacle avoidance and declared depth/region preferences.
- Configurable responses to nearby vehicles or disturbances, explicitly labelled as behavioural models.
- Several authored school configurations and at least one representative animated benthic/vegetation asset where it contributes to the reference environment.
- Deterministic behaviour seeds, spatial indexing, animation/behaviour LOD and entity budgets.
- Behaviour controls and scene editing for schools and individual entities.
- The API permits later species and behaviour extensions without requiring a new engine.

**Authoring and assets**

- Asset browser, entity placement, duplication, deletion, transform tools and property validation.
- A bounded asset pipeline with documented geometry/material/collision requirements.
- Undoable authoring operations, scene hierarchy selection and visible gizmos.
- Save all entity setup and environmental interactions in the project.

### Exit gate

The vehicle benchmark runs with waves/currents and contact; the ecological benchmark contains moving schools that avoid terrain and respond to the configured environment. Rendered geometry and physical/sensor geometry remain consistent. Buoyancy, drag, integration and controller benchmarks pass within stated tolerances. Behaviour is tested separately from appearance.

## Phase 4 — Simulation workflows, sensors, real data and integration

### Purpose

Complete the end-to-end product so it supports experiments, real locations and other applications—not only scene viewing.

### Deliverables

**Sensors**

- Synthetic camera output with explicit intrinsics, pose, timing and underwater optics settings.
- Initial multibeam/imaging sonar with beam/range structure, geometric target returns and documented attenuation/backscatter/noise approximations.
- Sound-speed/depth effects within the selected propagation model, plus travel time and configurable communication delay.
- Basic depth, orientation/motion and positioning-style observations with configurable noise and sampling rates.
- Sensor mounting, visualisation, overlays and separate physical truth versus noisy observations.
- Sensor benchmarks and comparison datasets, with accuracy and limitations published per model.

**Experiments and reproducibility**

- Run configurations, initial-state reset, seeds, independent sensor/model rates and controller scheduling.
- Record commands, environment/model configuration and observations.
- Saved runs, replay/scrubbing, selected-state inspection and result plots.
- Clear guarantees for replay versus re-simulation; display divergence when guarantees are not met.
- Batch/headless execution, cancellation, progress and repeatable outputs.
- CSV and a documented scientific data export; image/sensor dataset export where implemented.
- Include source data, model versions, coordinate reference, units, seed and validation status in exports.

**Real-world data and atlas foundation**

- Audit candidate public bathymetry and oceanographic sources for coverage, formats, resolution, provenance and redistribution needs.
- Implement a bounded set of supported loaders and cache formats, with resampling and explicit missing-data handling.
- Geographic region selection, map/globe overview and local scene generation.
- Global overview coverage from suitable available datasets; detailed simulation uses selected regions and a declared resolution.
- Distinguish measured, interpolated and simulated data visually and in metadata.
- Package at least one real-data regional demonstration and one synthetic scenario.
- Offline project bundles with required assets/data references, so a shareable project is more than a small file pointing at unavailable dependencies.

**SDK and integrations**

- Documented Rust library APIs and example programs for creating scenes, querying the ocean and stepping/running experiments.
- A controller/data interface with explicit timestamp, coordinate, command and observation contracts.
- One demonstrated robotics integration and one external application/engine integration selected during Phase 1.
- Integration adapters may use the external system's supported transport; no adapter makes a non-Rust runtime mandatory for the core desktop product.
- Versioned extension examples, compatibility tests and failure reporting.

**Complete desktop workflows**

- Sensor setup, run manager, recording/replay, results inspection and exports are accessible through the editor.
- Searchable scene/assets, visible loading/error states, useful defaults and guided reference projects.
- Packaging/updates, migrations, project portability and recovery are exercised with realistic project sizes.

### Exit gate

A new user installs Pelagic, opens a regional ocean scene, adds a vehicle and fish, changes the sea state, runs a controller/sensor experiment, saves a recording, replays it and exports the results. Another user can open the shared project bundle and reproduce the declared experiment. The same workflow can be run headlessly. All essential v1 capabilities are now integrated.

## Phase 5 — Validation, performance, polish and complete v1 release

### Purpose

Turn the integrated engine/studio into a dependable product with stated accuracy, supported limits, usable documentation and a repeatable release process.

### Deliverables

**Scientific and numerical acceptance**

- Re-run every accepted wave, current, motion, buoyancy, collision and sensor benchmark from stored inputs.
- Publish the model assumptions, comparison conditions, numerical errors and known failure ranges.
- Check step-size sensitivity, stability, reproducibility and long-run drift.
- Verify schooling/avoidance invariants, deterministic behaviour and species-model limits without overstating ecological validation.
- Document which intended studies fit the models and which require a higher-fidelity solver or real-world testing.

**CPU and memory acceptance**

- Measure all three reference scenes on the declared CPU baseline, including the no-GPU rendering path.
- Profile simulation, sampling, behaviour, sensors, rendering, UI, I/O and memory separately.
- Apply bounded concurrency, data-oriented storage, allocation control and scalable visual detail where profiling supports them.
- Validate quality tiers and overloaded-scene behaviour. Display performance limits rather than silently changing physical accuracy.
- Idle UI stops unnecessary animation; background/minimised rendering and jobs follow explicit policies.
- Long-session and larger-project tests detect leaks, unbounded queues and corrupted outputs.

**Product and release acceptance**

- Usability pass across creation, editing, input, camera movement, experiments, results and recovery.
- Keyboard, focus and assistive-technology support for the agreed desktop accessibility baseline.
- Check theme contrast, readable labels, display scaling, resizing, animation preferences and error messages.
- Replace remaining placeholders that appear in the user-facing product; keep diagnostic views explicitly marked.
- Windows release packaging and signing/distribution decisions are resolved. Additional OS support is advertised only after that platform passes its own acceptance suite.
- Complete the Rust SDK documentation, project format specification, integration guides and worked tutorials.
- Deliver curated projects: surface sea-state exploration, underwater vehicle operation, fish-school environment, sensor experiment and real-region exploration.
- Automated build/test/release checks, dependency/license inventory, asset credits, contributor instructions, issue templates and a versioned changelog.
- Establish maintenance, support and issue triage. The open-source engine remains usable independently of optional company services.

### Final release gate

All release checklist items below pass. There are no known blockers in save/recovery, scene editing, run integrity or the supported reference workflows. Unmet essential requirements block v1; they are not renamed as later phases to make the release appear complete.

## End-of-Phase-5 release checklist

| Area | Required v1 outcome |
|---|---|
| Native product | Rust engine and desktop studio; installation, startup, updates and one dependable launch path |
| Editor | Coherent panels, asset/scene tools, editable inspectors, camera controls, history, recovery and preferences |
| Ocean | Queryable waves/sea states, currents, depth-dependent properties, bathymetry and bounded regional scenes |
| Graphics | Actual terrain/entity geometry, coherent surface/underwater lighting and visibility, scalable CPU rendering |
| Marine life | Animated reference fish, schools, terrain/vehicle interaction and configurable behaviour |
| Physical entities | Surface/submerged vehicles, buoyancy, hydrodynamics, actuators, contact, floats and particles within supported bounds |
| Sensors | Camera, sonar and basic motion/navigation observations with timing, noise and model limits |
| Experiments | Controllers, recording, saved runs, replay, batch/headless execution and useful exports |
| Real data | Supported imports, provenance, region selection, overview atlas and portable project bundles |
| Developer use | Documented Rust SDK, extension examples and demonstrated external integrations |
| Trust | Reproducible numerical/physical benchmarks, accuracy statements, performance reports and known limitations |
| Delivery | Reference projects, tutorials, credits, release artifacts and a maintenance process |

## Performance plan

These are proposed starting targets to ratify against reference hardware in Phase 1, not claims about the current code:

- Reference vehicle simulation: 60 fixed steps/s; target p95 core step below 5 ms for the agreed scene, with sensors budgeted separately.
- Interactive CPU viewport: target 30 frames/s at the agreed baseline resolution and scene complexity. Higher quality may run at lower frame rates.
- Common UI feedback: visible response within 100 ms on the reference computer; long work is cancellable and does not block input.
- Hover/press transitions: approximately 120–180 ms, with reduced-motion handling and no continuous idle redraw.
- Small local project opening: target first useful viewport within 3 seconds; larger datasets show staged loading and honest progress.
- Freeze measurable memory/idle-CPU limits after recording the Phase-1 baseline. Do not invent power-consumption claims from FPS alone.

## How the phases depend on each other

Phase 1 defines contracts, UX and acceptance scenes. Phase 2 establishes the shared ocean world and renderer. Phase 3 adds interacting physical/biological entities. Phase 4 completes the real-data, sensor, integration and experiment workflows. Phase 5 validates and ships that integrated product.

Validation, performance measurement, project migration and UI consistency continue in every phase. They are not postponed until Phase 5. A feature arrives with its tests, documentation, saved-scene support and inspector workflow.

## Work deliberately outside the complete v1 scope

- Full high-resolution three-dimensional CFD for the entire world ocean.
- Verified detailed mapping of every seafloor location or real-time knowledge of all ocean conditions.
- Predictive models of every marine species, ecosystem or climate process.
- Unbounded fleets, schools, particles or sensor rates on arbitrary CPU-only hardware.
- A promise that synthetic observations automatically transfer to every real sensor or operational setting.
- Custom diver equipment manufacturing and a global crowdsourced observation network before the software/data interfaces have been demonstrated.
- Adoption commitments from any institution or claims of operational certification without the corresponding evaluation.

These are scope boundaries, not missing hidden requirements for Phase 5. If one becomes mandatory, change the v1 acceptance contract and its resource estimate explicitly.

## Observations, atlas growth and company work

V1 includes data import/provenance and an atlas foundation. The Phase-1 schema should reserve observation metadata such as time, position, units, calibration, uncertainty and source permissions. Phase 4 proves the ingestion path with existing observations; Phase 5 documents it. Building and distributing new collection hardware is a separate program with its own validation and fieldwork plan.

Company formation, support offerings, collaboration and funding can run alongside the five engineering phases. They do not substitute for product acceptance or assume any particular customer. Preserve a clear boundary between the open engine, datasets, third-party assets and optional hosted/support services.

## Execution rules

1. Approve this scope and the Phase-1 acceptance scenes before adding more features.
2. Break each phase into small demonstrable increments with a written exit checklist.
3. Review the actual application and measured engine behaviour, not only screenshots or test counts.
4. Prefer a coherent vertical slice over disconnected demonstration panels.
5. Maintain one backlog, separating defects, accepted phase scope and post-v1 ideas.
6. Set dates and staffing estimates after Phase 1 establishes the benchmark workload and architecture. Five phase labels do not imply a short schedule.
7. Stop for a phase review when its gate passes. Start the next phase from the agreed specification rather than inventing its UI during implementation.

## Immediate next action

Review build 0.4 against the [Phase 3/4 delivery contract](docs/PHASES-3-4.md) and [measured workloads](docs/BENCHMARKS-3-4.md). Phase 5 must close the recorded graphics, accessibility, model-validation, performance and distribution acceptance items; it remains the complete v1 release gate. The earlier reports remain historical baselines, not current feature inventories.

The Phase 5 implementation and current release evidence are tracked in
[PHASE-5.md](docs/PHASE-5.md). That report is the current acceptance record; the
paragraph above and phase-specific reports retain the earlier build history.
