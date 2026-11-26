# Test campaigns, recordings and reproducibility

Status: 23 November 2026. Available in the GPU editor (Experiments tab) and
headless from the command line.

## What a campaign is

A campaign runs the current scene many times as headless batches and judges
every run against acceptance criteria. Run `i` uses seed `first seed + i`,
which changes wave phases, marine-life behaviour and sensor noise. Optionally
each run also scales the significant wave height and the current speed by a
uniform random factor within ± the given fraction.

Every run is deterministic. Any run, including a failing one, can be
reproduced exactly from its index: **Reproduce** in the results table re-runs
it as a recording that can be replayed, scrubbed and exported.

## Metrics (tracked vehicle)

The tracked vehicle is the sensor rig's vehicle (Sensors > entity), otherwise
the first vehicle.

| Metric | Definition |
|---|---|
| Nav error max / RMS | Distance between the noisy position fix and the true position at the fix's sample time (fixes are delivered after the sensor latency, so each is compared with truth at its own timestamp). |
| Depth excursion | Largest change in depth from the first sample. |
| Seabed clearance | Smallest gap between the vehicle's bounding sphere and the seabed directly below it. |
| Vehicle separation | Smallest gap between the bounding spheres of any two vehicles (infinite with one vehicle). |
| Tilt | Largest angle between the body's up axis and vertical (roll and pitch combined). |
| Speed, distance | Largest speed and path length. |
| Sonar coverage | Mean fraction of sonar beams with a return. |

A metric with no data (for example no position fixes) is reported as NaN, and
an enabled check on it fails rather than silently passing.

## Command line (CI)

```
Pelagic.exe --campaign <project.pelagic> <report-folder> [options]
  --runs=N  --seconds=S  --seed=B  --wave-jitter=F  --current-jitter=F  --threads=N
  --max-nav-error=M  --max-depth-excursion=M  --min-clearance=M
  --min-separation=M  --max-tilt=DEG  --min-sonar-coverage=F
```

Exit code 0 means every run passed, 2 means some runs failed their criteria,
1 means invalid input or an error. The report folder receives
`campaign.json` (full report), `campaign.csv` (one row per run, stable
columns) and `summary.txt`. Runs execute on all cores but one (at most 8).
Measured on the reference laptop (Iris Xe laptop CPU): 12 runs × 20 s of
the robotics demo (240 simulated seconds) complete in about 3.2 s.

`Pelagic.exe --rerecord <in.pelagic-run> <out.pelagic-run>` re-simulates a
recording made by an older build (same scene, checkpoint and commands) and
verifies the result, so **Verify re-simulation** passes again on this build.

## Reproducibility guarantee

Simulation maths (sin, cos, exp, pow, atan2, …) goes through portable musl
implementations (`libm`, see `pelagic::math::Det`) instead of the platform
library. Basic arithmetic and square roots are exactly specified by IEEE 754
and Rust does not fuse or reorder floating-point operations, so the same
scene, seed and commands produce bit-identical results in the editor, the
command line and the SDK on any x86-64 machine. The test
`shipped_reference_recording_verifies_in_another_executable` checks this:
the reference recording is made by the editor executable and re-simulated
exactly by a different test executable. (Before this change the two
executables differed in the last bits after 6 ticks.)

Rendering is not part of this guarantee and keeps the faster platform maths.

## Editor

- **Experiments** tab, left column: live recording, headless batch, loaded
  recording (Replay, scrub, Save, Verify, dataset and camera exports), Open
  recording, Export mounted camera, Retry backups.
- **Experiments** tab, right column: campaign setup and criteria, Run
  campaign, results table (failed-only filter, hover a result for the
  reasons), Reproduce per run, Export report.
- **Sensor Monitor** tab: latest live or recorded observation, navigation
  readouts (depth, position, DVL velocity, gyro, accelerometer) and the
  sonar fan, which zooms to the returns. "View from mounted camera" shows
  the 3D viewport from the sensor camera on its vehicle.

## Limits

- Metrics cover one tracked vehicle plus pairwise separation; there are no
  mission/waypoint scripts or controller-in-the-loop interfaces in the
  campaign yet (the MAVLink bridge exists separately in the SDK).
- Jitter varies sea state and current only; mass, thruster and sensor
  parameters are not yet dispersed.
- Recordings hold at most 1200 samples, which bounds run length times sample
  rate.
