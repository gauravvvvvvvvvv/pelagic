# Seabed survey planning and coverage

Status: 24 November 2026.

Pelagic plans survey lines for a multibeam echosounder or a side-scan sonar
over the scene's seabed, measures the coverage, gaps and depth uncertainty
they achieve against IHO S-44, and can run the lines with a vehicle's own
physics in the scene's sea and current to show what a real platform would
deliver.

## Standard

IHO S-44 *Standards for Hydrographic Surveys*, Edition 6.2.0 (October 2024),
Table 1:

| Order | THU (95 %) | TVU a, b (95 %) | Feature detection | Bathymetric coverage |
|---|---|---|---|---|
| Order 2 | 20 m + 10 % of depth | 1.0 m, 0.023 | not specified | 5 % |
| Order 1b | 5 m + 5 % | 0.5 m, 0.013 | not specified | 5 % |
| Order 1a | 5 m + 5 % | 0.5 m, 0.013 | cubes > 2 m to 40 m, 10 % of depth beyond | 100 % |
| Special Order | 2 m | 0.25 m, 0.0075 | cubes > 1 m | 100 % |
| Exclusive Order | 1 m | 0.15 m, 0.0075 | cubes > 0.5 m | 200 % |

Maximum TVU at depth d is √(a² + (b·d)²). Pelagic checks TVU and coverage;
it does not simulate positioning (THU).

## How it works

**Sounding uncertainty.** For a beam at angle θ from vertical, height h
above the seabed: σ = √(σ₀² + (k·h)² + (h·tanθ·σθ)²), the fixed (heave,
draft, tide), range (sound speed) and pointing (attitude, refraction)
parts; TVU = 1.96·σ. Defaults: σ₀ 0.05 m, k 0.15 %, σθ 0.1° (editable).
Outer beams lose accuracy fastest, so the usable swath for an order is the
smallest of the swath angle, the angle the maximum range reaches, and the
angle where TVU meets the order's limit.

**Side-scan.** Covers from a blind zone under the vehicle (default one
altitude wide on each side) out to its range on each side; it measures no
depth, so its coverage counts as feature search, not bathymetry.

**Lines.** Parallel lines on the area's axis. Each spacing adapts to the
narrowest swath along the neighbouring lines (shallow water narrows a
multibeam swath), with the chosen overlap; for 200 % (Exclusive Order) the
spacing halves and the outer lines sit on the boundary. Lines are cut where
the water is too shallow for the platform (land, shoals), so a line can have
several segments. **Lines along the contours** turns the lines across the
mean slope, which keeps the swath width steady.

**Coverage.** Edge beams are ray-cast onto the seabed from the transducer's
actual pose, the swath between successive samples is rasterised on a grid
aligned with the area, and every cell keeps its number of passes and its
best TVU. Gaps ("holidays") are connected groups of cells short of the
required passes. Land and water too shallow for the platform are left out
of the percentages and reported separately. For the 100 % and 200 % orders
Pelagic reports "meets the order" from 99.5 % of the workable area with the
required passes (S-44 notes that full coverage does not mean continuous
soundings) and always lists the remaining gaps and the largest one, so you
can plan infill lines.

**Physics run.** With a vehicle selected, Pelagic builds the scene's world
(seabed, sea, current) with only that vehicle and runs the lines: a speed
controller holds the survey speed through the water (at most 80 % of the
forward thrust, so depth keeping and steering keep some authority; 75 % of
survey speed in turns), integral line-of-sight steering (look-ahead 3 hull
lengths, at least 15 m) feeds a heading loop sized from what the vehicle
can deliver (full rudder and thruster yaw moment at 20° heading error,
damping from the rigid plus added yaw inertia), underwater vehicles follow
the seabed at the set altitude, and turns are racetracks with straight
run-ins. The swath is cast from the vehicle's real position and attitude
every 0.1 s, so roll, pitch, crab angle and cross-track error show up as
narrower, skewed or missing coverage. A vehicle that cannot reach the
survey speed is reported.

Found while testing: a heading autopilot tuned from rigid-body inertia
alone (as the trials use) asked the example AUV for about 16 % fin, so it
turned wide and came onto each line late (27 m RMS cross-track, 80 %
coverage); it was also too slow for the legacy demo boat, which is
directionally unstable (strong Munk moment, no skeg or rudder) and weaved
±8 m (74 % coverage). Sizing the loop from the available yaw authority
gives 0.05–0.2 m RMS for the AUV, the workboat and the demo boat on the
same demo plan.

## In the editor (Survey tab)

1. The area starts fitted to the scene's seabed. Adjust its centre, size
   and line heading, or use **Fit to seabed** and **Lines along the
   contours**.
2. Choose the sonar (multibeam: swath angle, range, beams, ping rate;
   side-scan: range per side, blind zone), the platform (surface vessel
   with transducer depth, or underwater at an altitude) and the IHO order.
   The tab shows the order's limits at the area's depth.
3. **▶ Plan survey** gives the lines and their planned coverage.
   **▶ Run with vehicle physics** (select a vehicle first) also runs them.
4. The map shows coverage (blue meets the order, green extra passes, orange
   gaps, brown data outside the TVU limit, grey-brown land or too shallow),
   depth or uncertainty, with the planned lines and the vehicle's track.
   Hover a cell for its depth, passes and TVU. The chart shows the
   cross-track error on the lines.

**Export…** writes `summary.txt`, `survey.json`, `lines.csv`, `coverage.asc`,
`tvu.asc` and `depth.asc` (ESRI ASCII grids in scene metres, north-up) and,
for scenes with a geographic origin, `lines.gpx` and `survey.waypoints`
(QGC WPL 110 mission).

## Command line

```
Pelagic.exe --survey <project.pelagic | demo> <out-folder>
  [--sensor=mbes|sss] [--swath=DEG] [--range=M] [--beams=N] [--ping=HZ]
  [--underwater] [--altitude=M] [--draft=M]
  [--order=2|1b|1a|special|exclusive] [--overlap=PCT] [--speed=KN] [--run-in=M]
  [--area=CX,CY,LENGTH,WIDTH] [--heading=DEG|contours]
  [--vehicle=NAME | --vehicle-file=PATH]
```

Exit code 0 when the (simulated, else planned) coverage meets the order,
2 when it falls short, 1 on error.

## Limits

- Seabed from the scene's grid (bilinear); beams are rays (no footprint
  size, no multipath or absorption); side-scan range is geometric.
- No positioning (THU) or refraction simulation; the uncertainty model is a
  standard three-term estimate, not a full TPU computation.
- Feature detection is judged from sounding spacing (at least about three
  soundings across the feature), not from simulated returns.
- Transits between line segments are straight and do not avoid land;
  plan physics runs over navigable water.

## References

- International Hydrographic Organization (2024). *IHO Standards for
  Hydrographic Surveys*, S-44 Edition 6.2.0, Table 1.
- Fossen, T. I. (2011). *Handbook of Marine Craft Hydrodynamics and Motion
  Control* (line-of-sight guidance).
