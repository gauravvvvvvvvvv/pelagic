# Phase 1–2 delivery and acceptance

15 November 2026. This is an environment implementation and editor foundation, not the complete Phase 5 product. The original roadmap remains the v1 scope. This report distinguishes implemented software from unverified product acceptance.

## Delivered

Phase 1: native Rust shell, logo/audio startup, Geist, themes, visible chrome/icon, hub and new-project workflow, atomic saves, v1/v2 migration, v3 scene schema, recent projects, keyboard editing/navigation, bounded history, recovery, headless/desktop shared models and explicit ownership/extension contracts.

Phase 2: bilinear synthetic/imported bathymetry, explicit grid bounds, terrain normals, sand/rock materials, placed harbour geometry, local geographic conversion, regular linear waves and seeded directional spectra, orbital flow, depth/tidal currents, a Rust XYZ/time-grid current adapter, temperature/salinity/density/sound-speed queries, CPU depth-tested meshes, sky/sun, underwater attenuation, approximate water reflections/colour/foam/caustics, cached geometry/lighting, quality tiers, bilinear viewport reconstruction, surface/underwater/section views and persisted environment inspectors.

Terrain import uses a background worker with bounded input, cancellation and embedded asset storage. Scene/surface/flow queries use the same core objects as the renderer. Display settings leave physical state unchanged. First-frame rendering includes terrain-shadow and sky cache construction; steady-frame measurements exclude it.

## Reference workload

Reference workstation: Intel Core i5-11320H (4 cores, 8 threads), 16 GB RAM, Windows, Rust GNU release build, serial CPU rendering, no GPU requirement. Benchmarks are development measurements on this machine, not minimum-system qualification.

All three workloads use a synthetic 512×512 m region, 129×129 source elevations at 4 m spacing, a 32-component sea (Hs=1.2 m, Tp=6 s, heading 35°, spread 25°, seed 42), and one simplified submerged vehicle at 60 Hz. Small learning scene: coastal surface. Vehicle scene: underwater coast. Larger environment scene: canyon terrain, with ecological entities explicitly reserved for Phase 3. The benchmark omits the desktop harbour to isolate the common renderer/environment path.

`cargo +stable-x86_64-pc-windows-gnu run --release --example phase2` warms each renderer, measures 30 frames, sorts elapsed times, and reports rank 29 as p95. It also averages 600 fixed steps. This measures core rendering, not complete UI frame latency. Quality budgets: Low 512×320, Balanced 800×500, High 1120×700. Viewport aspect changes actual height; these are reproducible benchmark dimensions.

The small-scene target is p95 render time ≤33.3 ms at Low. Balanced/High are optional slower inspection tiers. The vehicle core-step target is <5 ms. First useful small-project viewport target is <3 s, excluding the deliberate four-second startup presentation. UI feedback target is <100 ms during normal low-quality interaction; high-quality rendering can exceed it. Idle animation timers stop; process idle/power and broad hardware qualification remain Phase 5 measurements.

See `BENCHMARKS.md` for the final measured results, commands and limitations.

## Scientific scope and checks

The regular model is Airy superposition. Spectra use 32 log-frequency bins from 0.5 to 2.5 times peak frequency, a JONSWAP-shaped energy curve with γ=3.3, triangular directional perturbations and deterministic phases. Energy is normalised to Hs²/16. This is a compact illustrative spectrum, not a complete spectral evolution solver. Constant nominal depth controls dispersion and orbital vertical structure; local seabed variations do not cause refraction/shoaling. Editor validation bounds nominal steepness/depth, but individual random crests and shallow shoreline cells can violate linear assumptions. Shoreline breaking/run-up and wetting/drying are not modelled.

Wind menu values derive initial Hs≈0.21 U²/g and Tp≈0.83 U for a convenient fully-developed-sea scale, then use the same compact spectrum. These presets do not track fetch, gusts, wind stress or evolving wave energy. The rendered mesh and far-field LOD approximate analytic samples; small waves below mesh resolution can alias. Bilinear reconstruction reduces pixel block edges but is not temporal anti-aliasing. No image-based validation against photographs is claimed.

The water profile blends temperatures around a configurable thermocline, uses a linear density estimate and an empirical sound-speed polynomial. Reference vehicle buoyancy uses constant density and the existing sonar uses constant sound speed. Profile coupling to advanced dynamics/sensors is Phase 3/4 work. Rendering extinction and scattering are illustrative. Static structure ray queries exist; collision response and geometry-aware sensors are later phases.

Primary references for the modelling boundaries: [TU Delft linear-wave theory](https://coastal.citg.tudelft.nl/book/2a_pre_knowledge_waves.html), [SWAN technical documentation](https://swanmodel.sourceforge.io/online_doc/swantech/node64.html), and [TEOS-10](https://www.teos-10.org/) (the thermodynamic standard this illustrative profile does **not** implement).

## Gate status

Automated acceptance covers save/reopen precision, schema migration/rejection, embedded grid round trips, invalid input, history/recovery, keyboard/controller functions, supported layout hit targets, fixed-step separation, numerical wave/terrain/current properties, deterministic rendering, and startup audio fade. Generated editor/surface/underwater/section previews are inspected from this program's own output; no user-screen capture is needed.

The bounded environment slice is packaged for review. It does not certify every aspirational Phase 1–2 roadmap criterion: user acceptance of visual quality, full accessibility support and broad UI latency/idle-memory qualification remain open. Reduced-motion startup/hover behaviour follows the Windows animation preference. Large-world tiling/streaming is outside this bounded regional slice and remains atlas work. Renderer geometry is resampled, single-region CPU graphics; it is not Unreal-quality production graphics. Keep open acceptance items visible instead of treating test counts as visual sign-off.

Phase 3 next adds contact, improved buoyancy/hydrodynamics, real vehicle assets and animated schooling fish. Phase 4 adds complete sensor/experiment/real-data workflows. Phase 5 remains the complete v1 acceptance and distribution gate.
