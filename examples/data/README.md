# Reference data and asset attribution

`goa-etopo1.csv` and `world-etopo1.csv` were retrieved from NOAA CoastWatch ERDDAP's ETOPO1 Ice Surface dataset on 14 November 2026 UTC. The local source covers latitude 14.8–15.2°, longitude 73.3–73.7° at one arc-minute spacing. The global overview samples every 120th source node (2°). Header/unit rows are retained. The regional loader bilinearly resamples to an isotropic metre grid; the desktop embeds that processed grid and its origin/source metadata.

ETOPO1 combines land topography and ocean bathymetry. These are coarse composite/resampled data, not a newly surveyed local bathymetric chart. The overview must not be interpreted as local detail; rendering materials, currents, waves and life remain synthetic. NOAA's source conditions state that ETOPO1 is not for navigation. Its source vertical data have not all been converted to a common datum.

Credit: Amante, C. and B.W. Eakins (2009), *ETOPO1 1 Arc-Minute Global Relief Model: Procedures, Data Sources and Analysis*, NOAA Technical Memorandum NESDIS NGDC-24, [doi:10.7289/V5C8276M](https://doi.org/10.7289/V5C8276M). NOAA makes the data public domain and requests attribution; see the [NCEI FAQ](https://www.ngdc.noaa.gov/mgg/global/dem_faq.html), [dataset metadata](https://www.ncei.noaa.gov/metadata/geoportal/rest/metadata/item/gov.noaa.ngdc.mgg.dem%3A316/html), and [ERDDAP dataset](https://coastwatch.pfeg.noaa.gov/erddap/griddap/etopo180.html).

`current-grid.csv` is a Pelagic-generated synthetic complete XYZ/time SI lattice. `inspection-frame.obj` is a Pelagic-generated geometry-only box sample. Neither represents measured field conditions or a commercial vehicle. The supplied app logo remains the project owner's artwork; bundled Geist uses its included SIL Open Font License.
