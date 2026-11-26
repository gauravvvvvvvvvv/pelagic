# Oil spill drift

Status: 24 November 2026.

Pelagic follows a surface oil spill from its release under the live wind
and current forecast (or steady values). It estimates how much evaporates,
and shows where and when the oil reaches the shore of a real-world scene. It
is a planning and exercise aid for response teams and for testing
monitoring vehicles.

## Model

- **Parcels.** The release is split into parcels (3,000 by default),
  released at once or evenly over a release period.
- **Drift.** Each parcel moves with:
  - the surface current;
  - a wind drift of 3 % of the 10 m wind by default (the common "3 %
    rule"), optionally deflected to the right of the wind;
  - a random walk for horizontal eddy diffusion, Δx ~ N(0, 2KΔt) with
    K = 10 m²/s by default.

  Steps are 10 minutes, integrated at the midpoint (RK2). Wind and current
  come from the same 25-point forecast blend as search-and-rescue drift.
- **Spreading.** Parcels start inside the Fay (1971) gravity–viscous radius
  R = 1.45 (Δ g V² t^1.5 / ν^0.5)^(1/6) ten minutes after release (about
  180 m for 100 m³ after an hour); diffusion spreads them after that.
- **Evaporation.** Fingas's empirical equations, %Ev = (a + b·T)·ln t, with
  t in minutes and T in °C. Each parcel evaporates by its own age.

  | Oil | Equation (%Ev) |
  |---|---|
  | Alaska North Slope (2002) | (2.86 + 0.045 T) ln t |
  | Alberta Sweet Mixed Blend | (3.24 + 0.054 T) ln t |
  | Arabian Light (2001) | (2.4 + 0.045 T) ln t |
  | Arabian Medium | (1.89 + 0.045 T) ln t |
  | Arabian Heavy | (1.31 + 0.045 T) ln t |
  | Crude by distillation | [0.165 %D + 0.045 (T − 15)] ln t |

  %D is the percentage distilled at 180 °C. Evaporation is capped at 80 %.
  Refined products such as diesel follow a square-root law in Fingas's
  work; they are not included because their coefficients could not be
  verified for this release.
- **Shoreline.** With a real-world scene (created on the Earth tab), its
  seabed grid is the coastline: a parcel that reaches land (elevation
  ≥ 0) strands at the water's edge and stops evaporating.
- **Mass balance:**
  - afloat, evaporated and ashore volumes;
  - the slick area (100 m cells holding floating oil) and mean thickness;
  - the centre of the floating oil, and the first landfall.

## In the editor (Spill tab)

1. Set the oil, volume, position, how long ago the release began (and over
   how long), and how long to follow it. Use **Earth view centre**,
   **Scene location**, or right-click the sea on the Earth tab → **Oil
   spill here**.
2. Choose *Live forecast* or *Steady* wind and current, and whether to
   strand on this scene's shore. **Model settings** has wind drift,
   deflection, diffusivity, water temperature and parcel count.
3. **▶ Drift spill.** The map shows parcels in orange, fainter as they
   evaporate, red once ashore, with the centre track. The chart gives the
   mass balance over time. The slider and ▶ step through it, and the oil is
   also drawn on the Earth globe.

**Export…** writes `summary.txt`, `mass_balance.csv` (hourly),
`particles.csv` (final state) and `slick.geojson` (floating and stranded
parcels at the end, for GIS).

## Command line

```
Pelagic.exe --spill <lat> <lon> <out-folder> [--oil=ANS|ASMB|ARAB-LIGHT|ARAB-MEDIUM|ARAB-HEAVY|CUSTOM]
  [--distilled=PCT] [--volume=M3] [--ago=H] [--release=H] [--hours=H]
  [--wind=SPEED,FROM --current=SPEED,TOWARDS] [--wind-factor=PCT]
  [--deflection=DEG] [--diffusivity=K] [--temp=C] [--project=PATH]
  [--particles=N] [--seed=N]
```

Exit code 0 when no oil reaches the shore, 2 when some does, 1 on error.

## Limits

- No emulsification, natural dispersion, dissolution, sinking, or response
  operations (booms, skimmers, dispersants). Oil properties do not change
  with weathering.
- Wind and current are forecasts blended between 25 points; the live
  current already includes wave (Stokes) drift, so 2–3 % of the wind is
  usual (some wave drift may be counted twice).
- The coastline is only the scene's seabed grid (about 5 km square for
  Earth-built scenes); beyond it there is no shore.
- Thickness is a mean over occupied cells, not a slick thickness
  distribution.

## References

- Fay, J. A. (1971). Physical processes in the spread of oil on a water
  surface. *Proceedings of the Joint Conference on Prevention and Control
  of Oil Spills*, 463–467.
- Fingas, M. F. (2004). Modeling evaporation using models that are not
  boundary-layer regulated. *Journal of Hazardous Materials* 107, 27–36;
  and Environment Canada's empirical evaporation equations as presented by
  M. Fingas (Freshwater Spills Symposium, 2004).
