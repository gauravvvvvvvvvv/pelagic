# Phase 2 CPU measurements

15 November 2026, Intel i5-11320H / 16 GB RAM / Windows, GNU Rust release build. No GPU or external runtime used. Single development-machine run; background load, thermal state and clock variation affect results.

Command: `cargo +stable-x86_64-pc-windows-gnu run --release --example phase2`. Source: `examples/phase2.rs`. Each case contains one body, a 129² bathymetry source and 32 wave components. The world starts rendering after 600 fixed steps (10 simulated seconds), then advances two steps between measured frames. One warm-up render; 30 timed renders; p95 is sorted rank 29. No window painting, input, shadow-cache construction or first-frame sky setup is included in these frame times.

| Scene | Tier / resolution | Mean render ms | p95 render ms | Mean core step µs |
|---|---|---:|---:|---:|
| Coastal surface | Low / 512×320 | 20.46 | 22.06 | 5.31 |
| Coastal surface | Balanced / 800×500 | 49.77 | 56.49 | 5.31 |
| Coastal surface | High / 1120×700 | 79.37 | 88.52 | 5.31 |
| Underwater vehicle | Low / 512×320 | 18.20 | 18.92 | 4.80 |
| Underwater vehicle | Balanced / 800×500 | 52.14 | 59.68 | 4.80 |
| Underwater vehicle | High / 1120×700 | 78.78 | 94.22 | 4.80 |
| Canyon environment | Low / 512×320 | 24.38 | 26.07 | 5.85 |
| Canyon environment | Balanced / 800×500 | 67.21 | 83.18 | 5.85 |
| Canyon environment | High / 1120×700 | 103.51 | 116.05 | 5.85 |

The 33.3 ms small-scene **core render** budget passes at Low, which is the desktop default. The core-step budget passes comfortably for this one-body workload. These figures do not establish full-window 30 fps, UI responsiveness at every tier, or scaling to fleets/fish/sensors. Balanced/High explicitly trade throughput for detail and may block the serial UI while rendering; use Low for interaction on this workstation. No performance estimate for future phases is inferred from one body.

Render resolution follows the actual viewport aspect and tier cap. The footer shows most recent renderer duration and dimensions. Quality settings alter mesh/display resolution only; the 60 Hz dt and 32-component physical sea remain unchanged. Caches rebuild after geometry/light/quality changes; first-frame cost is higher than this steady baseline.

Numerical regression checks include dispersion residual <1e−12, exact discrete spectral energy, finite-difference wave derivatives, seeded repeatability, finite-record variance, terrain bounds/interpolation, regular-wave height/period, affine space/time-current reconstruction, read-only rendering, legacy schema migration, exact saved-camera precision, embedded terrain round trips and edit recovery. Tests establish these properties, not field accuracy or final visual acceptance.

Additional packaged-build checks: all three bundled v3 projects load and render their first 512×320 frame through `--check-project` (coast 45.91 ms, underwater 26.92 ms, canyon 54.85 ms). This excludes OS startup, window painting and the deliberate intro. A minimized, input-idle app recorded 0 ms process CPU during a two-second sample after startup, with 12.7 MB working set. An active foreground sample included mouse events and is not an idle result. Native close exits the process cleanly; a reentrant destruction/quit bug found during this check was fixed.

Final verification: 25 automated tests pass, formatting passes, and strict workspace/all-target Clippy passes. The root `Pelagic.exe` hash matches the release binary. The three bundled reference projects are independently reopened by the packaged executable.

Pending release measurements: sustained full-app latency/idle CPU across broader scenarios, peak memory across realistic project histories, cold start on minimum hardware, long-run numerical stability under the Phase 3 body/contact model, and multi-platform/compiler reproducibility. No power, operational or scientific certification claim is made.
