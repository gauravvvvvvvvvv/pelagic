# Standard trials

Status: 23 November 2026.

Standard trials measure how a vehicle behaves, using the same procedures as
tank tests and sea trials. Run them on any vehicle in the editor (the
**Trials** tab next to Viewport and Earth) or from the command line on a
vehicle definition. Every trial is a headless, deterministic simulation, so
the same vehicle and settings always give the same numbers on any machine.

Scope: platforms (surface vessels, submersibles, AUVs, ROVs, USVs, drifters)
and their navigation and survey equipment. Weapon payloads and targeting are
outside the product's scope.

## The trials

| Trial | What it does | What it reports |
|---|---|---|
| Free decay | Displaces the hull in heave (surface vessels), roll and pitch, releases it in still water | damped period, damping ratio (log decrement), closed-form linear estimate, metacentric height implied by the period |
| Speed | Holds a straight course at each thrust level until the speed settles (≤ 300 s) | steady speed, time to 90 %, effective resistance coefficient thrust/speed², largest tilt |
| Turning circle | From the steady approach, full rudder (or the thruster yaw command) to port and to starboard, 540° of turn | advance, transfer, tactical diameter, steady diameter, times to 90°/180°, steady speed and yaw rate, drift angle, heel, track |
| Zig-zag | 10°/10° and 20°/20° (rudder angle = switching angle), starboard first | initial turning time and distance, first and second overshoots, period, heading record |
| Stopping | Full astern from the approach (coasting if the vehicle has no astern thrust) | time to stop, track reach, head reach, lateral deviation, track |
| Wave response | Regular waves at each period and heading (head, beam), zero speed, heading held | heave, roll and pitch amplitudes and response ratios (heave/wave amplitude, angle/wave slope) |

Conventions: world Z up; the vehicle starts heading along +X. Heading grows to
port (anticlockwise seen from above). Roll is positive starboard side down,
pitch positive bow down. A vehicle is tested **at the surface** if it is not
an underwater vehicle and floats; otherwise **submerged** at the operating
depth (default 20 m), with depth held by its own depth controller during
manoeuvres and wave tests. Passive submerged bodies and all submerged free
decay tests are ballasted to neutral buoyancy, as in a tank.

Steering: a vehicle with rudders (yaw-channel fins) is steered by rudder
angle, exactly like ship trials. Otherwise the yaw command is a yaw moment
through the thrusters, as a fraction of the vehicle's yaw authority. During
approaches the heading is held by a trial autopilot (natural frequency
0.4 rad/s, damping 0.9), not by the vehicle's own heading controller.

Environment: still, deep water by default (200 m, the scene's density).
**Scene's current, sea and depth** runs the manoeuvres in the scene's current
and irregular sea instead; free decay and wave response always use still
water so their results stay comparable.

## Acceptance limits

Optional limits give each run a pass or fail and set the command-line exit
code. **IMO reference (MSC.137(76))** applies the IMO Standards for Ship
Manoeuvrability at the trial speed: advance ≤ 4.5 L, tactical diameter ≤ 5 L,
initial turning ≤ 2.5 L, 10°/10° overshoots by L/V (first 10°, 5 + 0.5·L/V
or 20°; second 25°, 17.5 + 0.75·L/V or 40°), 20°/20° first overshoot ≤ 25°,
stopping track reach ≤ 15 L. These are defined for ships with rudders at full
scale; for other craft they are a guide. Other limits: top speed, roll damping
ratio, roll response in waves. A limit applies only when its trial ran; a
trial that ran but gave no value (for example a turn that never reached 90°)
fails its limit.

Warnings flag results that need attention: legacy (untuned) coefficients,
tilts beyond 30° during a manoeuvre, a hull that settles heeled.

## Command line

```
Pelagic.exe --trials <vehicle.pelagic-vehicle.json> <report-folder> [options]
  --only=decay,speed,turning,zigzag,stopping,waves   --quick
  --scene=<project.pelagic>   --water-depth=M   --depth=M
  --approach=F   --rudder=DEG   --yaw=F   --wave-height=M   --threads=N
  --imo   --min-speed=V   --max-advance=L   --max-tactical=L
  --max-overshoot=DEG   --max-stopping=L   --min-roll-damping=Z   --max-roll-rao=R
```

Exit code 0 when every check passes, 2 when a check fails, 1 for invalid
input or errors. The folder receives `trials.json` (everything, including the
exact vehicle tested), `summary.txt`, `speed.csv`, `decay.csv`, `tracks.csv`,
`zigzag.csv` and `waves.csv`.

Example (the shipped workboat; the full suite takes about 0.3 s):

```
Pelagic.exe --trials examples/vehicles/workboat-usv.pelagic-vehicle.json out --imo
```

## Physics behind the numbers (and its limits)

- Hydrostatics: the hull is a box; surface-piercing bodies integrate buoyancy
  over 4×3 columns against the wave surface. The effective waterplane is
  V / hull depth, which sets the heave stiffness, the linear estimate and the
  waterline damping (ζ = 0.12 in heave, roll and pitch).
- Added mass and added inertia are diagonal. The added-mass Coriolis force and
  Munk moment are both applied, so a hull exchanges energy between sliding
  and turning but never creates it. Surface-piercing hulls keep only their
  yaw (manoeuvring) part.
- Fins, keels and rudders are flat plates with a lift-curve slope, stall and
  a flat-plate coefficient beyond stall (see `docs/VEHICLE-DEFINITIONS.md`).
  There is no interaction between surfaces, hull or propeller wash.
- Drag is quadratic per body axis. There is no wave-making resistance,
  squat or shallow-water effect, and no diffraction or radiation forces in
  waves beyond the waterline damping.
- Results are as good as the coefficients. Use measured, towing-tank or CFD
  values when available; the Estimate buttons give first-order values.
