# Realistic ocean rendering and real seafloor

The GPU viewport draws the sea, sky, seabed and seabed life from physical
models and real data. This page explains what is simulated, what is
measured data, and where the limits are.

## Sea surface

- **Physics waves, exactly.** The wind sea and swell that move vessels
  (`pelagic::waves::SeaState`, 32 + 16 JONSWAP components) are evaluated per
  vertex on the GPU, so a boat rides the crests you see.
- **Short waves on top (FFT).** The spectral tail above the physics band
  (from 2.5 × the peak frequency down to wavelengths of ~12 cm) is
  synthesised with three 256² FFT cascades (301 m, 57 m, 10.7 m tiles) from
  the same JONSWAP spectrum (Tessendorf 2001). Nothing below the physics
  cut-off is added. The GPU result is tested against a direct sum over the
  same modes (`gpu_fft_matches_a_direct_sum_over_the_spectrum`).
- **Choppy crests.** Horizontal (Gerstner) displacement sharpens crests.
  Physics components use a choppiness of 0.6. At typical steepness this
  moves the drawn surface a few centimetres from the linear surface the
  physics uses: up to ~0.2 m in an 8 m storm.
- **Whitecaps calibrated to observations.** Breaking is where the surface
  Jacobian falls below a threshold chosen so that the breaking fraction
  equals the Monahan & O'Muircheartaigh (1980) whitecap coverage for the
  wind. Foam persists (3 s decay), gathers on the crests of the dominant
  waves and in wave groups, and is streaked along the wind.
- **Wakes.** Moving surface vessels draw a Kelvin wake (19.47° arms,
  transverse wavelength 2πU²/g), stern wash and bow foam. This is a visual
  pattern, not a hull-wave solution.
- **Optics.** Fresnel reflection of the sky and clouds; sun glitter from a
  Beckmann distribution of the unresolved slope variance (mip-filtered
  slope moments, as in Bruneton et al. 2010); light through thin crests;
  refraction of the seabed with spectral absorption; and water colour from
  absorption and scattering. Absorption is (0.45, 0.065, 0.03) per metre
  in red, green and blue, scaled by the scene's clarity.
- **To the true horizon.** The surface is a camera-projected grid whose
  rays meet the curved sea (radius 6,371 km). Rays that miss it end on the
  geometric horizon, sqrt(2Rh) away: about 5 km from 2 m up, 113 km from
  1 km and 357 km from 10 km. Grid density follows the screen.

## Sky and light

- Single-scattering atmosphere (Rayleigh, maritime Mie aerosol, ozone)
  rendered into a sky-view table each frame. Sunlight colour and sky light
  come from the same model on the CPU (`render-gpu/src/sky.rs`, tested).
- Sun position: real solar position (NOAA equations) for real-world scenes.
  The viewport "Time" slider moves the sun along its real path for that
  latitude and day.
- Clouds: a procedural cloud deck at 1.5 km with self-shadowing, cloud
  shadows on the sea and seabed, and a cover slider.
- Bloom on glints and the sun; ACES tone mapping; auto exposure.

## Underwater

- Light fades with depth by colour (red first). Volumetric light shafts are
  integrated along each view ray, following the focusing pattern of the
  surface along the refracted sun. Caustics fall on the seabed.
- Looking up you see Snell's window (true refraction through the moving
  surface) with total internal reflection outside it.

## Seabed, land and life

- Full-resolution scene terrain with materials from height and slope: sand
  ripples, rock, silt and mud with depth, wet and dry beach, vegetation,
  rock and snow on land.
- A backdrop of 257 × 257 points at 320 m (82 km square) is stitched to the
  scene without cracks. It shows real coasts, islands and mountains to the
  horizon (visual only).
- Coral colonies, sea fans, sponges, soft corals, rocks and swaying
  seagrass are placed only where the habitat map says reef or seagrass and
  the depth allows. Shapes and colours are procedural (representative, not
  individual surveyed colonies).

## Real-world scenes ("Create ocean scene here")

