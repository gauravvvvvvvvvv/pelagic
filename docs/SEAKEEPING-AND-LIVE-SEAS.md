# Seakeeping and live seas

Status: 23 November 2026.

Real sea states from global forecasts drive scenes, and the seakeeping
module turns a vehicle's motions into operating limits and a go/no-go
outlook for the coming week. Scope: platforms and their navigation and
survey work. Weapon payloads and targeting are outside the product's scope.

## Live seas (Earth tab)

- **〰 Live waves** shows the global wave-height forecast on the globe:
  colour grows with significant wave height (calm water stays clear), arrows
  show the direction the waves travel. The slider and ▶ step through three
  days hour by hour. Hovering the sea shows height, period and direction.
- **Sea forecast** (toolbar, at the view centre) or right-click → **Sea
  forecast here** opens a 7-day hourly forecast for that point: total sea,
  wind sea and swell (height, period, direction), wind, surface current and
  sea temperature, with a chart. **Apply to current scene** sets the scene's
  sea from the chosen hour (undoable); **Create scene here** builds a scene
  on the real seabed with that hour's sea.
- **Live sea in new scenes** (on by default): every ocean scene created on
  the globe starts with the current hour's forecast sea. If the forecast
  cannot be reached, the scene keeps its default sea.

What a scene receives: wind sea as a JONSWAP spectrum (32 components,
γ 3.3, directional spread from the scene), swell as a separate narrow
spectrum (16 components, γ 7, 10° spread) travelling its own way, wind
speed (which sets whitecap coverage), surface current and surface
temperature. Directions are converted from compass "from" bearings to the
scene's axes (X east, Y north). Where the data give mean periods only, the
peak period is taken as mean / 0.9. Heights beyond the scene's linear-wave
envelope (steepness, or a quarter of the water depth) are clamped and the
Output Log says so. The scene records the source and hour in **Ocean
environment > Waves > Source** and in the saved file.

### Data source, licence and privacy

Data come from [Open-Meteo](https://open-meteo.com) (wave models by ECMWF,
Météo-France / Copernicus Marine, NOAA NCEP and DWD; wind from its weather
API), licensed CC BY 4.0. Pelagic shows the attribution wherever the data
appear. The free service is for **non-commercial use** (under 10,000
requests a day); commercial users need an Open-Meteo API subscription and
set `PELAGIC_OPEN_METEO_KEY`, which switches to the customer servers.

Requests send only coordinates and the variables wanted, with a generic
user agent. Responses are cached in `%LOCALAPPDATA%\Pelagic\forecasts` for
one 6-hour model cycle; the global map (576 points on a 10° grid) is
fetched at most once per cycle and paced under 600 locations a minute.
Forecasts are model predictions, not observations, and are not suitable for
navigation.

## Seakeeping (Seakeeping tab)

For the selected vehicle the module runs long-crested irregular seas
(JONSWAP, Hs 1 m reference) for each peak period and relative heading
(0° following, 90° beam, 180° head seas), at its transit speed with heading
held, and measures RMS heave, roll and pitch, vertical acceleration at the
bow and at a working point, felt lateral acceleration at the working point,
and the bow's motion and velocity relative to the water surface. Responses
scale linearly with wave height, so each cell gives:

- a **limiting significant wave height** for the chosen criteria and the
  criterion that sets it (table, colour = more operable when lighter);
- motions in any forecast sea: wind sea and swell are interpolated in the
  table for their own period and heading and combined by adding variances;
- slamming (Ochi: bow emergence with relative velocity above 0.093·√(g·L))
  and deck wetness probabilities from Rayleigh statistics of the relative
  motion, using the bow draft and freeboard at rest.

Criteria presets: NORDFORSK 1987 general operability limiting criteria for
naval vessels, merchant ships (L ≤ 100 m) and fast small craft (RMS values;
bow = forward perpendicular, working point = bridge), and an example for
survey / launch and recovery. Every limit can be edited or switched off;
confirm values against the standard your programme uses.

**Operability outlook:** load a forecast (Earth tab, or **Forecast at this
scene's location** for real-world scenes), set the planned course, and the
tab shows the share of operable hours, go/no-go now with the limiting
criterion, an hour strip (hover for the motions) and roll, pitch and
acceleration against their limits.

## Command line

```
Pelagic.exe --forecast <lat> <lon> <out-folder>
Pelagic.exe --seakeeping <vehicle.pelagic-vehicle.json> <report-folder>
    [--quick] [--speed=F] [--criteria=naval|merchant|fast|survey]
    [--forecast=LAT,LON --course=DEG] [--threads=N]
```

`--forecast` writes `forecast.json` and `forecast.csv`. `--seakeeping`
writes `seakeeping.json`, `responses.csv` (per-metre responses and limiting
heights), `summary.txt` and, with a forecast, `outlook.csv`; it exits 2 when
the vehicle is not operable in the current forecast hour. The full 63-cell
table takes about 15 s on a laptop.

## Limits

- Linear scaling from the reference sea: where the reference run already
  shows large motions (a warning names the cell), heights far from 1 m are
  approximate. Seas are long-crested in the table.
- Hydrodynamics are those of the vehicle model: box hull, column
  hydrostatics, diagonal added mass, waterline damping ζ 0.12, fins as flat
  plates; no diffraction or radiation forces beyond that damping, no
  forward-speed hydrodynamic lift on the hull.
- The global map is coarse (10°) and shows the model's best-match blend; the
  point forecast uses the service's nearest sea grid cell (about 8–25 km).
