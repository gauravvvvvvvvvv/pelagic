# Vehicle definitions: bring your own prototype

Status: 23 November 2026.

A vehicle definition (`*.pelagic-vehicle.json`) describes a prototype's
physics in one shareable, version-controllable file: identity, mass
properties, hull size, hydrodynamic coefficients, thrusters, actuator limits,
controllers and an optional hull mesh. Import it into any scene, edit it in
Details, test it (batches, campaigns, sensors, real-world locations) and
export it again.

Scope: platforms (surface vessels, submersibles, AUVs, ROVs, USVs, drifters)
and their navigation and survey equipment. Weapon payloads and targeting are
outside the product's scope.

## In the editor

- **File > Import vehicle definition…** (or *Import vehicle definition…* in
  the scene overview) places the vehicle near the camera focus. Surface
  vessels start at the surface. A hull OBJ named by `mesh_obj` (relative to
  the definition file) is added to the project library.
- Select the vehicle, then **Export vehicle definition…** in Details writes
  the definition, plus `<name>.obj` (metres, body frame) if the body uses a
  library mesh. Exported files list inertia and drag explicitly: the values
  the simulation actually uses.
- Details sections: Transform, Physics (mass, displaced volume, thrust
  limit, net buoyancy), **Hydrodynamics** (inertia, drag, rotational drag,
  added mass, added inertia, centre of buoyancy, surface crossing,
  collisions), **Thrusters** (position, direction, max thrust; add/remove, up
  to 32), **Fins and rudders** (role, position, orientation, area, lift slope,
  stall, deflection; add rudder, hydroplanes or a fixed fin; up to 16),
  **Controllers** (actuator lag, depth hold, heading hold), Appearance.
- Test the vehicle with the standard trials (Trials tab, `docs/TRIALS.md`).
- **Estimate (blunt)** / **Estimate (streamlined)** fill inertia, drag,
  rotational drag and added mass from the hull size (formulas below).
  **Legacy defaults** returns to the mass-scaled values of earlier versions.

## Format (version 1)

Units are SI. Body frame: X forward, Y left, Z up, origin at the centre of
mass.

| Field | Meaning |
|---|---|
| `format`, `version` | `"pelagic-vehicle"`, `1` |
| `name` | 1–80 characters |
| `kind` | 0 underwater vehicle, 1 surface vessel, 2 drifter / passive body |
| `mass` | kg (0.01–100 000) |
| `volume` | displaced volume when fully submerged, m³ |
| `model.half` | hull half-extents, m (0.02–20 each) |
| `model.inertia` | principal moments of inertia, kg·m²; `null` = solid box |
| `model.drag` | quadratic drag per axis, N/(m/s)²; `null` = legacy mass-scaled |
| `model.angular_drag` | quadratic rotational drag per axis, N·m/(rad/s)²; `null` = legacy |
| `model.added_mass` | added mass per axis, kg |
| `model.added_inertia` | added (fluid) moments of inertia about X/Y/Z, kg·m²; default 0 |
| `model.center_buoyancy` | centre of buoyancy relative to the centre of mass, m |
| `model.partial_buoyancy` | integrate buoyancy against the wave surface when crossing it |
| `model.profile_density` | use the local water-column density instead of the scene's |
| `model.contacts` | collide with the seabed, structures and other bodies |
| `model.thrust_limit` | total commanded force limit, N |
| `model.response_seconds` | first-order actuator lag, s |
| `model.hold_depth` / `hold_heading` | controller targets (m below surface / radians), or `null` |
| `model.thrusters[]` | `position` (m), unit `direction`, `limit` (N); allocated by least squares |
| `model.fins[]` | lifting surfaces, see below; default none |
| `mesh_obj` | hull OBJ path relative to the definition, optional |
| `notes` | free text: origin, revision, data sources, assumptions |

Examples: `examples/vehicles/survey-auv.pelagic-vehicle.json` (2.2 m survey
AUV, five thrusters, cruciform tail with rudders and hydroplanes, depth and
heading hold) and `examples/vehicles/workboat-usv.pelagic-vehicle.json` (6 m
uncrewed workboat, twin stern thrusters, twin rudders and a lateral-plane
fin). Their notes record how each coefficient was
derived; tests check that both load and behave as designed (the AUV holds
15 m, the workboat floats at its design draft).