| Data | Source | Resolution |
|---|---|---|
| Seafloor depth | NOAA NCEI DEM Global Mosaic (ETOPO 2022 + coastal DEMs) | 15″ (~450 m) open ocean; down to 1/9″ (~3 m) where NOAA has coastal surveys |
| Land | AWS Terrain Tiles (zoom 13) | ~19 m |
| Coral reefs | UNEP-WCMC Global Distribution of Warm-water Coral Reefs v4.1 | map polygons, resampled to 20 m |
| Seagrass | UNEP-WCMC Global Distribution of Seagrasses v7.1 | as above |
| Sea state | Open-Meteo live forecast for the place and hour | model grid |
| Sun | Solar position for the place, now | — |

The scene's provenance records how many nodes came from NOAA and the finest
source cell size at the centre. New scenes open on the nearest mapped reef
(underwater), facing the shore, or on the sea. The viewport **Views** menu
jumps between Aerial, Sea surface, Coral reef, Shore and Seabed.

## Viewport quick bar

Views · Sea (glassy calm to storm presets) · Live sea now (real-world
scenes) · Time of day · Clouds · Water clarity (crystal tropical to turbid
harbour).

## Limits

- "Every detail of the sea" is limited by the data that exist. Only about a
  quarter of the seafloor has been mapped directly by ship sonar. Elsewhere
  depth comes from satellite gravity models at kilometre scale, so the open
  ocean is ~450 m resolution. Shallow coasts outside NOAA's surveys rely on
  the global composite.
- Reef and seagrass maps are global compilations of varying age and
  accuracy; they say where habitat is, not what each colony looks like.
- Short FFT waves are visual. Floating bodies respond only to the physics
  components.
- Not a calibrated optical sensor. Camera-sensor exports still use the CPU
  reference renderer.
- Iris Xe (1430×630 viewport, quality Medium): about 36–45 fps on the open
  sea; 36 fps underwater on a reef.

## Startup, marine life and graphics settings

- **Startup.** With nothing else to open, Pelagic starts on a real sea: your last real-world scene, else Kavaratti atoll in Lakshadweep. It is built once from live data and cached as `%LOCALAPPDATA%\Pelagic\startup-sea.pelagic`. Real-world scenes take the live sea for the current hour when opened.
- **Land colour.** Sentinel-2 satellite colour is draped on land in the scene and its backdrop.
- **Marine life.** The groups recorded within ~50 km in OBIS decide which animals swim in the view: sharks, rays, sea turtles, whales and dolphins, reef fish. The Life here menu lists the species. The shapes are procedural and represent the groups; they are not tracked individuals.
- **Graphics menu** (top right):
  - Low, Medium, High or Ultra: sea grid density, light-shaft steps and seabed-life budgets;
  - V-Sync off for an uncapped frame rate;
  - render scale from 50% to 200%;
  - a sea that keeps moving while the simulation is paused.

  Dedicated GPUs start on Ultra.

## Flying, the compass and reaching land

- **Controls.** Drag with the left or right mouse button to look around; the
  pointer is hidden and held in the window while you do.
  - Move with WASD or the arrow keys; go up and down with Q/E or PageUp/PageDown.
  - With an actor selected, W/E/R choose tools instead, and the arrows still move.
  - Shift moves 5× faster; + and − double or halve the speed.
  - Movement accelerates and brakes smoothly.
  - The camera never goes into the ground or the seabed.
- **Speed.** Set it with the slider (0.5 m/s to 20 km/s) or a preset: Swim, Boat, Fast boat, Helicopter, Plane, Jet or Rocket.
- **Navigation display.** Top left: latitude and longitude, depth (and the seabed below) or altitude, speed in m/s, knots and km/h, heading and pitch. A compass tape shows the heading, and a green marker gives the bearing and distance of the nearest land.
- **Buttons.**
  - **Go to land** flies to just off the nearest coast and looks at it.
  - **Surface** and **Dive** move between above and below the water.
  - **Scene** returns to the scene's own viewpoint.
  - **Build detailed scene here** rebuilds the 20 m scene where you are (as Explore in 3D does).

## The world beyond the scene

