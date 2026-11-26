# Build 0.4 measurements

15 November 2026. Intel Core i5-11320H (4 cores / 8 threads), 16 GB RAM, Windows, GNU Rust release build. CPU renderer; no GPU requirement. These are development measurements from this machine, not hardware qualification or field validation.

The release engine fingerprint used for delivery is `f3ccc7d53200b5a8`. It includes core source, Cargo files, compiler version, target and profile. Scenes/runs record their actual engine fingerprint; the portable package also has per-file SHA-256 hashes. The fingerprint identifies a build and is not a cryptographic attestation.

## Reference workloads

`cargo +stable-x86_64-pc-windows-gnu run --release --example phase34` measures 600 fixed steps with sensor scheduling and 30 warm render frames at Low / 512×320. Frame p95 is sorted rank 29; step p95 is rank 570. The renderer consumes snapshots; core stepping includes current/wave queries, contacts/controllers, life/tracers and the configured sensor schedule.

Both cases use synthetic 512×512 m terrain, a 129² source grid, Low terrain/water meshes, 32 spectral components and 60 Hz world ticks. Contact-enabled bodies substep at up to 120 Hz. The robotics scene has three bodies, 24 fish, 64 tracers and a 48-beam 10 Hz suite. The schooling scene has 96 fish and 64 tracers. Neither contains a large imported mesh.

| Workload | Cold render ms | Warm mean ms | Render p95 ms | Step mean µs | Step p95 µs |
|---|---:|---:|---:|---:|---:|
| Marine robotics lab | 20.69 | 18.18 | 20.19 | 903.79 | 1,016.60 |
| Pelagic schooling lab | 22.97 | 19.99 | 20.59 | 1,257.21 | 1,303.60 |

The 33.3 ms **core render** budget and 5 ms **core step** budget pass for these cases. Full-window painting, input handling, file export and arbitrary imported assets are excluded. A fixed 60 Hz step does not guarantee real-time throughput under every load. Balanced/High remain slower inspection tiers. Phase 2's earlier reports remain historical measurements, not the current workload definition.

The project check separately reopened all three shipped schema-v4 scenes and rendered a first Low frame: robotics 24.69 ms, ecology 26.43 ms, Goa 46.76 ms. This excludes OS process startup, native window painting and the deliberately four-second intro. It confirms the bounded load path, not a complete startup latency distribution.

## Acceptance evidence

- 42 automated tests pass in the release profile; formatting and strict workspace/all-target Clippy pass.
- An independent host loads the actual DLL, resolves ABI v1, advances 60 ticks, checks t=1 s, releases the world and verifies that the released handle is rejected.
- The separate-process private UDP test exchanges eight valid telemetry frames and eight accepted MAVLink controls over 90 ticks.
- A five-second reference experiment produces 51 samples; every truth sample matches re-simulation. A camera export produces 51 calibrated RGB images and complete JSON/CSV metadata without an incomplete marker.
- Tests include a mid-run checkpoint plus changing commands, saved-run round trips, field/parser rejection, real-data origin/unit checks, partial buoyancy/settling, allocator saturation, controller convergence, contact momentum/separation, seeded schooling, sensor latency/specific force, camera roll/FOV cache invalidation and layout label/control overlap checks.
- Generated editor, life, sonar, results and atlas previews are inspected from the application's own rendering output. User-desktop inspection is not part of this verification.

Read the model limits in PHASES-3-4.md. Open Phase 5 measurements include sustained full-app latency and memory across realistic histories, minimum-hardware qualification, long runs and extreme states, broader dataset imports, accessibility and independent model/data comparisons. No scientific or operational certification is inferred from these tests.