## Fins, keels, rudders and hydroplanes

Each entry of `model.fins` is a flat plate whose chord lies along body X:

| Field | Meaning |
|---|---|
| `position` | centre of pressure, body frame, m |
| `normal` | unit lift direction at positive angle of attack, perpendicular to X: (0, 1, 0) vertical fin, (0, 0, 1) horizontal fin |
| `area` | planform area, m² (0.0001–100) |
| `lift_slope` | dC_L/dα per radian; Helmbold for aspect ratio A: 2π·A/(2 + √(A² + 4)) |
| `stall_deg` | stall angle of attack |
| `channel` | 0 fixed fin/keel, 1 rudder (yaw), 2 hydroplane (pitch), 3 roll fin |
| `max_deflection_deg` | largest deflection of a control surface |

Normal force N = ½·ρ·S·U²·C_N(α), with C_N = a·sin α up to stall and a
flat-plate value beyond (max(0.6·a·sin α_stall, 1.2·|sin α|)); U is the flow
speed in the plate's chord–normal plane, including the body's rotation.
Control surfaces take a command in −1…1 per channel (the fraction of their
largest deflection); the deflection sign is chosen from the surface's
position so a positive command always turns the vehicle positively about
that axis. Fins aft of the centre of mass make a hull directionally stable
against its Munk moment; without them a streamlined hull turns broadside at
speed, as a real finless body would.

## Hull estimates

`pelagic::definition::estimate_hydrodynamics` (box hull, water density ρ,
half-extents a, b, c):

- inertia: solid box, I_x = m(b² + c²)/3 and cyclic;
- drag: 0.5·ρ·C_d·A per axis over the projected area, C_d = 1.0, or 0.3
  along X when streamlined;
- rotational drag: strip integration of cross-flow drag,
  K_z = 0.5·ρ·C_d·c·(a⁴ + b⁴) and cyclic;
- added mass: 2-D strip theory, A_y = ρ·π·c²·2a, A_z = ρ·π·b²·2a; 10 %
  (streamlined) or 20 % (blunt) of the displaced mass along X;
- added inertia: the same strips about the centre, N_ṙ = A_y·a²/3,
  M_q̇ = A_z·a²/3, K_ṗ = 2a·ρ·π/8·(b² − c²)².

These are first-order engineering estimates. Replace them with measured,
towing-tank or CFD coefficients when available.

Why it matters: the legacy defaults give very little rotational damping (for
the 1.3 m reference ROV about 1/100 of the strip-theory value). With them, an
ROV driven at 1.5 m/s pitches up and tumbles under the Munk moment, which is
a real destabilising effect of added mass. With estimated damping it holds
attitude. Earlier scenes keep the legacy values unless you press an
Estimate button, so their behaviour and recordings do not change.

## Limits

- The hull is a box for hydrostatics, drag and contacts; the mesh is visual.
- Coefficients are diagonal (no full 6×6 added-mass or damping matrix). The
  hull itself makes no lift; represent keels, skegs and the lateral plane
  with fixed fins. Fins do not interact with each other, the hull or
  propeller wash.
- One sensor rig per scene (Sensors in the Outliner), not per definition.

## Physics changes in this version

These change results for existing scenes (recordings made with earlier
builds no longer verify; re-record them with `Pelagic.exe --rerecord`):

- Thruster allocation controls roll and pitch only when a roll or pitch
  torque is requested. Propellers below the centre of mass are no longer
  throttled back to cancel their own pitch moment (the example workboat
  gets its full 4 kN instead of 2.2 kN).
- Waterline damping uses the effective waterplane V/(2·half.z), the same
  one the buoyancy uses, instead of the box footprint.
- The added-mass Coriolis force is applied with the Munk moment, so the
  pair conserves energy; surface-piercing hulls keep only the yaw parts.
- Added inertia (rotational added mass) and fins are new; both default to
  none, so bodies without them only see the three changes above.