Real terrain and seabed stream in around the camera as it moves, in three
rings:

| Ring | Size | Spacing | Elevation | Land colour |
|---|---|---|---|---|
| Near | 41 km | 160 m | NOAA DEM mosaic + terrain tiles (z11) | Sentinel-2 (z10) |
| Middle | 328 km | 1.28 km | as above (z8) | Sentinel-2 (z7) |
| Far | 1,280 km | 10 km | as above (z5) | Sentinel-2 (z4) |

- Each ring leaves out the area that the scene or a finer ring draws.
- Rings re-centre when you have moved about a quarter of their size; tiles are cached on disk.
- Everything is drawn on the curved Earth, so a coast 100 km away sits 785 m below your horizontal, as it should.
- The depth buffer is reversed with no far plane (depth = 0.1 m / distance), so it has the same relative precision at every range. Distant coasts and seabed no longer flicker against each other.
- Meteorological visibility sets the haze. With Live on it comes from the nearest airport report, or else the model.

## Trees and night

- **Trees** are placed where the ground is above sea level and the satellite colour is vegetation green, out to 5 km (finer spacing within 1.2 km):
  - coconut palms on tropical land below 25 m;
  - conifers above 48° latitude or 1,500 m;
  - broadleaf trees elsewhere.

  They are drawn as instances, cast shadows, and are capped by the graphics preset (600 to 5,000). Positions are representative (one candidate per cell), not surveyed trees.
- **Night.** With Live on, the sun is where it really is, so at night the sea and seabed are dark. A notice offers **See it by day**: the live sea and weather stay, with the light set to 10:00 local solar time. **Real sun** returns to the true sky.

## Marine animals

Animals swim around the camera wherever you go:
- **Species.** They come from the OBIS checklists for the half-degree area around the camera: bony fish, sharks and rays, sea turtles, whales and dolphins, sirenians and seals. The most recorded species that have models are used, up to 14, with no more than three per family.
- **Models.** Each is built from its family's body plan:
  - real proportions (body depth and width, head shape, tail stem);
  - fin layout and tail type (forked, lunate, rounded, truncate, heterocercal, flukes);
  - the right locomotion, drawn on the GPU: body and tail, whole body (eels), flapping wings (rays), vertical flukes (whales and dolphins) or flipper strokes (turtles).
- **Coverage.** The catalogue covers 111 families. 268 species have their own colouring (e.g. powder blue tang, Moorish idol, yellowfin tuna, blacktip reef shark, manta, green turtle, spinner dolphin), as do 17 genera. Others use their family's typical colouring.
- **Behaviour.**
  - Reef fish stay over reefs, pelagic schools in open water, rays and flatfish on the bottom, and dolphins near the surface. Each keeps to its depth range.
  - Schools hold formation; loose groups mill around.
  - Animals steer around the seabed and turn away from the camera.
  - Far behind you they are replaced by new animals at the edge of view.
- **Life here** lists every species shown, with its scientific name, family and number of records. If an area has no records (or you are offline), representative species are shown only in tropical seas, and the menu says so.

Animal positions are simulated; only which species occur here is data.
Close animals use a detailed mesh, distant ones a light one.

## Live and forecast

- **● Live** shows the sea and weather here now:
  - waves from the nearest NOAA buoy within 200 km if one reports, else the model value for this 15-minute step;
  - wind, visibility and cloud from the nearest airport report (METAR) within 150 km, else the model;
  - the sun at its real position (night stays night);
  - live AIS ships within 80 km, drawn in 3D with their wakes and labelled with name, type, speed and distance.

  It refreshes every 10 minutes and stays on the present; it never jumps an hour ahead.
- **Forecast** shows the predicted sea and weather in 1, 3, 6, 12 or 24 hours, or 2, 3 or 5 days, with the sun where it will be. Live switches off.
- The **Details panel** has a "Here and now" section:
  - position, the maritime zones you are in (Marine Regions), local time;
  - weather, visibility, wind, waves, swell, sea temperature and current, each with its source and age;
  - sunrise and sunset, the moon phase;
  - the nearest land, ships nearby and the species here.
