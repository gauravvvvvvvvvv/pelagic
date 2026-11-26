# Earth view: marine layers, live data and fleet tracking

Open **Earth → ☰ Layers**. Every layer can be switched on or off with one
click. All data are public; sources and licences are shown on the map
while a layer is on.

| Layer | What you see | Source (licence) | Updates |
|---|---|---|---|
| Coral reefs | Warm-water reef extent | UNEP-WCMC v4.1 (General Data License, non-commercial) | static |
| Seagrass | Seagrass meadows | UNEP-WCMC v7.1 (same) | static |
| Undersea feature names | ~5,400 named seamounts, plateaus, ridges, trenches, canyons, basins… (searchable in the box) | IHO-IOC GEBCO Gazetteer via NOAA NCEI | static |
| Islands | Island outlines | USGS / Esri / UNEP-WCMC Global Islands | static |
| Shipping lanes | Major, middle and minor sea routes | Benden (2022) Shipping Lanes, CC BY 4.0 | static |
| Ports | ~1,000 major ports | Natural Earth (public domain) | static |
| Lighthouses, buoys, harbours, traffic schemes | Nautical seamarks (zoom in) | OpenSeaMap, CC BY-SA 2.0 / OpenStreetMap ODbL | tiles |
| Maritime boundaries / EEZ | National maritime boundaries | Marine Regions (VLIZ), CC BY 4.0 | static |
| Sea and ocean names | Oceans, seas, gulfs, bays | Natural Earth (public domain) | static |
| Live waves | Global wave height and direction; a finer grid over the view when zoomed in | Open-Meteo (CC BY 4.0) | hourly |
| Wave buoys | ~800 reporting buoys: wave height, period, wind, water temperature | NOAA NDBC (public domain) | 10 min |
| Ships (AIS) | Live vessels with name, type, speed, course, destination, size, track | Fintraffic Digitraffic (Baltic, CC BY 4.0); aisstream.io worldwide with your free key | live |
| Marine life records | Sharks, rays, whales and dolphins, turtles, seals, fish, corals, seabirds in view | OBIS occurrence records | on demand |
| My fleet | Your vessels, tracks, predictions, warnings | you | live |
| Sea surface temperature | Daily SST analysis (colour scale) | GHRSST L4 MUR via NASA GIBS (public domain) | daily |

Click a ship, buoy, port, record or fleet vessel for its details. A ship's
recorded track and dead-reckoned course for the prediction horizon are
drawn while it is selected.

## Ships

- No account needed: the Baltic Sea (Finnish Digitraffic feed).
- Worldwide: paste a free key from [aisstream.io](https://aisstream.io) in
  the Layers menu. The key is stored only on this computer
  (`%LOCALAPPDATA%\Pelagic\aisstream.key`).
- Ship symbols are coloured by type: cargo, tanker, passenger, fishing,
  tug and service, search and rescue, law enforcement and medical, sailing
  and pleasure, high speed, other. Positions glide forward by dead
  reckoning between reports, for up to ten minutes.
- Public AIS does not carry the number of people on board. Many naval
  vessels, and some government vessels, do not transmit. Aircraft and
  drones are not AIS targets and are not shown.

## My fleet

Add vessels from the window or by right-clicking the map ("Add my vessel
here"). A vessel can follow a live AIS MMSI, or be updated by hand with
"Report at view centre"; course and speed are then derived from successive
reports. For each vessel the map shows:

- the recorded track;
- the estimated position now;
- a dead-reckoned prediction with ten-minute ticks for a chosen horizon
  (0–240 min).

Closest-approach warnings list every other vessel (AIS or fleet) predicted
to pass within 1 nm inside the horizon. They use flat-earth CPA/TCPA, which
is adequate within a few tens of miles. The fleet is saved to
`%LOCALAPPDATA%\Pelagic\fleet.json`.

## Not included

Tracking of military submarine or warship deployments is deliberately not
included, even from public reports: Pelagic does not do weapons, targeting
or military intelligence.

TeleGeography's submarine cable data is commercially licensed, so it is not
used. Where OpenStreetMap records cables, OpenSeaMap seamarks may show
them.

## Scripts (captures)

`globe`, `goto:<lat>,<lon>,<km>[,<tilt>[,<heading>]]`, `reefs`, `seagrass`,
`names`, `find:<feature name>`,
`layer:<lanes|ports|seas|buoys|ships|life|fleet|waves|seamarks|boundaries|islands>`,
`vessel:<lat>,<lon>,<knots>,<course>,<name>`.

## Click anywhere

Click any point on the globe (not on a ship or marker) for a place card:
- **Sentinel-2 satellite view** of about 2.4 km;
- **depth or height**;
- **live sea:** waves, swell, wind, current and sea temperature;
- **nearest port.**

**Explore in 3D** builds that place in the viewport. On land, in a port or in shallow water, the scene is centred on the nearest open water and the camera looks back at the place you clicked.

## Earthquakes

**Layers → Earthquakes (USGS, live)** shows magnitude 2.5+ earthquakes of the past week, refreshed every 5 minutes:
- **colour** by age: red is the last hour, orange the last day, yellow this week;
- **size** by magnitude.

The Seismic activity window lists the strongest events with tsunami flags and Go buttons. Click a circle for the place, depth, time and alert level. Data: USGS Earthquake Hazards Program (public domain). Pelagic shows the feed; it is not a warning system, so follow official tsunami warnings.

## Base maps

- **Satellite + seafloor** (default): Sentinel-2 imagery on land and in shallow water, bathymetry relief in deep water.
- **Satellite (Sentinel-2, 10 m)**, **Bathymetry relief** and **Satellite (Blue Marble)** are the other choices.
- **Satellite yesterday (NASA VIIRS, live)**: the latest complete day of true-colour imagery from NOAA-20. It shows that day's clouds, storms, plankton blooms and river sediment, at up to about 300 m per pixel (zoom 9). Public domain, via NASA GIBS.
- **Layers → Sea surface temperature (GHRSST, daily)**: the MUR Level 4 analysis from two days ago, semi-transparent over any base map. Public domain, via NASA GIBS.
- Sentinel-2 cloudless 2016 by EOX IT Services GmbH (contains modified Copernicus Sentinel data 2016), CC BY 4.0. Where 2016 masks small islands with flat ocean colour, the 2024 mosaic (CC BY-NC-SA 4.0, non-commercial) is used.
