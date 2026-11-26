# Search and rescue planning

Status: 24 November 2026.

Pelagic predicts where a person, life raft or boat drifts from its last
known position (LKP) under the live wind and current forecast, lays out the
standard search patterns, and scores each plan by its probability of success
(POS): the chance the search finds the object. It is a planning and training
aid for search-and-rescue work and for testing uncrewed search vehicles. It
does not replace a rescue coordination centre's own tools and procedures.

## How it works

**Drift (Monte Carlo leeway).** Each of several thousand particles is one
hypothesis of the object. It enters the water inside the LKP uncertainty
circle (radius holding 90 % of start positions) at a random time in the
incident window, then moves with the surface current plus its **leeway**:

- downwind leeway = (slope + ε/20) · U10 + offset + ε/2
- crosswind leeway, to the right or left of downwind, the same form

with U10 the 10 m wind. Slopes, offsets and their standard deviations come
from the US Coast Guard leeway taxonomy (Allen & Plourde 1999, report
CG-D-08-99; Allen 2005), as tabulated for SAROPS and OpenDrift. Each particle
keeps its own leeway error ε, its own wind error and current error (default
1 m/s and 0.1 m/s per component), and its crosswind side, which it changes
("jibes") with a 4 % chance per hour. Integration is midpoint (RK2) in
10-minute steps; downwind errors that would make the slope negative are
redrawn, as in the published model.

21 object classes are included: persons in the water in five states, life
rafts with and without ballast or drogue, sea kayak, surfboard, windsurfer,
skiff, sport boat, fishing vessel, sailboat, 20 ft container, bait box and
fishing-vessel debris. Use the general (mean) class unless more is known.

**Wind and current.** With *Live forecast*, Pelagic downloads hourly 10 m
wind and surface current from Open-Meteo on a 5 × 5 grid about 22 km apart
around the LKP, from two days ago to three days ahead, and blends the
locations by inverse distance. Land or ice cells are left out. With
*Steady* you give one wind and current instead (offline, what-if studies).
Open-Meteo's surface current includes tides and wave (Stokes) drift; the
leeway coefficients were measured against the near-surface current, so
some wave drift is counted twice in rough seas.

**Search plans.** For the search unit's speed, sweep width W and time on
scene, Pelagic lays out:

| Pattern | When it fits | Variants tried |
|---|---|---|
| Expanding square | small area around a precise datum | spacing W/C for C = 0.5–2 |
| Sector | very precise datum, small area | 1–4 passes (legs R, 120° turns, then rotated 30°, 15°, 45°) |
| Parallel track | large area, uncertain position | C = 0.5–2, area sized to the effort and the drift's shape |
| Creeping line | long, narrow area | as parallel track, legs across the axis, progressing up-drift |

Areas are centred on the datum and laid on the drift's principal axis
(long side down-drift). By default the whole pattern **moves with the
drifting datum**, as when a unit navigates relative to the water; turn this
off to fix it to the ground.

**Probability of success.** The search unit flies its track through the
drifting particles. Each particle's detection probability grows with a
lateral-range model of sweep width W:

- *Inverse cube law* (Koopman 1946): detection rate W²V / (8πr³) at range r
  for search speed V. For an infinite parallel sweep this gives
  POD = erf(√π · C / 2) with C = W / S (79 % at C = 1, 99 % at C = 2);
  Pelagic's tests reproduce both within 1 %.
- *Definite range*: certain inside W/2, nothing beyond (a hard swath).

POS is the mean detection probability over all particles, so it already
counts the chance that the object is outside the planned area. The table
also gives POC (share of particles inside the area at mid-search), POD
inside the area and coverage C = W · track length / area.

**Sweep width.** Take W from your SAR manual's tables for the object,
search unit, visibility and sea state. The camera helper gives a geometric
upper bound for an uncrewed vehicle's camera (pixels on target and the
horizon); whitecaps, glare, haze and waves hiding small objects reduce it.

## In the editor (Search & Rescue tab)

1. Set the last known position: type it, use **Earth view centre** or
   **Scene location**, or right-click the sea on the Earth tab →
   **Search and rescue here**.
2. Choose the object, the position uncertainty, how long ago it entered
   the water and the time window, and *Live forecast* or *Steady*.
3. Set the search unit: arrival on scene, time available, speed, sweep
   width, detection law and the patterns to try. The selected vehicle's
   name is used for the unit.
4. **Plan search.** The plans appear best first; click one to show it.

The map shows each hypothesis as a dot that fades as the search clears it,
the hourly datum track from the LKP, the search area (dashed) and the track
(solid where flown). Drag to pan, scroll to zoom, double-click to fit. The
slider and ▶ play the drift and the search; the chart shows POS building up
over the time on scene. The drift, track and area are also drawn on the
Earth globe.

**Export plan…** writes `summary.txt`, `drift.csv` (hourly datum, spread,
axis), `particles.csv` (positions at the end of the search with detection
probability), `plans.json`, `plan.gpx` (route with planned times, LKP and
datum) and `plan.waypoints` (QGC WPL 110 mission: home, search speed,
waypoints) for an uncrewed surface vehicle in QGroundControl or Mission
Planner. Waypoints are ground positions at their planned times, so a
drifting pattern is already compensated.

## Command line

```
Pelagic.exe --sar <latitude> <longitude> <out-folder> [options]
Pelagic.exe --sar --objects
  --object=ID --ago=H --window=H --radius=M --arrive=H --endurance=H
  --speed=KN --sweep=M --definite --fixed --pattern=SS|VS|PS|CS
  --particles=N --wind=SPEED,FROM --current=SPEED,TOWARDS --seed=N --threads=N
```

It writes the same files as **Export plan…** for the best plan. A live run
(download plus 19 plans) takes about 2 s.

## Limits

- Wind and current are a forecast (about 8–25 km cells), blended between 25
  locations; no coastline: particles can drift onto land, and nothing
  strands.
- Leeway coefficients are class means from field experiments; individual
  objects differ. Capsizing and swamping are not modelled.
- One search unit at a time; no fatigue, visibility or weather correction is
  applied to W automatically; search units move kinematically (constant
  speed along the track), not with their vessel physics.
- POS assumes the object is in the water for the whole search and that a
  detection is recognised; no false alarms.

## References

- Allen, A. A., & Plourde, J. V. (1999). *Review of leeway: field
  experiments and implementation.* US Coast Guard R&D Center, CG-D-08-99.
- Allen, A. A. (2005). *Leeway divergence.* US Coast Guard R&D Center,
  CG-D-05-05.
- Breivik, Ø., & Allen, A. A. (2008). An operational search and rescue model
  for the Norwegian Sea and the North Sea. *Journal of Marine Systems* 69,
  99–113.
- Koopman, B. O. (1946). *Search and screening.* OEG Report 56, US Navy.
