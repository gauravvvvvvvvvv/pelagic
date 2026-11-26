# Model validity and numerical acceptance

Pelagic is an interactive simulation engine. Its reference models support control
software development, repeatable sensor/software experiments, teaching and scene
authoring. They do not establish real vehicle performance, collision survivability,
ecological outcomes or operational safety. No tank, sea-trial or species dataset
was used to fit the supplied vehicle or fish coefficients.

| Model | Comparison and acceptance | Supported interpretation / failure range |
|---|---|---|
| Regular waves | Analytic height/period/direction; finite differences of the analytic surface; dispersion residual <1e-12 | Linear finite-depth Airy waves. No breaking, refraction, shoaling or wave–wave interactions |
| Spectral sea | Exact discrete variance Hs²/16; deterministic 32 components; finite-record variance within the stored regression tolerance | JONSWAP-shaped finite spectrum at nominal depth, not an FFT/global hydrodynamic solver |
| Currents | Affine XYZ/time reconstruction and boundary clamping | Prescribed flow, not fluid feedback or turbulence. Out-of-grid queries clamp to edges |
| Water | Finite bounded profiles and query consistency | Approximate temperature/salinity density and sound speed; not TEOS-10 or ray-bent acoustics |
| Neutral rigid body | 216,000 steps / 1 simulated hour at 60 Hz; position drift <1e-8 m | Uniform still water, balanced weight/buoyancy, no contacts/commands |
| Constant force | 30/60/120/240 Hz, 10 s, analytic velocity error <1e-10 m/s; position error ≤5/hz m and halves on refinement | Semi-implicit first-order translation. Halve dt and compare every new physical study |
| Quadratic drag | 120 Hz, 120 s; 16 N against coefficient 4 N/(m/s)² gives terminal speed 2 m/s within 0.01 m/s | Diagonal body drag, approximate implicit damping. No calibrated lift or propeller wake |
| Buoyancy/control | Stored settle, immersion, depth/heading hold, lag/saturation and added-mass tests | Sampled box immersion and diagonal added mass; no general calibrated 6×6 Fossen matrix |
| Contact | Seabed nonpenetration, proxy overlap removal, pair momentum checks | Discrete expanded-box/bottom proxies, ≤120 Hz substeps; no CCD, angular manifold or deformable structures. Fast/thin-object impacts can tunnel |
| Schooling | Deterministic seed, depth bounds, finite speed ≤1.5×configured swim speed; 36,000-step/10-minute test including structures | Illustrative pelagic schooling fish, not a fitted species. Separation/vehicle avoidance are steering preferences, not guaranteed fish-to-fish/vehicle clearance. Static boxes additionally project overlapping centres |
| Sonar | Stored flat-bed/obstacle geometry, range/travel-time and seeded-noise cases | Straight rays, 0.5 m bottom marching, simple reflection/attenuation. No multipath, reverberation, shadowed volumetric targets or fisheries acoustics |
| Motion/navigation | Seeded Gaussian noise, fixed-tick sample rate, delivery latency, specific-force and coordinate tests | Quaternion orientation is truth; basic IMU/navigation illustrations, not a full stochastic INS |
| Camera | Saved intrinsics/extrinsics, FOV and roll cache invalidation, read-only shared CPU renderer | Synthetic appearance with approximate lighting; no calibrated lens, rolling shutter, radiometry or camera noise |
| Record integrity | JSON round trip, mid-run world + sensor checkpoints, command timeline, exact truth/velocity/observation re-simulation, altered-observation rejection | Exact recorded truth playback is portable; bitwise re-simulation is a same-build/platform check. Legacy recordings without sensor checkpoints verify truth only |

Test sources are `tests/engine.rs`, `environment.rs`, `fields.rs`, `phase34.rs` and
`release_acceptance.rs`. Inputs, bounds and analytic comparisons live alongside
the assertions. Release packages include these sources and machine-readable
measurement reports, so a failed tolerance cannot be hidden behind a test count.
The renderer's accelerated regular-grid wave queries are also compared against
individual analytic queries at 0, 13.5 and 3,600 seconds: height, normal and vertical
speed differences must stay below 1e-10. Physical integration uses the point queries.

The ordinary desktop dt is 1/60 s. Contacts subdivide to at most 1/120 s. Desktop
catch-up is capped at 0.25 wall seconds per callback; exceeding it reports time
limiting and slows the simulation rather than enlarging physical dt. Quality tiers
change display sampling only. Minimized playback pauses; headless batch continues
independently. A long-run drift result in still water is not evidence for stability
under every possible force or bathymetry input.

Before using a new model quantitatively: measure the real body's mass, volume,
inertia, buoyancy centre and thrust/drag curves; fit coefficients; reserve validation
data; compare histories and uncertainty bounds; refine dt; then validate against
independent experiments. Use a higher-fidelity solver when the omitted effects
control your result. The published numbers are numerical checks, not field error bars.

Model background: [Fossen marine dynamics](https://www.fossen.biz/html/marineCraftModel.html),
[Reynolds steering](https://www.red3d.com/cwr/steer/). These describe the model
families; matching their full generality is not claimed by this implementation.
