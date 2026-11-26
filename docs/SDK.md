# Rust, C ABI and robotics adapters

The core is Rust. The desktop is a native Windows host; no browser, WebView, Python or JavaScript runtime is required. Serde handles versioned data. All coordinates use metres, seconds and a right-handed Z-up world: X east, Y north. Body axes are forward/left/up; quaternions map body into world.

## Shared Rust scene

`pelagic::scene::Scene` is the studio's typed, versioned scene. `Scene::read` loads a validated project; `pelagic::scene::world(&scene)` creates its world. `Scene::robotics_demo()` and `Scene::ecology_demo()` provide reference configurations. A `World` owns time and dynamic state. `checkpoint()`/`restore()` include velocities, orientation, actuator state, fish and tracers; environment/asset configuration comes from the matching scene.

```rust
let scene = pelagic::scene::Scene::robotics_demo();
let mut world = pelagic::scene::world(&scene)?;
for _ in 0..600 { world.step(); }
let truth = world.snapshot();
let water = world.ocean.water(pelagic::math::Vec3::new(0.0, 0.0, -20.0));
```

`Ocean` samples bed, waves, total flow and water properties. `BodyModel` configures geometry/proxies, added mass, buoyancy, contacts, thrusters and controllers. `Entity` factories create the ROV, surface vessel and drifter. `SchoolConfig` controls seeded behaviour. `CurrentField`, `Bathymetry` and `Renderer` remain replaceable Rust boundaries.

The C ABI is the external binary boundary. In-process Rust extensions compile against the same crate/API version; a stable Rust dynamic ABI is not assumed. Raw public numerical values must preserve constructor/validation invariants. The desktop's declared envelope is 16 bodies, 256 fish and 512 tracers. Low-quality measurements apply to the reference workloads, not arbitrary imported geometry or fleets.

## Headless experiments

From the source root on this workstation:

```powershell
cargo +stable-x86_64-pc-windows-gnu run --release --example experiment -- examples/scenes/v5-robotics.pelagic output/my-run 5
cargo +stable-x86_64-pc-windows-gnu run --release --example experiment -- --verify output/my-run/experiment.pelagic-run
cargo +stable-x86_64-pc-windows-gnu run --release --example experiment -- examples/scenes/v5-robotics.pelagic output/my-camera-run 5 --camera
```

Choose a new output directory. Bundles contain `scene.pelagic`, `experiment.pelagic-run`, `manifest.json`, `truth.csv`, `sonar.csv`, `navigation.csv`, `fish.csv` and `commands.csv`. Camera exports add `camera/calibration.json` and RGB8 P6 PPM images, up to 120 selected truth samples. Source stride and image timestamps are explicit. Missing returns use empty CSV fields, not a fabricated maximum-range hit. Fish IDs are stable row indices within a run's reference school.

Truth linear velocities use world axes; angular rates, actuator wrenches and specific force use body axes. Observation time is measurement time, `available_time` is simulated delivery time. Sonar `truth_range` and acoustic travel time are model truth; the measured range includes configured noise. Navigation position/velocity/depth and IMU readings have separate stated noise. The quaternion is truth orientation, not a noisy attitude filter estimate. Camera coordinates start at the upper-left image corner with pixel centres at n+0.5; the mounted forward vector and body-up convention define the optical basis, including body roll.

Rates are fixed-tick quantised. Live recording and batch use the same suite. World and sensor checkpoints make mid-run starts reproducible, including pending latency and previous IMU velocity; command changes are stored at their ticks. Playback needs no re-simulation. The verifier compares physical truth, velocities and checkpointed observations and reports its first divergence. Legacy records without a sensor checkpoint retain truth-only verification. Keep scene, model/build metadata and assets together. Changing `World.ocean` outside the recorded scene contract requires updating that provenance yourself.

## C ABI version 1

Build with `cargo +stable-x86_64-pc-windows-gnu build --release`. The library emits `target/release/pelagic.dll`; `include/pelagic.h` describes the ABI. It uses integer world handles, `repr(C)` scalar result structs and explicit status values. No Rust collection/string ownership crosses the boundary.

`pelagic_create(seed)` creates the default scene. `pelagic_create_scene(bytes,length)` copies/validates UTF-8 project JSON, with a 16 MB limit; the supplied memory must be readable for the call. Destroy handles before unloading the module. Invalid handles return status 0. Up to 32 handle-owned worlds are supported. Calls are serialized by the registry; use independently owned Rust worlds for parallel execution.

`pelagic_step(handle,ticks)` advances at most 72,000 ticks per call. `pelagic_command` supplies body-frame force/torque by stable entity ID. `pelagic_state` returns time, position and world velocity; `pelagic_sample` returns environment properties. Check `pelagic_abi_version()` before binding an external application. This early ABI is versioned; later breaking changes require a new version rather than silently changing layouts.

```powershell
cargo +stable-x86_64-pc-windows-gnu run --release --example engine_host
```

The host loads the actual DLL through Windows `LoadLibraryW`, resolves symbols, advances 60 ticks, checks t=1 s, destroys the handle and checks rejection of that released handle. This proves the current Windows x64 DLL boundary; it does not establish every foreign-engine/platform combination. The portable package includes the DLL, header and host executable. Run `SDK\engine_host.exe SDK\pelagic.dll` from the package root.

## MAVLink 1 subset

```powershell
cargo +stable-x86_64-pc-windows-gnu build --release --examples
cargo +stable-x86_64-pc-windows-gnu run --release --example mavlink_loopback
cargo +stable-x86_64-pc-windows-gnu run --release --example mavlink -- 127.0.0.1:14551 127.0.0.1:14550 30
```

The loopback test uses private ephemeral ports and a separate adapter process. The adapter accepts one complete MAVLink 1 `MANUAL_CONTROL` frame per datagram, target system 1, with checked CRC and normalized axes. It publishes `LOCAL_POSITION_NED` at 10 Hz and heartbeat at 1 Hz. Body right/down commands are converted to Pelagic left/up; world east/north/up becomes NED north/east/down. Each accepted joystick axis maps to bounded force/torque commands; the model's allocator/lag still applies. Commands return to zero after one second without a valid packet.

Transport is deliberately restricted to specified loopback peers. This is not MAVLink 2, signed transport, the parameter/mission protocol, an autopilot estimator, or a hardware-in-the-loop qualification. A client must select the supported v1 messages and the stated joystick convention. ROS 2 integration remains an additional adapter, not a required runtime dependency.

Protocol sources: the [MAVLink message reference](https://mavlink.io/en/messages/common.html), [manual-control service](https://mavlink.io/en/services/manual_control.html), and official generated headers for [MANUAL_CONTROL](https://github.com/mavlink/c_library_v2/blob/master/common/mavlink_msg_manual_control.h) and [LOCAL_POSITION_NED](https://github.com/mavlink/c_library_v2/blob/master/common/mavlink_msg_local_position_ned.h). Pelagic's implementation and loopback checks cover the declared subset only.
