//! Earth view: a streamed 3D globe built from real, openly licensed data.
//!
//! * Bathymetry relief (default): AWS Terrain Tiles "terrarium" elevation
//!   (GEBCO / ETOPO1 / SRTM composite, open data), hillshaded and depth-tinted
//!   locally. Zoom 0–15.
//! * Satellite: NASA GIBS Blue Marble shaded relief + bathymetry (public
//!   domain), zoom 0–8, upsampled deeper.
//!
//! Geometry is projected in f64 on the CPU (no precision jitter when zoomed
//! in to a few hundred metres) and drawn as textured egui meshes on the GPU.
//! Tiles are cached under `%LOCALAPPDATA%\Pelagic\tiles`. Tile requests send
//! only tile coordinates to the public data hosts.
#[path = "marine.rs"]
pub(super) mod marine;
#[path = "place.rs"]
mod place;
#[path = "sea_layer.rs"]
mod sea_layer;
#[path = "seafloor.rs"]
pub(super) mod seafloor;
use super::theme;
use crate::project::Scene;
use egui::{Color32, Pos2, Rect, RichText, Sense, Vec2};
use pelagic::metocean::{Conditions, Forecast};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex, mpsc},
    time::Instant,
};

const R: f64 = 6371.0; // km
const FOV: f64 = 45.0;
const MAX_LAT: f64 = 85.051_128_779_806_59;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Layer {
    Relief,
    Satellite,
    /// Overlays: UNEP-WCMC warm-water coral reef and seagrass extents.
    Reefs,
    Seagrass,
    /// Overlays: islands, maritime boundaries (EEZ), OpenSeaMap seamarks.
    Islands,
    Boundaries,
    Seamarks,
    /// Sentinel-2 cloudless satellite mosaic (10 m) and the default blend
    /// of it with bathymetry relief.
    Sentinel,
    Hybrid,
    /// Live satellite: yesterday's complete NASA VIIRS true-colour pass
    /// (clouds, storms, plankton blooms, sediment plumes as they were).
    Today,
    /// Overlay: daily sea-surface temperature analysis (GHRSST MUR).
    Sst,
}
/// UTC date `days` ago ("YYYY-MM-DD").
fn date_ago(days: i64) -> String {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    pelagic::metocean::format_utc(unix - days * 86_400)[..10].to_string()
}
impl Layer {
    /// The image date for live layers.
    fn date(self) -> Option<String> {
        match self {
            Layer::Today => Some(date_ago(1)),
            Layer::Sst => Some(date_ago(2)),
            _ => None,
        }
    }
    fn max_zoom(self) -> u8 {
        match self {
            Layer::Relief => 15,
            Layer::Satellite => 8,
            Layer::Reefs | Layer::Seagrass | Layer::Islands => 12,
            Layer::Boundaries => 10,
            Layer::Seamarks => 17,
            Layer::Sentinel | Layer::Hybrid => 15,
            Layer::Today => 9,
            Layer::Sst => 7,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Layer::Relief => "Bathymetry relief",
            Layer::Satellite => "Satellite (Blue Marble)",
            Layer::Reefs => "Coral reefs",
            Layer::Seagrass => "Seagrass",
            Layer::Islands => "Islands",
            Layer::Boundaries => "Maritime boundaries",
            Layer::Seamarks => "Seamarks",
            Layer::Sentinel => "Satellite (Sentinel-2, 10 m)",
            Layer::Hybrid => "Satellite + seafloor",
            Layer::Today => "Satellite yesterday (NASA VIIRS, live)",
            Layer::Sst => "Sea surface temperature (GHRSST, daily)",
        }
    }
    fn dir(self) -> &'static str {
        match self {
            Layer::Relief => "terrarium",
            Layer::Satellite => "bluemarble",
            Layer::Reefs => "reefs",
            Layer::Seagrass => "seagrass",
            Layer::Islands => "islands",
            Layer::Boundaries => "eez",
            Layer::Seamarks => "seamarks",
            Layer::Sentinel => "s2cloudless",
            Layer::Hybrid => "hybrid",
            Layer::Today => "viirs",
            Layer::Sst => "sst",
        }
    }
    fn url(self, z: u8, x: u32, y: u32) -> String {
        match self {
            Layer::Relief => {
                format!("https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png")
            }
            Layer::Satellite => format!(
                "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/BlueMarble_ShadedRelief_Bathymetry/default/GoogleMapsCompatible_Level8/{z}/{y}/{x}.jpeg"
            ),
            Layer::Reefs | Layer::Seagrass => {
                // The tile's Web Mercator box, rendered by the map service.
                let n = 2f64.powi(z as i32);
                let size = 40_075_016.685_578_5 / n;
                let west = -20_037_508.342_789_244 + x as f64 * size;
                let north = 20_037_508.342_789_244 - y as f64 * size;
                let habitat = if self == Layer::Reefs {
                    seafloor::Habitat::CoralReefs
                } else {
                    seafloor::Habitat::Seagrass
                };
                habitat.export_url([west, north - size, west + size, north], 3857, 256, 256)
            }
            Layer::Islands | Layer::Boundaries => {
                let n = 2f64.powi(z as i32);
                let size = 40_075_016.685_578_5 / n;
                let west = -20_037_508.342_789_244 + x as f64 * size;
                let north = 20_037_508.342_789_244 - y as f64 * size;
                let (s, e) = (north - size, west + size);
                if self == Layer::Islands {
                    format!(
                        "https://data-gis.unep-wcmc.org/server/rest/services/Global_Islands/MapServer/export?bbox={west:.3},{s:.3},{e:.3},{north:.3}&bboxSR=3857&imageSR=3857&size=256,256&layers=show:0,1,2&format=png32&transparent=true&dpi=96&f=image"
                    )
                } else {
                    format!(
                        "https://geo.vliz.be/geoserver/MarineRegions/wms?service=WMS&version=1.1.1&request=GetMap&layers=MarineRegions:eez_boundaries&styles=&srs=EPSG:3857&bbox={west:.3},{s:.3},{e:.3},{north:.3}&width=256&height=256&format=image/png&transparent=true"
                    )
                }
            }
            Layer::Seamarks => format!("https://tiles.openseamap.org/seamark/{z}/{x}/{y}.png"),
            Layer::Sentinel | Layer::Hybrid => format!(
                "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless_3857/default/g/{z}/{y}/{x}.jpg"
            ),
            Layer::Today => format!(
                "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_NOAA20_CorrectedReflectance_TrueColor/default/{}/GoogleMapsCompatible_Level9/{z}/{y}/{x}.jpg",
                self.date().unwrap_or_default()
            ),
            Layer::Sst => format!(
                "https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/GHRSST_L4_MUR_Sea_Surface_Temperature/default/{}/GoogleMapsCompatible_Level7/{z}/{y}/{x}.png",
                self.date().unwrap_or_default()
            ),
        }
    }
    fn attribution(self) -> &'static str {
        match self {
            Layer::Relief => {
                "Elevation: AWS Terrain Tiles (Mapzen) — GEBCO, ETOPO1, SRTM and others · open data"
            }
            Layer::Satellite => {
                "Imagery: NASA Blue Marble shaded relief & bathymetry via GIBS · public domain"
            }
            Layer::Reefs | Layer::Seagrass => seafloor::HABITAT_CREDIT,
            Layer::Islands => marine::ISLANDS_CREDIT,
            Layer::Boundaries => marine::BOUNDARY_CREDIT,
            Layer::Seamarks => marine::SEAMARK_CREDIT,
            Layer::Sentinel => SENTINEL_CREDIT,
            Layer::Hybrid => {
                "Sentinel-2 cloudless 2016 by EOX IT Services GmbH (Copernicus Sentinel data 2016), CC BY 4.0; small islands from Sentinel-2 cloudless 2024 (Copernicus Sentinel data 2024), CC BY-NC-SA 4.0 · Elevation: AWS Terrain Tiles (Mapzen) · open data"
            }
            Layer::Today => {
                "Imagery: NASA VIIRS (NOAA-20) corrected reflectance, yesterday's pass, via NASA GIBS · public domain"
            }
            Layer::Sst => {
                "Sea surface temperature (blue cold → red warm): GHRSST Level 4 MUR (NASA JPL), via NASA GIBS · public domain"
            }
        }
    }
}

type Key = (Layer, u8, u32, u32);

/// HTTPS client using the Windows TLS stack and any configured proxy. Sends a
/// generic user agent and tile coordinates only.
pub(super) fn http_agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(timeout)))
        .user_agent("Pelagic-ocean-engine/1.0")
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .proxy(ureq::Proxy::try_from_env())
        .build()
        .into()
}

struct Decoded {
    key: Key,
    rgba: Option<(Vec<u8>, Option<Vec<f32>>)>,
    error: Option<String>,
}

struct Queue {
    jobs: Mutex<VecDeque<Key>>,
    wake: Condvar,
}

/// Background tile fetch/decode workers with an on-disk cache.
struct Loader {
    queue: Arc<Queue>,
    results: mpsc::Receiver<Decoded>,
}
impl Loader {
    fn new() -> Self {
        let queue = Arc::new(Queue {
            jobs: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
        });
        let (send, results) = mpsc::channel();
        let agent: ureq::Agent = http_agent(20);
        let cache = crate::project::data_dir().join("tiles");
        for _ in 0..6 {
            let queue = queue.clone();
            let send = send.clone();
            let agent = agent.clone();
            let cache = cache.clone();
            std::thread::spawn(move || {
                loop {
                    let key = {
                        let mut jobs = queue.jobs.lock().unwrap();
                        while jobs.is_empty() {
                            jobs = queue.wake.wait(jobs).unwrap();
                        }
                        jobs.pop_back().unwrap()
                    };
                    let result = match key.0 {
                        Layer::Relief => relief(&agent, &cache, key.1, key.2, key.3)
                            .map(|elev| (shade(&elev, key.1, key.3), Some(elev.to_vec()))),
                        Layer::Hybrid => hybrid(&agent, &cache, key),
                        Layer::Sentinel => {
                            sentinel_rgba(&agent, &cache, key.1, key.2, key.3).map(|t| (t, None))
                        }
                        Layer::Satellite
                        | Layer::Reefs
                        | Layer::Seagrass
                        | Layer::Islands
                        | Layer::Boundaries
                        | Layer::Seamarks
                        | Layer::Today
                        | Layer::Sst => {
                            fetch(&agent, &cache, key).and_then(|bytes| decode(key, &bytes))
                        }
                    };
                    let message = match result {
                        Ok(data) => Decoded {
                            key,
                            rgba: Some(data),
                            error: None,
                        },
                        Err(e) => Decoded {
                            key,
                            rgba: None,
                            error: Some(e),
                        },
                    };
                    if send.send(message).is_err() {
                        return;
                    }
                }
            });
        }
        Self { queue, results }
    }
    /// Queues `key` (newest first) and returns stale requests dropped from
    /// the bounded queue, so the caller can request them again later.
    fn request(&self, key: Key) -> Vec<Key> {
        let mut jobs = self.queue.jobs.lock().unwrap();
        jobs.retain(|k| *k != key);
        jobs.push_back(key);
        let mut dropped = vec![];
        while jobs.len() > 96 {
            dropped.extend(jobs.pop_front());
        }
        self.queue.wake.notify_one();
        dropped
    }
}

fn fetch(agent: &ureq::Agent, cache: &std::path::Path, key: Key) -> Result<Vec<u8>, String> {
    let (layer, z, x, y) = key;
    let ext = if matches!(
        layer,
        Layer::Satellite | Layer::Sentinel | Layer::Hybrid | Layer::Today
    ) {
        "jpg"
    } else {
        "png"
    };
    // Live layers are cached per image date.
    let dir = match layer.date() {
        Some(d) => cache.join(layer.dir()).join(d),
        None => cache.join(layer.dir()),
    };
    let path = dir
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.{ext}"));
    if let Ok(bytes) = std::fs::read(&path)
        && !bytes.is_empty()
    {
        return Ok(bytes);
    }
    let mut response = agent
        .get(&layer.url(z, x, y))
        .call()
        .map_err(|e| e.to_string())?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(4 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        let tmp = path.with_extension("part");
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
    Ok(bytes)
}

/// Decodes a tile to 256×256 RGBA; relief tiles also return elevations (m).
fn decode(key: Key, bytes: &[u8]) -> Result<(Vec<u8>, Option<Vec<f32>>), String> {
    match key.0 {
        Layer::Satellite | Layer::Sentinel | Layer::Hybrid | Layer::Today => {
            let mut decoder = zune_jpeg::JpegDecoder::new(bytes);
            let rgb = decoder.decode().map_err(|e| format!("{e:?}"))?;
            let (w, h) = decoder.dimensions().ok_or("jpeg size")?;
            if w * h * 3 != rgb.len() {
                return Err("unexpected jpeg layout".into());
            }
            let mut rgba = Vec::with_capacity(w * h * 4);
            for p in rgb.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            Ok((resample(&rgba, w, h), None))
        }
        Layer::Relief => {
            let elev = terrarium(bytes)?;
            Ok((shade(&elev, key.1, key.3), Some(elev)))
        }
        Layer::Seamarks => {
            let (rgba, w, h) = seafloor::decode_png(bytes)?;
            Ok((resample(&rgba, w, h), None))
        }
        Layer::Sst => {
            let (mut rgba, w, h) = seafloor::decode_png(bytes)?;
            // Partly see-through so coasts and relief show beneath.
            for p in rgba.as_chunks_mut::<4>().0 {
                p[3] = (p[3] as u32 * 170 / 255) as u8;
            }
            Ok((resample(&rgba, w, h), None))
        }
        Layer::Islands | Layer::Boundaries => {
            let (rgba, w, h) = seafloor::decode_png(bytes)?;
            let tint = if key.0 == Layer::Islands {
                [255, 236, 160]
            } else {
                [255, 120, 220]
            };
            let src = resample(&rgba, w, h);
            // Islands are drawn as outlines (filled shapes would hide the
            // relief); boundaries are already lines.
            let solid = |x: i32, y: i32| {
                let (x, y) = (x.clamp(0, 255) as usize, y.clamp(0, 255) as usize);
                src[(y * 256 + x) * 4 + 3] > 30
            };
            let mut out = vec![0u8; 256 * 256 * 4];
            for y in 0..256i32 {
                for x in 0..256i32 {
                    let edge = key.0 == Layer::Boundaries
                        || [(1, 0), (-1, 0), (0, 1), (0, -1)]
                            .iter()
                            .any(|(dx, dy)| !solid(x + dx, y + dy));
                    if solid(x, y) && edge {
                        let i = (y as usize * 256 + x as usize) * 4;
                        out[i..i + 4].copy_from_slice(&[tint[0], tint[1], tint[2], 220]);
                    }
                }
            }
            Ok((out, None))
        }
        Layer::Reefs | Layer::Seagrass => {
            // Recoloured to one clear tint per habitat; empty pixels clear.
            let (rgba, w, h) = seafloor::decode_png(bytes)?;
            let tint = if key.0 == Layer::Reefs {
                [255, 104, 76]
            } else {
                [64, 214, 112]
            };
            let mut out = resample(&rgba, w, h);
            for p in out.as_chunks_mut::<4>().0 {
                if p[3] > 30 {
                    *p = [tint[0], tint[1], tint[2], 215];
                } else {
                    *p = [0; 4];
                }
            }
            Ok((out, None))
        }
    }
}

fn resample(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    if (w, h) == (256, 256) {
        return rgba.to_vec();
    }
    let mut out = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256 {
        for x in 0..256 {
            let i = ((y * h / 256) * w + x * w / 256) * 4;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    out
}

type ReliefCache = Mutex<VecDeque<((u8, u32, u32), Arc<Vec<f32>>)>>;
static RELIEF: ReliefCache = Mutex::new(VecDeque::new());

/// Elevations for a relief tile with no-data gaps filled. Above zoom 8 the
/// open terrain tiles leave much of the ocean at exactly 0 m (no bathymetry
/// at that resolution); those pixels are filled from the parent tile, which
/// recursively bottoms out in the complete global ETOPO/GEBCO zooms.
fn relief(
    agent: &ureq::Agent,
    cache: &std::path::Path,
    z: u8,
    x: u32,
    y: u32,
) -> Result<Arc<Vec<f32>>, String> {
    if let Some((_, e)) = RELIEF.lock().unwrap().iter().find(|(k, _)| *k == (z, x, y)) {
        return Ok(e.clone());
    }
    let mut elev = terrarium(&fetch(agent, cache, (Layer::Relief, z, x, y))?)?;
    if z > 8 && elev.contains(&0.0) {
        let parent = relief(agent, cache, z - 1, x / 2, y / 2)?;
        let (ox, oy) = ((x % 2) as f32 * 128.0, (y % 2) as f32 * 128.0);
        for py in 0..256 {
            for px in 0..256 {
                let i = py * 256 + px;
                if elev[i] != 0.0 {
                    continue;
                }
                // Bilinear sample of the parent at this pixel centre.
                let sx = (ox + (px as f32 + 0.5) * 0.5 - 0.5).clamp(0.0, 255.0);
                let sy = (oy + (py as f32 + 0.5) * 0.5 - 0.5).clamp(0.0, 255.0);
                let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(255), (y0 + 1).min(255));
                let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
                let p = |xx: usize, yy: usize| parent[yy * 256 + xx];
                elev[i] = p(x0, y0) * (1.0 - tx) * (1.0 - ty)
                    + p(x1, y0) * tx * (1.0 - ty)
                    + p(x0, y1) * (1.0 - tx) * ty
                    + p(x1, y1) * tx * ty;
            }
        }
    }
    let elev = Arc::new(elev);
    let mut memo = RELIEF.lock().unwrap();
    memo.push_back(((z, x, y), elev.clone()));
    while memo.len() > 48 {
        memo.pop_front();
    }
    Ok(elev)
}

/// Terrarium PNG → elevations in metres (256×256, row 0 = north).
pub fn terrarium(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut data = vec![0; reader.output_buffer_size().ok_or("png size")?];
    let info = reader.next_frame(&mut data).map_err(|e| e.to_string())?;
    let channels = info.color_type.samples();
    if info.width != 256 || info.height != 256 || channels < 3 {
        return Err("unexpected terrarium tile".into());
    }
    Ok(data
        .chunks(channels)
        .take(256 * 256)
        .map(|p| (p[0] as f32 * 256.0 + p[1] as f32 + p[2] as f32 / 256.0) - 32768.0)
        .collect())
}

fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Depth/height colour ramp (GEBCO-like ocean blues, natural land tints).
fn ramp(e: f32) -> [f32; 3] {
    let ocean: [(f32, [f32; 3]); 7] = [
        (-11000.0, [8.0, 14.0, 48.0]),
        (-6000.0, [14.0, 34.0, 88.0]),
        (-4000.0, [22.0, 60.0, 126.0]),
        (-2000.0, [36.0, 96.0, 162.0]),
        (-200.0, [64.0, 150.0, 198.0]),
        (-30.0, [112.0, 196.0, 214.0]),
        (0.0, [150.0, 214.0, 212.0]),
    ];
    let land: [(f32, [f32; 3]); 6] = [
        (0.0, [188.0, 186.0, 140.0]),
        (100.0, [92.0, 134.0, 72.0]),
        (600.0, [130.0, 140.0, 82.0]),
        (1500.0, [150.0, 122.0, 86.0]),
        (3000.0, [168.0, 150.0, 132.0]),
        (5000.0, [246.0, 246.0, 250.0]),
    ];
    let stops: &[(f32, [f32; 3])] = if e < 0.0 { &ocean } else { &land };
    if e <= stops[0].0 {
        return stops[0].1;
    }
    for w in stops.windows(2) {
        if e <= w[1].0 {
            return lerp(w[0].1, w[1].1, (e - w[0].0) / (w[1].0 - w[0].0));
        }
    }
    stops[stops.len() - 1].1
}

/// Hillshaded, depth-tinted RGBA for a relief tile.
fn shade(elev: &[f32], z: u8, y: u32) -> Vec<u8> {
    let n = 2f64.powi(z as i32);
    let lat = tile_lat(y as f64 + 0.5, n);
    let pixel = (156_543.03 * lat.to_radians().cos() / n) as f32; // metres per pixel
    let exaggeration = (6.0 - z as f32 * 0.3).max(1.6);
    let (lx, ly, lz) = (-0.5f32, 0.6f32, 0.62f32);
    let at = |x: i32, y: i32| elev[(y.clamp(0, 255) * 256 + x.clamp(0, 255)) as usize];
    let mut out = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256 {
        for x in 0..256 {
            let e = at(x, y);
            let dx = (at(x + 1, y) - at(x - 1, y)) / (2.0 * pixel) * exaggeration;
            let dy = (at(x, y - 1) - at(x, y + 1)) / (2.0 * pixel) * exaggeration;
            let len = (dx * dx + dy * dy + 1.0).sqrt();
            let light = ((-dx * lx - dy * ly + lz) / len).clamp(0.0, 1.0);
            let shade = 0.45 + 0.75 * light;
            let c = ramp(e);
            let coast = if e > -2.0 && e < 2.0 { 0.85 } else { 1.0 };
            out.extend_from_slice(&[
                (c[0] * shade * coast).min(255.0) as u8,
                (c[1] * shade * coast).min(255.0) as u8,
                (c[2] * shade * coast).min(255.0) as u8,
                255,
            ]);
        }
    }
    out
}

fn tile_lat(y: f64, n: f64) -> f64 {
    (std::f64::consts::PI * (1.0 - 2.0 * y / n))
        .sinh()
        .atan()
        .to_degrees()
}
fn tile_lon(x: f64, n: f64) -> f64 {
    x / n * 360.0 - 180.0
}
fn unit(lat: f64, lon: f64) -> [f64; 3] {
    let (la, lo) = (lat.to_radians(), lon.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// Orbit camera aimed at a surface point (lat, lon) from `distance` km,
/// tilted `tilt` degrees from vertical and facing compass `heading` degrees.
/// `east`/`north` are the camera's screen right/up axes.
#[derive(Clone, Copy)]
struct View {
    eye: [f64; 3],
    east: [f64; 3],
    north: [f64; 3],
    forward: [f64; 3],
    focal: f64,
    center: Pos2,
    rect: Rect,
}
impl View {
    fn new(lat: f64, lon: f64, distance: f64, tilt: f64, heading: f64, rect: Rect) -> Self {
        let n = unit(lat, lon);
        let (la, lo) = (lat.to_radians(), lon.to_radians());
        let east0 = [-lo.sin(), lo.cos(), 0.0];
        let north0 = [-la.sin() * lo.cos(), -la.sin() * lo.sin(), la.cos()];
        let (sh, ch) = heading.to_radians().sin_cos();
        let (st, ct) = tilt.to_radians().sin_cos();
        // Horizontal facing direction and screen-right in the local frame.
        let ahead = add(scale(north0, ch), scale(east0, sh));
        let right = sub(scale(east0, ch), scale(north0, sh));
        let back = sub(scale(n, ct), scale(ahead, st));
        Self {
            eye: add(scale(n, R), scale(back, distance)),
            east: right,
            north: add(scale(n, st), scale(ahead, ct)),
            forward: scale(back, -1.0),
            focal: rect.height() as f64 / (2.0 * (FOV.to_radians() * 0.5).tan()),
            center: rect.center(),
            rect,
        }
    }
    /// Camera height above the sphere (km).
    fn height(&self) -> f64 {
        length(self.eye) - R
    }
    /// Screen position of a surface point, and whether it faces the camera.
    fn project(&self, p: [f64; 3]) -> Option<(Pos2, bool)> {
        let d = sub(p, self.eye);
        let depth = dot(d, self.forward);
        if depth < 1e-4 {
            return None;
        }
        let facing = dot(p, sub(self.eye, p)) > 0.0;
        Some((
            Pos2::new(
                self.center.x + (dot(d, self.east) / depth * self.focal) as f32,
                self.center.y - (dot(d, self.north) / depth * self.focal) as f32,
            ),
            facing,
        ))
    }
    /// Latitude/longitude under a screen point (ray–sphere intersection).
    fn pick(&self, p: Pos2) -> Option<(f64, f64)> {
        let x = (p.x - self.center.x) as f64 / self.focal;
        let y = -(p.y - self.center.y) as f64 / self.focal;
        let mut d = [
            self.forward[0] + self.east[0] * x + self.north[0] * y,
            self.forward[1] + self.east[1] * x + self.north[1] * y,
            self.forward[2] + self.east[2] * x + self.north[2] * y,
        ];
        let len = dot(d, d).sqrt();
        d = scale(d, 1.0 / len);
        let b = dot(self.eye, d);
        let c = dot(self.eye, self.eye) - R * R;
        let disc = b * b - c;
        if disc < 0.0 {
            return None;
        }
        let t = -b - disc.sqrt();
        if t <= 0.0 {
            // The sphere lies behind the camera along this ray (sky).
            return None;
        }
        let hit = [
            self.eye[0] + d[0] * t,
            self.eye[1] + d[1] * t,
            self.eye[2] + d[2] * t,
        ];
        Some((
            (hit[2] / R).clamp(-1.0, 1.0).asin().to_degrees(),
            hit[1].atan2(hit[0]).to_degrees(),
        ))
    }
}

/// UV rectangle of child tile (x, y) inside its parent's `uv`.
fn parent_uv(uv: Rect, x: u32, y: u32) -> Rect {
    let (qx, qy) = ((x & 1) as f32, (y & 1) as f32);
    Rect::from_min_max(
        Pos2::new((qx + uv.min.x) * 0.5, (qy + uv.min.y) * 0.5),
        Pos2::new((qx + uv.max.x) * 0.5, (qy + uv.max.y) * 0.5),
    )
}
/// Sub-rectangle of `uv` for tile (x, y) that lies `shift` levels below it.
fn descend_uv(uv: Rect, shift: u8, x: u32, y: u32) -> Rect {
    if shift == 0 {
        return uv;
    }
    let k = (1u32 << shift) as f32;
    let (ox, oy) = ((x % (1 << shift)) as f32, (y % (1 << shift)) as f32);
    let (w, h) = (uv.width() / k, uv.height() / k);
    Rect::from_min_size(
        Pos2::new(uv.min.x + ox * w, uv.min.y + oy * h),
        Vec2::new(w, h),
    )
}
/// Bilinear elevation at tile UV (0..1, pixel-centre aligned).
fn sample_elevation(e: &[f32], u: f32, v: f32) -> f32 {
    let sx = (u * 256.0 - 0.5).clamp(0.0, 255.0);
    let sy = (v * 256.0 - 0.5).clamp(0.0, 255.0);
    let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(255), (y0 + 1).min(255));
    let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
    let p = |xx: usize, yy: usize| e[yy * 256 + xx];
    (p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx) * (1.0 - ty)
        + (p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx) * ty
}
fn size_of(r: Rect) -> f32 {
    if r.is_positive() {
        r.width().max(r.height())
    } else {
        0.0
    }
}

struct Tile {
    texture: egui::TextureHandle,
    used: u64,
}

pub struct Globe {
    pub focus: bool,
    layer: Layer,
    lat: f64,
    lon: f64,
    altitude: f64,
    /// Camera tilt from vertical and compass heading (degrees).
    tilt: f64,
    heading: f64,
    /// Vertical exaggeration of the 3D relief (0 = flat).
    exaggeration: f64,
    loader: Option<Loader>,
    tiles: HashMap<Key, Tile>,
    elevations: HashMap<Key, Arc<Vec<f32>>>,
    pending: HashMap<Key, Instant>,
    failed: HashMap<Key, Instant>,
    frame: u64,
    goto: String,
    hover: Option<(f64, f64)>,
    last_error: Option<String>,
    building: Option<mpsc::Receiver<Result<Scene, String>>>,
    status: String,
    /// Live seas: global wave map and point forecasts.
    sea: sea_layer::SeaLayer,
    /// New ocean scenes start with the forecast sea of the current hour.
    live_sea: bool,
    /// Forecast sea chosen for the current scene (taken by the editor).
    pub apply_sea: Option<(Conditions, String)>,
    /// Search-and-rescue drift shown on the globe (set by the editor).
    pub sar: Option<Arc<SarOverlay>>,
    /// "Search and rescue here" was chosen (taken by the editor).
    pub sar_here: Option<(f64, f64)>,
    /// Oil spill drift shown on the globe, and "Oil spill here".
    pub spill: Option<Arc<SarOverlay>>,
    pub spill_here: Option<(f64, f64)>,
    /// Habitat overlays and undersea feature names.
    reefs: bool,
    seagrass: bool,
    features_on: bool,
    features: Option<Arc<Vec<seafloor::Feature>>>,
    features_loading: Option<mpsc::Receiver<Result<Vec<seafloor::Feature>, String>>>,
    islands: bool,
    boundaries: bool,
    seamarks: bool,
    /// Daily sea-surface temperature overlay.
    pub(super) sst: bool,
    pub(super) marine: marine::Marine,
    /// Map point under the last right-click (the context menu acts on it).
    context_point: Option<(f64, f64)>,
    /// Place card for the last clicked point.
    place: Option<place::PlaceCard>,
}

/// A search-and-rescue drift drawn on the globe: possible positions with
/// the share of probability still unsearched, planned track and area.
#[derive(Clone, Debug, Default)]
pub struct SarOverlay {
    pub lkp: (f64, f64),
    pub particles: Vec<(f64, f64, f32)>,
    pub track: Vec<(f64, f64)>,
    pub area: Vec<(f64, f64)>,
    /// Particle colour (alpha follows each particle's weight).
    pub colour: (u8, u8, u8),
}

impl Default for Globe {
    fn default() -> Self {
        Self {
            focus: false,
            layer: Layer::Hybrid,
            lat: 12.0,
            lon: 68.0,
            altitude: 14_000.0,
            tilt: 0.0,
            heading: 0.0,
            exaggeration: 2.0,
            loader: None,
            tiles: HashMap::new(),
            elevations: HashMap::new(),
            pending: HashMap::new(),
            failed: HashMap::new(),
            frame: 0,
            goto: String::new(),
            hover: None,
            last_error: None,
            building: None,
            status: String::new(),
            sea: Default::default(),
            live_sea: true,
            apply_sea: None,
            sar: None,
            sar_here: None,
            spill: None,
            spill_here: None,
            reefs: false,
            seagrass: false,
            features_on: false,
            features: None,
            features_loading: None,
            islands: false,
            boundaries: false,
            seamarks: false,
            sst: false,
            marine: Default::default(),
            context_point: None,
            place: None,
        }
    }
}

const PLACES: [(&str, f64, f64, f64); 7] = [
    ("Arabian Sea · Goa shelf", 15.2, 73.2, 400.0),
    ("Mariana Trench", 11.35, 142.2, 900.0),
    ("Great Barrier Reef", -18.3, 147.7, 900.0),
    ("Monterey Canyon", 36.75, -122.05, 120.0),
    ("Mid-Atlantic Ridge", 30.0, -42.0, 2500.0),
    ("Hawaiian Islands", 20.6, -157.4, 900.0),
    ("Strait of Gibraltar", 35.95, -5.6, 150.0),
];

impl Globe {
    /// Scripted view (captures/tests): latitude, longitude, altitude in km.
    pub fn set_view(&mut self, lat: f64, lon: f64, altitude: f64) {
        self.lat = lat.clamp(-84.0, 84.0);
        self.lon = lon;
        self.altitude = altitude.clamp(0.4, 30_000.0);
    }
    /// Scripted camera orientation: tilt from vertical and heading, degrees.
    pub fn set_orientation(&mut self, tilt: f64, heading: f64) {
        self.heading = heading.rem_euclid(360.0);
        self.tilt = tilt.clamp(0.0, self.max_tilt());
    }
    /// Tilt is allowed close to the surface and phased out from orbit.
    fn max_tilt(&self) -> f64 {
        78.0 * ((9_000.0 - self.altitude) / 7_000.0).clamp(0.0, 1.0)
    }
    pub fn set_satellite(&mut self, on: bool) {
        self.layer = if on { Layer::Satellite } else { Layer::Relief };
    }
    /// Latitude and longitude at the centre of the view.
    pub fn center(&self) -> (f64, f64) {
        (self.lat, (self.lon + 540.0).rem_euclid(360.0) - 180.0)
    }
    pub fn create_here(&mut self) {
        self.start_build(self.lat, self.lon);
    }
    /// Shows or hides the live global wave map.
    pub fn set_live_waves(&mut self, on: bool) {
        self.sea.on = on;
    }
    /// Loads the hourly sea forecast at a point (opens its panel).
    pub fn request_forecast(&mut self, lat: f64, lon: f64) {
        self.sea.request_point(lat, lon);
    }
    /// A real-world scene is being built (scripted captures wait for it).
    pub fn building(&self) -> bool {
        self.building.is_some()
    }
    /// Live-sea data is downloading (scripted captures wait for it).
    pub fn sea_loading(&self) -> bool {
        self.sea.downloading()
    }
    /// The last point forecast loaded on the Earth view.
    pub fn forecast(&self) -> Option<Arc<Forecast>> {
        self.sea.forecast.clone()
    }
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || self.building.is_some() || self.sea.busy()
    }

    pub fn poll(&mut self) {
        // Tile uploads happen in `ui`, which has the egui context. Forecast
        // downloads finish here every frame, even with the Earth tab hidden.
        self.sea.poll();
    }

    fn zoom_level(&self, rect: Rect) -> f64 {
        // Approximate web-mercator zoom whose pixels match the screen.
        let metres_per_px = self.altitude * 1000.0 * (FOV.to_radians() * 0.5).tan() * 2.0
            / rect.height().max(1.0) as f64;
        (156_543.03 * self.lat.to_radians().cos().max(0.05) / metres_per_px).log2()
    }

    fn texture(&mut self, ctx: &egui::Context, key: Key) -> Option<(egui::TextureId, Rect)> {
        // Best loaded texture for `key`, walking up to ancestors with sub-UVs.
        let (layer, mut z, mut x, mut y) = key;
        let mut uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        // Request the tile itself and, if nothing within three levels above
        // it is loaded yet, a coarse ancestor as a quick placeholder (the
        // queue is newest-first, so the placeholder arrives first).
        let coarse = key.1.saturating_sub(3);
        loop {
            let k = (layer, z, x, y);
            if let Some(t) = self.tiles.get_mut(&k) {
                t.used = self.frame;
                return Some((t.texture.id(), uv));
            }
            if (z == key.1 || z == coarse) && z <= layer.max_zoom() {
                let retry = self
                    .failed
                    .get(&k)
                    .is_none_or(|t| t.elapsed().as_secs() > 30);
                if retry && !self.pending.contains_key(&k) && self.pending.len() < 160 {
                    let dropped = self.loader.get_or_insert_with(Loader::new).request(k);
                    for d in dropped {
                        self.pending.remove(&d);
                    }
                    self.pending.insert(k, Instant::now());
                    ctx.request_repaint();
                }
            }
            if z == 0 {
                return None;
            }
            uv = parent_uv(uv, x, y);
            z -= 1;
            x >>= 1;
            y >>= 1;
        }
    }

    fn receive(&mut self, ctx: &egui::Context) {
        let Some(loader) = &self.loader else {
            return;
        };
        let mut uploaded = 0;
        while uploaded < 24 {
            let Ok(result) = loader.results.try_recv() else {
                break;
            };
            self.pending.remove(&result.key);
            match result.rgba {
                Some((rgba, elev)) => {
                    let texture = ctx.load_texture(
                        format!("tile-{:?}", result.key),
                        egui::ColorImage::from_rgba_unmultiplied([256, 256], &rgba),
                        egui::TextureOptions::LINEAR,
                    );
                    self.tiles.insert(
                        result.key,
                        Tile {
                            texture,
                            used: self.frame,
                        },
                    );
                    if let Some(e) = elev {
                        // Hybrid tiles carry the same relief elevations.
                        let (_, z, x, y) = result.key;
                        self.elevations
                            .insert((Layer::Relief, z, x, y), Arc::new(e));
                    }
                    self.failed.remove(&result.key);
                    uploaded += 1;
                }
                None => {
                    self.last_error = result.error;
                    self.failed.insert(result.key, Instant::now());
                }
            }
        }
        // Expire stale pending markers (dropped from the bounded queue).
        self.pending.retain(|_, t| t.elapsed().as_secs() < 20);
        // Bounded caches: evict least recently used.
        if self.tiles.len() > 560 {
            let mut used: Vec<_> = self.tiles.iter().map(|(k, t)| (t.used, *k)).collect();
            used.sort();
            for (_, k) in used.into_iter().take(self.tiles.len() - 480) {
                if k.1 > 2 {
                    self.tiles.remove(&k);
                }
            }
        }
        if self.elevations.len() > 96 {
            let keep: Vec<Key> = self
                .elevations
                .keys()
                .filter(|k| self.tiles.contains_key(k))
                .copied()
                .collect();
            self.elevations.retain(|k, _| keep.contains(k));
            while self.elevations.len() > 96 {
                let k = *self.elevations.keys().next().unwrap();
                self.elevations.remove(&k);
            }
        }
        if uploaded > 0 {
            ctx.request_repaint();
        }
    }

    /// Elevation (m) at a point from the deepest loaded relief tile.
    fn elevation(&self, lat: f64, lon: f64) -> Option<(f32, u8)> {
        let mut best: Option<(f32, u8)> = None;
        for z in (0..=15u8).rev() {
            let n = 2f64.powi(z as i32);
            let fx = (lon + 180.0) / 360.0 * n;
            let la = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
            let fy = (1.0 - (la.tan() + 1.0 / la.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
            let (x, y) = (fx.floor() as u32, fy.floor() as u32);
            if let Some(e) = self.elevations.get(&(Layer::Relief, z, x, y)) {
                let px = ((fx - x as f64) * 256.0).clamp(0.0, 255.0) as usize;
                let py = ((fy - y as f64) * 256.0).clamp(0.0, 255.0) as usize;
                best = Some((e[py * 256 + px], z));
                break;
            }
        }
        best
    }

    /// Relief elevations covering `key`: the tile itself or its nearest
    /// loaded ancestor, with the UV sub-rectangle for `key`.
    fn elevation_grid(&self, key: Key) -> Option<(Arc<Vec<f32>>, Rect)> {
        let (_, mut z, mut x, mut y) = key;
        let mut uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        loop {
            if let Some(e) = self.elevations.get(&(Layer::Relief, z, x, y)) {
                return Some((e.clone(), uv));
            }
            if z == 0 {
                return None;
            }
            uv = parent_uv(uv, x, y);
            z -= 1;
            x >>= 1;
            y >>= 1;
        }
    }

    fn draw_tiles(&mut self, ui: &egui::Ui, view: &View, painter: &egui::Painter) -> usize {
        // Metres per screen pixel per km of viewing distance.
        let mpp_per_km =
            1000.0 * (FOV.to_radians() * 0.5).tan() * 2.0 / view.rect.height().max(1.0) as f64;
        let zoom_cap = self.layer.max_zoom() as f64 + 3.0;
        let mut stack = vec![(0u8, 0u32, 0u32)];
        let mut meshes: Vec<(f64, egui::Mesh)> = vec![];
        let mut visited = 0usize;
        // Geographic footprint of the view (3x3 screen picks) in mercator
        // units with viewing distance, so tiles that contain the view are
        // kept and refined even when none of their sparse sample points land
        // on screen (deep zoom).
        let footprint: Vec<(f64, f64, f64)> = (0..3)
            .flat_map(|j| (0..3).map(move |i| (i, j)))
            .filter_map(|(i, j)| {
                let p = Pos2::new(
                    view.rect.left() + view.rect.width() * (0.02 + 0.48 * i as f32),
                    view.rect.top() + view.rect.height() * (0.02 + 0.48 * j as f32),
                );
                view.pick(p)
            })
            .map(|(lat, lon)| {
                let d = length(sub(scale(unit(lat, lon), R), view.eye));
                let la = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
                (
                    (lon + 180.0) / 360.0,
                    (1.0 - (la.tan() + 1.0 / la.cos()).ln() / std::f64::consts::PI) / 2.0,
                    d,
                )
            })
            .collect();
        let ctx = ui.ctx().clone();
        let exaggeration = self.exaggeration / 1000.0; // m -> km
        while let Some((z, x, y)) = stack.pop() {
            visited += 1;
            let n = 2f64.powi(z as i32);
            let res = if z < 3 { 16 } else { 8 };
            let mut points = Vec::with_capacity((res + 1) * (res + 1));
            let mut units = Vec::with_capacity((res + 1) * (res + 1));
            let mut any_facing = false;
            let mut bounds = Rect::NOTHING;
            let mut near = f64::MAX;
            for j in 0..=res {
                let lat = tile_lat(y as f64 + j as f64 / res as f64, n);
                for i in 0..=res {
                    let lon = tile_lon(x as f64 + i as f64 / res as f64, n);
                    let world = scale(unit(lat, lon), R);
                    units.push(unit(lat, lon));
                    let p = view.project(world);
                    if let Some((s, facing)) = p {
                        any_facing |= facing;
                        if facing {
                            bounds = bounds.union(Rect::from_min_max(s, s));
                            near = near.min(length(sub(world, view.eye)));
                        }
                    }
                    points.push(p);
                }
            }
            let mut contains_view = false;
            for (fx, fy, d) in &footprint {
                let (tx, ty) = (fx * n, fy * n);
                if tx >= x as f64 && tx < x as f64 + 1.0 && ty >= y as f64 && ty < y as f64 + 1.0 {
                    contains_view = true;
                    near = near.min(*d);
                }
            }
            if !contains_view && (!any_facing || !bounds.intersects(view.rect.expand(4.0))) {
                continue;
            }
            // Zoom whose texels match screen pixels at this tile's nearest
            // visible point, so tilted views use coarse tiles far away.
            let mid_lat = tile_lat(y as f64 + 0.5, n).to_radians().cos().max(0.05);
            let target = (156_543.03 * mid_lat / (near.max(0.05) * mpp_per_km))
                .log2()
                .clamp(0.0, zoom_cap);
            let max_zoom = self.layer.max_zoom().max(15);
            if z < max_zoom
                && z < 19
                && visited < 2400
                && (size_of(bounds) > 320.0 || (z as f64) < target.floor() || z < 2)
            {
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    stack.push((z + 1, x * 2 + dx, y * 2 + dy));
                }
                continue;
            }
            let request_z = z.min(self.layer.max_zoom());
            let key = (
                self.layer,
                request_z,
                x >> (z - request_z),
                y >> (z - request_z),
            );
            let Some((texture, uv)) = self.texture(&ctx, key) else {
                continue;
            };
            let uv = descend_uv(uv, z - request_z, x, y);
            // 3D relief: displace along the surface normal by the (exaggerated)
            // elevation, from the best loaded relief data covering this tile.
            let relief = if exaggeration > 0.0 {
                let ez = z.min(15);
                self.elevation_grid((Layer::Relief, ez, x >> (z - ez), y >> (z - ez)))
                    .map(|(e, r)| (e, descend_uv(r, z - ez, x, y)))
            } else {
                None
            };
            let mut radii = vec![R; points.len()];
            if let Some((e, r)) = &relief {
                for j in 0..=res {
                    for i in 0..=res {
                        let k = j * (res + 1) + i;
                        let h = sample_elevation(
                            e,
                            r.min.x + r.width() * i as f32 / res as f32,
                            r.min.y + r.height() * j as f32 / res as f32,
                        ) as f64;
                        radii[k] = R + h * exaggeration;
                        points[k] = view.project(scale(units[k], radii[k]));
                    }
                }
            }
            let mut mesh = egui::Mesh::with_texture(texture);
            let mut index = vec![u32::MAX; points.len()];
            for j in 0..=res {
                for i in 0..=res {
                    let k = j * (res + 1) + i;
                    if let Some((s, _)) = points[k] {
                        index[k] = mesh.vertices.len() as u32;
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: s,
                            uv: Pos2::new(
                                uv.min.x + uv.width() * i as f32 / res as f32,
                                uv.min.y + uv.height() * j as f32 / res as f32,
                            ),
                            color: Color32::WHITE,
                        });
                    }
                }
            }
            for j in 0..res {
                for i in 0..res {
                    let a = j * (res + 1) + i;
                    let quad = [a, a + 1, a + res + 1, a + res + 2];
                    if quad
                        .iter()
                        .any(|k| !points[*k].is_some_and(|p| p.1) || index[*k] == u32::MAX)
                    {
                        continue;
                    }
                    mesh.indices
                        .extend([index[quad[0]], index[quad[1]], index[quad[2]]]);
                    mesh.indices
                        .extend([index[quad[1]], index[quad[3]], index[quad[2]]]);
                }
            }
            // Skirts: short walls hanging below each tile edge hide the
            // hairline cracks between neighbours at different detail levels.
            // They are drawn before the surface, which then covers any part
            // that projects onto the tile itself.
            let mut skirt = vec![];
            let drop = (40_075.0 / n * 0.015).max(0.25);
            let edges: [Vec<usize>; 4] = [
                (0..=res).collect(),
                (0..=res).map(|i| res * (res + 1) + i).collect(),
                (0..=res).map(|j| j * (res + 1)).collect(),
                (0..=res).map(|j| j * (res + 1) + res).collect(),
            ];
            for edge in &edges {
                for pair in edge.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    if index[a] == u32::MAX
                        || index[b] == u32::MAX
                        || !points[a].is_some_and(|p| p.1)
                        || !points[b].is_some_and(|p| p.1)
                    {
                        continue;
                    }
                    let (Some((sa, _)), Some((sb, _))) = (
                        view.project(scale(units[a], radii[a] - drop)),
                        view.project(scale(units[b], radii[b] - drop)),
                    ) else {
                        continue;
                    };
                    let base = mesh.vertices.len() as u32;
                    for (pos, k) in [(sa, a), (sb, b)] {
                        let uv = mesh.vertices[index[k] as usize].uv;
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos,
                            uv,
                            color: Color32::WHITE,
                        });
                    }
                    skirt.extend([index[a], index[b], base + 1]);
                    skirt.extend([index[a], base + 1, base]);
                }
            }
            skirt.append(&mut mesh.indices);
            mesh.indices = skirt;
            if !mesh.indices.is_empty() {
                // Habitat overlays share the tile's geometry, drawn after it.
                let mut overlays = vec![];
                for (on, overlay) in [
                    (self.islands, Layer::Islands),
                    (self.reefs, Layer::Reefs),
                    (self.seagrass, Layer::Seagrass),
                    (self.boundaries, Layer::Boundaries),
                    (self.seamarks && z >= 9, Layer::Seamarks),
                    (self.sst, Layer::Sst),
                ] {
                    if !on {
                        continue;
                    }
                    let oz = z.min(overlay.max_zoom());
                    let key = (overlay, oz, x >> (z - oz), y >> (z - oz));
                    if let Some((texture, found)) = self.texture(&ctx, key) {
                        let ouv = descend_uv(found, z - oz, x, y);
                        let mut m = mesh.clone();
                        m.texture_id = texture;
                        for v in &mut m.vertices {
                            let fx = (v.uv.x - uv.min.x) / uv.width().max(1e-9);
                            let fy = (v.uv.y - uv.min.y) / uv.height().max(1e-9);
                            v.uv = Pos2::new(
                                ouv.min.x + ouv.width() * fx,
                                ouv.min.y + ouv.height() * fy,
                            );
                        }
                        overlays.push(m);
                    }
                }
                meshes.push((near, mesh));
                for m in overlays {
                    meshes.push((near, m));
                }
            }
        }
        // Painter's order: far tiles first so near relief overlaps them.
        meshes.sort_by(|a, b| b.0.total_cmp(&a.0));
        let drawn = meshes.len();
        for (_, mesh) in meshes {
            painter.add(egui::Shape::mesh(mesh));
        }
        drawn
    }

    /// Per screen column, the vertical extent covered by the globe (a convex
    /// body, so one interval per column): `(x, top, bottom)`.
    fn silhouette(view: &View) -> Vec<Option<(f32, f32, f32)>> {
        const COLUMNS: usize = 72;
        const ROWS: usize = 40;
        let r = view.rect;
        (0..=COLUMNS)
            .map(|c| {
                let x = r.left() + r.width() * c as f32 / COLUMNS as f32;
                let at = |y: f32| view.pick(Pos2::new(x, y)).is_some();
                let ys: Vec<f32> = (0..=ROWS)
                    .map(|i| r.top() + r.height() * i as f32 / ROWS as f32)
                    .collect();
                let hits: Vec<bool> = ys.iter().map(|y| at(*y)).collect();
                let first = hits.iter().position(|h| *h)?;
                let last = hits.iter().rposition(|h| *h)?;
                let edge = |mut miss: f32, mut hit: f32| {
                    for _ in 0..12 {
                        let mid = (miss + hit) * 0.5;
                        if at(mid) {
                            hit = mid;
                        } else {
                            miss = mid;
                        }
                    }
                    hit
                };
                let top = if first == 0 {
                    ys[0]
                } else {
                    edge(ys[first - 1], ys[first])
                };
                let bottom = if last == ROWS {
                    ys[ROWS]
                } else {
                    edge(ys[last + 1], ys[last])
                };
                Some((x, top, bottom))
            })
            .collect()
    }

    /// Strips between adjacent silhouette columns, from `ends(top, bottom)`
    /// = ((y0, colour0), (y1, colour1)) in each column.
    fn column_strips(
        columns: &[Option<(f32, f32, f32)>],
        painter: &egui::Painter,
        ends: impl Fn(f32, f32) -> ((f32, Color32), (f32, Color32)),
    ) {
        let mut mesh = egui::Mesh::default();
        for pair in columns.windows(2) {
            let (Some(a), Some(b)) = (pair[0], pair[1]) else {
                continue;
            };
            let base = mesh.vertices.len() as u32;
            for (x, top, bottom) in [a, b] {
                let ((y0, c0), (y1, c1)) = ends(top, bottom);
                mesh.colored_vertex(Pos2::new(x, y0), c0);
                mesh.colored_vertex(Pos2::new(x, y1), c1);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    fn polar_caps(&self, view: &View, painter: &egui::Painter) {
        let color = if self.layer == Layer::Relief {
            Color32::from_rgb(214, 226, 236)
        } else {
            Color32::from_rgb(220, 228, 236)
        };
        for pole in [1.0f64, -1.0] {
            let mut mesh = egui::Mesh::default();
            let c = view.project(scale([0.0, 0.0, pole], R));
            let Some((center, true)) = c else {
                continue;
            };
            mesh.colored_vertex(center, color);
            let mut ok = true;
            for i in 0..=64 {
                let lon = i as f64 / 64.0 * 360.0 - 180.0;
                match view.project(scale(unit(MAX_LAT * pole, lon), R)) {
                    Some((p, _)) => mesh.colored_vertex(p, color),
                    None => ok = false,
                }
            }
            if !ok {
                continue;
            }
            for i in 1..=64u32 {
                mesh.add_triangle(0, i, i + 1);
            }
            painter.add(egui::Shape::mesh(mesh));
        }
    }

    fn atmosphere(&self, view: &View, painter: &egui::Painter) {
        let d = R + self.altitude;
        let angle = (R / d).asin();
        let radius = (view.focal * angle.tan()) as f32;
        if radius > view.rect.width().max(view.rect.height()) * 1.5 {
            return;
        }
        let mut mesh = egui::Mesh::default();
        let inner = Color32::from_rgba_unmultiplied(90, 160, 255, 150);
        let outer = Color32::from_rgba_unmultiplied(40, 90, 200, 0);
        let width = radius * 0.06 + 6.0;
        for i in 0..=96 {
            let a = i as f32 / 96.0 * std::f32::consts::TAU;
            let dir = Vec2::new(a.cos(), a.sin());
            mesh.colored_vertex(view.center + dir * (radius - 2.0), inner);
            mesh.colored_vertex(view.center + dir * (radius + width), outer);
        }
        for i in 0..96u32 {
            let k = i * 2;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 3, k + 2);
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    fn stars(&self, rect: Rect, painter: &egui::Painter) {
        let mut seed = 0x9e37_79b9u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        // Stars drift slightly with longitude for a sense of rotation.
        let drift = (self.lon / 360.0) as f32;
        for _ in 0..260 {
            let x = (next() + drift).rem_euclid(1.0);
            let y = next();
            let b = 60.0 + next() * 150.0;
            painter.circle_filled(
                Pos2::new(
                    rect.left() + x * rect.width(),
                    rect.top() + y * rect.height(),
                ),
                next() * 1.1 + 0.3,
                Color32::from_gray(b as u8),
            );
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Scene> {
        self.frame += 1;
        let ctx = ui.ctx().clone();
        self.receive(&ctx);
        let mut created = None;
        if let Some(rx) = &self.building
            && let Ok(result) = rx.try_recv()
        {
            self.building = None;
            match result {
                Ok(scene) => {
                    self.status = format!("Created \"{}\".", scene.name);
                    created = Some(scene);
                }
                Err(e) => self.status = format!("Could not build a scene here: {e}"),
            }
        }
        // Toolbar. Wraps onto a second line on narrow windows so no control
        // is ever pushed off-screen or covered.
        egui::Frame::new()
            .fill(theme::HEADER)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let building = self.building.is_some();
                    let create = ui
                        .add_enabled(
                            !building,
                            egui::Button::new(
                                RichText::new("🌊 Create ocean scene here").strong(),
                            )
                            .fill(theme::ACCENT.gamma_multiply(0.55)),
                        )
                        .on_hover_text("Build a Pelagic scene from the real seabed, coast, reefs and sun at the map centre (≈5 km square, 82 km backdrop). Or right-click any point.");
                    super::a11y::name(&create, "Create ocean scene here");
                    if create.clicked() {
                        self.start_build(self.lat, self.lon);
                    }
                    if building {
                        ui.spinner();
                        ui.label(RichText::new("building…").small().color(theme::MUTED));
                    }
                    ui.separator();
                    let layer = egui::ComboBox::from_id_salt("layer")
                        .selected_text(self.layer.name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.layer, Layer::Relief, Layer::Relief.name());
                            ui.selectable_value(&mut self.layer, Layer::Satellite, Layer::Satellite.name());
                            ui.selectable_value(&mut self.layer, Layer::Sentinel, Layer::Sentinel.name());
                            ui.selectable_value(&mut self.layer, Layer::Hybrid, Layer::Hybrid.name());
                            ui.selectable_value(&mut self.layer, Layer::Today, Layer::Today.name());
                        });
                    super::a11y::name(&layer.response, "Map layer");
                    let layers = ui.menu_button("☰ Layers", |ui| {
                        ui.set_min_width(330.0);
                        self.layers_menu(ui);
                    });
                    super::a11y::name(&layers.response, "Map layers");
                    ui.menu_button("📍 Places", |ui| {
                        for (name, lat, lon, alt) in PLACES {
                            if ui.button(name).clicked() {
                                self.lat = lat;
                                self.lon = lon;
                                self.altitude = alt;
                                ui.close();
                            }
                        }
                    });
                    let go = ui.add(
                        egui::TextEdit::singleline(&mut self.goto)
                            .hint_text("lat, lon or a seamount, trench…")
                            .desired_width(190.0),
                    );
                    super::a11y::name(&go, "Go to latitude, longitude or undersea feature");
                    if go.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        let parts: Vec<f64> = self
                            .goto
                            .split([',', ' '])
                            .filter_map(|s| s.trim().parse().ok())
                            .collect();
                        if parts.len() == 2 && parts[0].abs() <= 90.0 && parts[1].abs() <= 180.0 {
                            self.lat = parts[0];
                            self.lon = parts[1];
                            self.altitude = self.altitude.min(300.0);
                        } else if !self.goto.trim().is_empty() {
                            self.find_feature();
                        } else {
                            self.status =
                                "Enter latitude, longitude or an undersea feature name.".into();
                        }
                    }
                    ui.separator();
                    let tilted = self.tilt > 1.0;
                    if ui
                        .selectable_label(tilted, "⛰ 3D tilt")
                        .on_hover_text("Tilt toward the horizon (Shift+drag or middle-drag to tilt and rotate)")
                        .clicked()
                    {
                        if tilted {
                            self.tilt = 0.0;
                            self.heading = 0.0;
                        } else {
                            self.altitude = self.altitude.min(1_500.0);
                            self.tilt = 60.0f64.min(self.max_tilt());
                        }
                    }
                    ui.add(
                        egui::DragValue::new(&mut self.exaggeration)
                            .range(0.0..=10.0)
                            .speed(0.05)
                            .fixed_decimals(1)
                            .prefix("relief ×"),
                    )
                    .on_hover_text("Vertical exaggeration of the 3D seabed and terrain (0 = flat)");
                });
                self.sea_toolbar(ui);
            });
        let mut rect = ui.available_rect_before_wrap();
        if self.sea.forecast.is_some() || self.sea.panel_wanted() {
            let width = 340.0_f32.min(rect.width() * 0.45);
            let panel =
                egui::Rect::from_min_max(egui::pos2(rect.right() - width, rect.top()), rect.max);
            rect.max.x -= width;
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(panel)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.painter().rect_filled(panel, 0.0, theme::PANEL);
            egui::Frame::new()
                .inner_margin(egui::Margin::same(10))
                .show(&mut child, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("sea-forecast-panel")
                        .show(ui, |ui| self.forecast_panel(ui));
                });
        }
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        super::a11y::name(&response, "Earth globe");
        let painter = ui.painter_at(rect);
        self.tilt = self.tilt.min(self.max_tilt());
        let view = View::new(
            self.lat,
            self.lon,
            self.altitude,
            self.tilt,
            self.heading,
            rect,
        );
        // Space fades to a daytime sky as the camera descends into the
        // atmosphere (visible once tilted toward the horizon).
        let sky = (1.0 - view.height() / 250.0).clamp(0.0, 1.0) as f32;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(2, 3, 8));
        if sky < 1.0 {
            self.stars(rect, &painter);
        }
        if sky > 0.0 {
            let a = (sky * 255.0) as u8;
            let high = Color32::from_rgba_unmultiplied(22, 52, 112, a);
            let low = Color32::from_rgba_unmultiplied(120, 165, 220, a);
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(rect.left_top(), high);
            mesh.colored_vertex(rect.right_top(), high);
            mesh.colored_vertex(rect.left_bottom(), low);
            mesh.colored_vertex(rect.right_bottom(), low);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(1, 3, 2);
            painter.add(egui::Shape::mesh(mesh));
        }
        let columns = Self::silhouette(&view);
        if self.tilt < 0.5 {
            self.atmosphere(&view, &painter);
        } else {
            // Limb glow above the horizon.
            let glow = rect.height() * (0.03 + 0.22 * sky);
            Self::column_strips(&columns, &painter, |top, _| {
                (
                    (
                        top + 1.0,
                        Color32::from_rgba_unmultiplied(150, 195, 245, 200),
                    ),
                    (top - glow, Color32::from_rgba_unmultiplied(60, 110, 210, 0)),
                )
            });
        }
        // Base ocean so the globe never shows the sky while tiles stream.
        let ocean = Color32::from_rgb(18, 46, 98);
        Self::column_strips(&columns, &painter, |top, bottom| {
            ((top, ocean), (bottom, ocean))
        });
        let drawn = self.draw_tiles(ui, &view, &painter);
        self.polar_caps(&view, &painter);
        self.draw_sea(&view, &painter);
        self.draw_sar(&view, &painter);
        self.draw_features(&view, &painter);
        self.update_marine(&view);
        self.draw_marine(&view, &painter);
        if sky > 0.0 && self.tilt > 0.5 {
            // Aerial perspective: distant terrain fades into horizon haze.
            let haze = rect.height() * 0.22;
            let a = (sky * 220.0) as u8;
            Self::column_strips(&columns, &painter, |top, _| {
                (
                    (top, Color32::from_rgba_unmultiplied(150, 190, 232, a)),
                    (
                        top + haze,
                        Color32::from_rgba_unmultiplied(150, 190, 232, 0),
                    ),
                )
            });
        }
        // Compass: needle points north; click to reset heading and tilt.
        let compass =
            Rect::from_center_size(rect.right_top() + Vec2::new(-30.0, 52.0), Vec2::splat(40.0));
        let compass_response =
            ui.interact(compass, ui.scope_id().with("globe-compass"), Sense::click());
        painter.circle(
            compass.center(),
            18.0,
            Color32::from_black_alpha(150),
            egui::Stroke::new(1.0, Color32::from_white_alpha(90)),
        );
        let (sh, ch) = (self.heading.to_radians() as f32).sin_cos();
        let needle = Vec2::new(-sh, -ch) * 13.0;
        painter.line_segment(
            [compass.center() - needle * 0.6, compass.center() + needle],
            egui::Stroke::new(2.5, Color32::from_rgb(230, 70, 60)),
        );
        painter.text(
            compass.center() + needle * 1.25,
            egui::Align2::CENTER_CENTER,
            "N",
            egui::FontId::proportional(10.0),
            Color32::WHITE,
        );
        if compass_response
            .on_hover_text("Reset to north-up, looking straight down")
            .clicked()
        {
            self.heading = 0.0;
            self.tilt = 0.0;
        }
        // Interaction: drag to pan, wheel to zoom about the cursor, double-click zooms in.
        let modifier = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl);
        if response.dragged_by(egui::PointerButton::Middle)
            || (modifier && response.dragged_by(egui::PointerButton::Primary))
        {
            // Orbit about the view centre: horizontal = heading, vertical = tilt.
            let d = response.drag_delta();
            self.heading = (self.heading - d.x as f64 * 0.3).rem_euclid(360.0);
            self.tilt = (self.tilt + d.y as f64 * 0.25).clamp(0.0, self.max_tilt());
        } else if response.dragged_by(egui::PointerButton::Primary)
            || response.dragged_by(egui::PointerButton::Secondary)
        {
            // Pan in screen directions, rotated by the heading.
            let d = response.drag_delta();
            let k = ((self.altitude.min(2.5 * R) / R) / view.focal).to_degrees();
            let (sh, ch) = self.heading.to_radians().sin_cos();
            let (dx, dy) = (d.x as f64, d.y as f64);
            let north = dy * ch + dx * sh;
            let east = -dx * ch + dy * sh;
            self.lat = (self.lat + north * k).clamp(-84.0, 84.0);
            self.lon += east * k / self.lat.to_radians().cos().max(0.1);
            self.lon = (self.lon + 540.0).rem_euclid(360.0) - 180.0;
        }
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if response.hovered() && scroll != 0.0 {
            let before = response.hover_pos().and_then(|p| view.pick(p));
            self.altitude =
                (self.altitude * 0.86f64.powf(scroll as f64 / 50.0)).clamp(0.4, 30_000.0);
            // Keep the point under the cursor fixed.
            if let (Some((la, lo)), Some(p)) = (before, response.hover_pos()) {
                let after = View::new(
                    self.lat,
                    self.lon,
                    self.altitude,
                    self.tilt,
                    self.heading,
                    rect,
                )
                .pick(p);
                if let Some((la2, lo2)) = after {
                    self.lat = (self.lat + la - la2).clamp(-84.0, 84.0);
                    self.lon += lo - lo2;
                }
            }
        }
        if response.double_clicked()
            && let Some((la, lo)) = response.interact_pointer_pos().and_then(|p| view.pick(p))
        {
            self.lat = la.clamp(-84.0, 84.0);
            self.lon = lo;
            self.altitude = (self.altitude * 0.35).max(0.4);
        }
        self.hover = response.hover_pos().and_then(|p| view.pick(p));
        // The context menu acts on the point that was right-clicked, not on
        // the pointer position (which leaves the globe as it moves onto the
        // menu, which used to empty the menu before an item could be chosen).
        if response.secondary_clicked() {
            self.context_point = response.interact_pointer_pos().and_then(|p| view.pick(p));
        }
        if response.clicked()
            && let Some(p) = response.interact_pointer_pos()
            && !self.pick_marine(&view, p)
            && let Some((la, lo)) = view.pick(p)
        {
            self.open_place(la, (lo + 540.0).rem_euclid(360.0) - 180.0);
        }
        response.context_menu(|ui| {
            if let Some((la, lo)) = self.context_point {
                if ui
                    .button(format!("🌊 Create ocean scene at {la:.3}°, {lo:.3}°"))
                    .clicked()
                {
                    self.start_build(la, lo);
                    ui.close();
                }
                if ui.button("Sea forecast here").clicked() {
                    self.sea.request_point(la, lo);
                    ui.close();
                }
                if ui.button("Search and rescue here").clicked() {
                    self.sar_here = Some((la, (lo + 540.0).rem_euclid(360.0) - 180.0));
                    ui.close();
                }
                if ui.button("Oil spill here").clicked() {
                    self.spill_here = Some((la, (lo + 540.0).rem_euclid(360.0) - 180.0));
                    ui.close();
                }
                if ui.button("Add my vessel here").clicked() {
                    self.add_vessel_at(la, (lo + 540.0).rem_euclid(360.0) - 180.0);
                    ui.close();
                }
                if ui.button("Centre here").clicked() {
                    self.lat = la;
                    self.lon = lo;
                    ui.close();
                }
            }
        });
        // Overlays.
        let mut info = String::new();
        if let Some((la, lo)) = self.hover {
            info = format!("{la:.4}°, {lo:.4}°");
            if let Some((e, _)) = self.elevation(la, lo) {
                if e < 0.0 {
                    info.push_str(&format!(" · depth {:.0} m", -e));
                } else {
                    info.push_str(&format!(" · elevation {e:.0} m"));
                }
            }
            if self.sea.on
                && let Some((hs, towards, period)) = self.sea.sample(la, lo)
            {
                info.push_str(&format!(
                    " · waves Hs {hs:.1} m, {period:.1} s, towards {:.0}°",
                    towards.rem_euclid(360.0)
                ));
            }
        }
        painter.text(
            rect.left_bottom() + Vec2::new(10.0, -26.0),
            egui::Align2::LEFT_BOTTOM,
            info,
            egui::FontId::proportional(13.0),
            Color32::WHITE,
        );
        let mut status = format!(
            "{drawn} tiles · {} loading · zoom {:.1}",
            self.pending.len(),
            self.zoom_level(rect).max(0.0)
        );
        if let Some(e) = &self.last_error
            && self.tiles.is_empty()
        {
            status.push_str(&format!(" · network: {e}"));
        }
        if !self.status.is_empty() {
            status = format!("{} · {status}", self.status);
        }
        for extra in [self.marine.status.clone(), self.marine.feed_status()] {
            if !extra.is_empty() {
                status = format!("{extra} · {status}");
            }
        }
        let camera_km = view.height();
        status.push_str(&format!(
            " · {:.4}°, {:.4}° · camera {} · tilt {:.0}° · heading {:.0}°",
            self.lat,
            self.lon,
            if camera_km >= 10.0 {
                format!("{camera_km:.0} km")
            } else {
                format!("{:.0} m", camera_km * 1000.0)
            },
            self.tilt,
            self.heading
        ));
        // Status above the credits, both wrapped to the map width so long
        // lines never run into each other.
        let wrap = rect.width() - 20.0;
        let credits = painter.layout(
            self.credits(),
            egui::FontId::proportional(10.0),
            Color32::from_white_alpha(150),
            wrap,
        );
        let status = painter.layout(
            status,
            egui::FontId::proportional(10.5),
            Color32::from_white_alpha(185),
            wrap,
        );
        let credits_top = rect.bottom() - 8.0 - credits.size().y;
        let status_top = credits_top - 4.0 - status.size().y;
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.left(), status_top - 4.0),
                rect.right_bottom(),
            ),
            0.0,
            Color32::from_black_alpha(90),
        );
        painter.galley(
            Pos2::new(rect.left() + 10.0, status_top),
            status,
            Color32::WHITE,
        );
        painter.galley(
            Pos2::new(rect.left() + 10.0, credits_top),
            credits,
            Color32::WHITE,
        );
        painter.text(
            rect.left_top() + Vec2::new(10.0, 8.0),
            egui::Align2::LEFT_TOP,
            "Drag to pan · Wheel to zoom · Shift+drag or middle-drag to tilt/rotate · Double-click to dive · Right-click to create a scene",
            egui::FontId::proportional(11.0),
            Color32::from_white_alpha(150),
        );
        self.place_window(&ctx, rect);
        self.marine_details(&ctx, rect);
        self.fleet_window(&ctx, rect);
        self.seismic_window(&ctx, rect);
        if self.busy() || self.marine.busy() || self.marine.ships {
            ctx.request_repaint_after(std::time::Duration::from_millis(if self.marine.ships {
                1000
            } else {
                60
            }));
        }
        created
    }

    /// Default real sea built when Pelagic starts with nothing to open:
    /// Kavaratti atoll, Lakshadweep (coral reef, deep water close by).
    /// Builds a detailed scene at a place (the viewport's "build here").
    pub(crate) fn build_here(&mut self, lat: f64, lon: f64) {
        self.lat = lat;
        self.lon = lon;
        self.start_build(lat, lon);
        self.status = format!("Building a detailed scene at {lat:.4}°, {lon:.4}°");
    }
    pub(crate) fn build_default_sea(&mut self) {
        self.lat = 10.56;
        self.lon = 72.62;
        self.altitude = 60.0;
        self.start_build(10.555, 72.612);
        self.status =
            "Building the real sea at Kavaratti, Lakshadweep (seafloor, reefs, live waves)".into();
    }

    /// A finished scene build, if any (polled every frame by the editor).
    pub(crate) fn poll_build(&mut self) -> Option<Scene> {
        let rx = self.building.as_ref()?;
        let result = rx.try_recv().ok()?;
        self.building = None;
        match result {
            Ok(scene) => {
                self.status = format!("Created \"{}\".", scene.name);
                Some(scene)
            }
            Err(e) => {
                self.status = format!("Could not build a scene here: {e}");
                None
            }
        }
    }

    fn start_build(&mut self, lat: f64, lon: f64) {
        self.start_build_with(lat, lon, None);
    }

    /// Builds a scene at (lat, lon) with `sea`, or with the live forecast
    /// sea of the current hour when enabled (the seabed alone if the
    /// forecast cannot be reached).
    fn start_build_with(&mut self, lat: f64, lon: f64, sea: Option<(Conditions, String)>) {
        let (send, receive) = mpsc::channel();
        let live = self.live_sea;
        std::thread::spawn(move || {
            let result = build_scene(lat, lon).map(|mut scene| {
                let chosen = sea.or_else(|| {
                    live.then(|| crate::forecast::point(lat, lon, 1).ok())
                        .flatten()
                        .map(|f| {
                            let i = f.index_at(&crate::forecast::now_hour());
                            (f.hours[i].clone(), f.source.clone())
                        })
                });
                if let Some((conditions, source)) = chosen {
                    let _ = pelagic::metocean::apply(&mut scene, &conditions, &source);
                    if scene.validate().is_err() {
                        // Keep the seabed scene if the sea does not fit it.
                        scene.swell_height = 0.0;
                    }
                }
                scene
            });
            let _ = send.send(result);
        });
        self.building = Some(receive);
        self.status = format!("Sampling real seabed at {lat:.3}°, {lon:.3}°…");
    }
}

impl Globe {
    /// The search-and-rescue drift: possible positions (fading where
    /// searched), the search area, the planned track and the LKP.
    fn draw_sar(&self, view: &View, painter: &egui::Painter) {
        for o in self.sar.iter().chain(self.spill.iter()) {
            Self::draw_overlay(o, view, painter);
        }
    }

    fn credits(&self) -> String {
        let mut text = self.layer.attribution().to_string();
        if self.reefs || self.seagrass {
            text.push_str(" · ");
            text.push_str(seafloor::HABITAT_CREDIT);
        }
        if self.features_on {
            text.push_str(" · ");
            text.push_str(seafloor::GAZETTEER_CREDIT);
        }
        if self.sst {
            text.push_str(" · ");
            text.push_str(Layer::Sst.attribution());
        }
        if self.sea.on {
            text.push_str(" · ");
            text.push_str(crate::forecast::ATTRIBUTION);
        }
        for c in self.marine_credits() {
            text.push_str(" · ");
            text.push_str(c);
        }
        text
    }

    fn load_features(&mut self) {
        if self.features.is_some() || self.features_loading.is_some() {
            return;
        }
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let agent = http_agent(60);
            let _ = send.send(seafloor::gazetteer(&agent, &seafloor::cache_dir()));
        });
        self.features_loading = Some(receive);
        self.status = "Loading undersea feature names…".into();
    }

    pub(crate) fn features_loading(&mut self) -> bool {
        self.poll_features();
        self.features_loading.is_some()
            || self.marine.busy()
            || self.place.as_ref().is_some_and(|p| p.loading())
    }

    fn poll_features(&mut self) {
        let Some(rx) = &self.features_loading else {
            return;
        };
        let Ok(result) = rx.try_recv() else {
            return;
        };
        self.features_loading = None;
        match result {
            Ok(list) => {
                self.status = format!("{} named undersea features", list.len());
                self.features = Some(Arc::new(list));
            }
            Err(e) => {
                self.status = format!("Undersea names unavailable: {e}");
                self.features_on = false;
            }
        }
    }

    /// Flies to the undersea feature whose name best matches the search.
    pub(crate) fn find_feature(&mut self) {
        let Some(list) = self.features.clone() else {
            self.features_on = true;
            self.load_features();
            self.status = "Loading undersea feature names; press Enter again in a moment.".into();
            return;
        };
        let query = self.goto.trim().to_lowercase();
        let best = list
            .iter()
            .filter(|f| f.label().to_lowercase().contains(&query))
            .min_by_key(|f| (f.label().to_lowercase() != query, f.rank(), f.label().len()));
        match best {
            Some(f) => {
                self.lat = f.lat;
                self.lon = f.lon;
                self.altitude = match f.rank() {
                    0 => 2_500.0,
                    1 => 600.0,
                    _ => 200.0,
                };
                self.status = format!("{} · {:.3}°, {:.3}°", f.label(), f.lat, f.lon);
                self.features_on = true;
            }
            None => self.status = format!("No undersea feature named “{}”.", self.goto.trim()),
        }
    }

    /// Named undersea features in view, largest kinds first, without
    /// overlapping labels.
    fn draw_features(&mut self, view: &View, painter: &egui::Painter) {
        self.poll_features();
        if !self.features_on {
            return;
        }
        let Some(list) = self.features.clone() else {
            return;
        };
        let height = view.height();
        let max_rank = if height > 6_000.0 {
            0
        } else if height > 1_200.0 {
            1
        } else {
            2
        };
        let font = egui::FontId::proportional(11.0);
        let mut placed: Vec<Rect> = vec![];
        for rank in 0..=max_rank {
            for f in list.iter().filter(|f| f.rank() == rank) {
                if placed.len() >= 90 {
                    return;
                }
                let p = unit(f.lat, f.lon);
                let Some((q, true)) = view.project([p[0] * R, p[1] * R, p[2] * R]) else {
                    continue;
                };
                if !view.rect.shrink(4.0).contains(q) {
                    continue;
                }
                let colour = match rank {
                    0 => Color32::from_rgb(255, 214, 120),
                    1 => Color32::from_rgb(150, 222, 255),
                    _ => Color32::from_rgb(210, 214, 222),
                };
                let galley = painter.layout_no_wrap(f.label(), font.clone(), colour);
                let rect = Rect::from_min_size(
                    q + Vec2::new(7.0, -galley.size().y * 0.5 - 2.0),
                    galley.size() + Vec2::new(4.0, 4.0),
                );
                if placed.iter().any(|r| r.expand(2.0).intersects(rect)) {
                    continue;
                }
                placed.push(rect);
                painter.circle_filled(q, 3.0, colour);
                painter.circle_stroke(
                    q,
                    3.0,
                    egui::Stroke::new(1.0, Color32::from_black_alpha(170)),
                );
                painter.rect_filled(rect, 3.0, Color32::from_black_alpha(120));
                painter.galley(rect.min + Vec2::new(2.0, 2.0), galley, colour);
            }
        }
    }

    fn draw_overlay(o: &SarOverlay, view: &View, painter: &egui::Painter) {
        let at = |lat: f64, lon: f64| {
            let p = unit(lat, lon);
            view.project([p[0] * R, p[1] * R, p[2] * R])
                .filter(|(_, facing)| *facing)
                .map(|(q, _)| q)
        };
        for &(lat, lon, left) in &o.particles {
            if let Some(q) = at(lat, lon) {
                painter.rect_filled(
                    Rect::from_center_size(q, Vec2::splat(2.0)),
                    0.0,
                    Color32::from_rgba_unmultiplied(
                        o.colour.0,
                        o.colour.1,
                        o.colour.2,
                        (30.0 + 170.0 * left) as u8,
                    ),
                );
            }
        }
        let line = |points: &[(f64, f64)], stroke: egui::Stroke| {
            let path: Vec<Pos2> = points.iter().filter_map(|p| at(p.0, p.1)).collect();
            if path.len() == points.len() && path.len() > 1 {
                painter.add(egui::Shape::line(path, stroke));
            }
        };
        line(
            &o.area,
            egui::Stroke::new(1.5, Color32::from_rgb(0x19, 0x9e, 0x70)),
        );
        line(
            &o.track,
            egui::Stroke::new(1.2, Color32::from_rgb(0x39, 0x87, 0xe5)),
        );
        if let Some(q) = at(o.lkp.0, o.lkp.1) {
            let s = egui::Stroke::new(2.0, theme::STRONG);
            painter.line_segment([q - Vec2::new(5.0, 5.0), q + Vec2::new(5.0, 5.0)], s);
            painter.line_segment([q - Vec2::new(5.0, -5.0), q + Vec2::new(5.0, -5.0)], s);
        }
    }
}

/// Whole hours from `a` to `b` ("YYYY-MM-DDTHH:MM"), 0 if unreadable.
pub(crate) fn hours_since(a: &str, b: &str) -> i64 {
    sea_layer::hours_between(a, b).unwrap_or(0)
}

/// Terrarium elevations on a node-centred `size`² grid of `spacing` metres
/// around (lat, lon), row 0 south, bilinearly sampled from zoom `z` tiles.
pub(super) fn terrarium_grid(
    agent: &ureq::Agent,
    cache: &std::path::Path,
    z: u8,
    lat: f64,
    lon: f64,
    size: usize,
    spacing: f64,
) -> Result<Vec<f64>, String> {
    let n = 2f64.powi(z as i32);
    let half = (size - 1) as f64 * spacing * 0.5;
    let mut tiles: HashMap<(u32, u32), Arc<Vec<f32>>> = HashMap::new();
    let mut sample = |la: f64, lo: f64| -> Result<f64, String> {
        let fx = (lo + 180.0) / 360.0 * n;
        let r = la.to_radians();
        let fy = (1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
        let (px, py) = (fx * 256.0 - 0.5, fy * 256.0 - 0.5);
        let (x0, y0) = (px.floor(), py.floor());
        let (tx, ty) = (px - x0, py - y0);
        let mut get = |gx: f64, gy: f64| -> Result<f64, String> {
            let (gx, gy) = (gx as i64, gy as i64);
            let (tile_x, tile_y) = (
                (gx.div_euclid(256)).rem_euclid(n as i64) as u32,
                gy.div_euclid(256).clamp(0, n as i64 - 1) as u32,
            );
            if let std::collections::hash_map::Entry::Vacant(slot) = tiles.entry((tile_x, tile_y)) {
                slot.insert(relief(agent, cache, z, tile_x, tile_y)?);
            }
            let t = &tiles[&(tile_x, tile_y)];
            let (ix, iy) = (gx.rem_euclid(256) as usize, gy.rem_euclid(256) as usize);
            Ok(t[iy * 256 + ix] as f64)
        };
        let a = get(x0, y0)?;
        let b = get(x0 + 1.0, y0)?;
        let c = get(x0, y0 + 1.0)?;
        let d = get(x0 + 1.0, y0 + 1.0)?;
        Ok(a * (1.0 - tx) * (1.0 - ty) + b * tx * (1.0 - ty) + c * (1.0 - tx) * ty + d * tx * ty)
    };
    let metres_per_degree_lat = 111_132.0;
    let metres_per_degree_lon = 111_320.0 * lat.to_radians().cos();
    let mut out = Vec::with_capacity(size * size);
    for j in 0..size {
        let y = -half + j as f64 * spacing;
        for i in 0..size {
            let x = -half + i as f64 * spacing;
            out.push(sample(
                lat + y / metres_per_degree_lat,
                lon + x / metres_per_degree_lon,
            )?);
        }
    }
    Ok(out)
}

/// Land from the terrain tiles (SRTM-class detail), sea floor from NOAA's
/// mosaic (ETOPO 2022 and coastal surveys) wherever it has data.
/// Sentinel-2 colours (sRGB bytes) on a node-centred grid, row 0 south.
pub(super) fn sentinel_grid(
    agent: &ureq::Agent,
    cache: &std::path::Path,
    z: u8,
    lat: f64,
    lon: f64,
    size: usize,
    spacing: f64,
) -> Result<Vec<u8>, String> {
    let n = 2f64.powi(z as i32);
    let half = (size - 1) as f64 * spacing * 0.5;
    let mut tiles: HashMap<(u32, u32), Arc<Vec<u8>>> = HashMap::new();
    let mut out = Vec::with_capacity(size * size * 3);
    for j in 0..size {
        let la = lat + (-half + j as f64 * spacing) / 111_132.0;
        for i in 0..size {
            let lo = lon + (-half + i as f64 * spacing) / (111_320.0 * lat.to_radians().cos());
            let fx = (lo + 180.0) / 360.0 * n;
            let r = la.to_radians();
            let fy = (1.0 - (r.tan() + 1.0 / r.cos()).ln() / std::f64::consts::PI) / 2.0 * n;
            let (tx, ty) = (
                (fx.floor() as i64).rem_euclid(n as i64) as u32,
                (fy.floor() as i64).clamp(0, n as i64 - 1) as u32,
            );
            if let std::collections::hash_map::Entry::Vacant(slot) = tiles.entry((tx, ty)) {
                slot.insert(Arc::new(sentinel_rgba(agent, cache, z, tx, ty)?));
            }
            let t = &tiles[&(tx, ty)];
            let px = (((fx - fx.floor()) * 256.0) as usize).min(255);
            let py = (((fy - fy.floor()) * 256.0) as usize).min(255);
            let k = (py * 256 + px) * 4;
            out.extend_from_slice(&t[k..k + 3]);
        }
    }
    Ok(out)
}

pub(super) fn merge_elevations(land: &[f64], sea: Option<&[f32]>) -> (Vec<f64>, usize) {
    let mut from_sea = 0;
    let out = land
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let n = sea
                .and_then(|s| s.get(i))
                .copied()
                .filter(|v| *v != 0.0)
                .unwrap_or(f32::NAN) as f64;
            let e = if !n.is_finite() || (t > 0.5 && n > -3.0) {
                t
            } else {
                from_sea += 1;
                n
            };
            e.clamp(-10_999.0, 8_999.0)
        })
        .collect();
    (out, from_sea)
}

/// Day of year (1-366) and UTC hours for a Unix time.
pub(crate) fn utc_day_hours(unix: i64) -> (f64, f64) {
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400) as f64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy_march = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy_march + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    let day = doy_march - (153 * mp + 2) / 5 + 1;
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let before = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334][(month - 1) as usize];
    let doy = before + day + if leap && month > 2 { 1 } else { 0 };
    (doy as f64, seconds / 3600.0)
}

/// Sun elevation and azimuth (degrees) at a place now; at night, 10:00
/// local solar time today, so a new scene is never black.
pub(crate) fn sun_now(lat: f64, lon: f64) -> (f64, f64, bool) {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (day, hours) = utc_day_hours(unix);
    let (e, a) = pelagic_render_gpu::sky::solar_position(lat, lon, day, hours);
    if e >= 4.0 {
        return (e.min(90.0), a, true);
    }
    let morning = (10.0 - lon / 15.0).rem_euclid(24.0);
    let (e, a) = pelagic_render_gpu::sky::solar_position(lat, lon, day, morning);
    (e.clamp(-6.0, 90.0), a, false)
}

/// Builds a Pelagic scene at (lat, lon):
///
/// * seabed: a 257×257 grid at 20 m (5.1 km square) — sea floor from the
///   NOAA DEM Global Mosaic, land from the AWS terrain tiles;
/// * backdrop: 257×257 at 320 m (82 km square) from the same sources, so
///   real coasts, islands and mountains reach the horizon (visual only);
/// * habitat: coral reef and seagrass extents from UNEP-WCMC's global maps;
/// * sun: the real solar position there now (10:00 local if it is night).
///
/// Each source fails soft: without NOAA the terrain tiles are used, without
/// habitat data no reef is drawn.
pub fn build_scene(lat: f64, lon: f64) -> Result<Scene, String> {
    build_at(lat, lon, None)
}

/// Nearest grid node at least 8 m deep to the grid centre, as (x, y) metres.
fn nearest_water(grid: &[f64], n: usize, spacing: f64) -> Option<(f64, f64)> {
    nearest_deeper(grid, n, spacing, -12.0).or_else(|| nearest_deeper(grid, n, spacing, -8.0))
}

fn nearest_deeper(grid: &[f64], n: usize, spacing: f64, limit: f64) -> Option<(f64, f64)> {
    let half = (n - 1) as f64 * spacing * 0.5;
    let mut best: Option<(f64, f64, f64)> = None;
    for j in 0..n {
        for i in 0..n {
            if grid[j * n + i] <= limit {
                let (x, y) = (-half + i as f64 * spacing, -half + j as f64 * spacing);
                let r = x.hypot(y);
                if best.is_none_or(|b| r < b.2) {
                    best = Some((x, y, r));
                }
            }
        }
    }
    best.map(|b| (b.0, b.1))
}

/// Builds at (lat, lon); a point on land or in shallow water moves the
/// scene to the nearest open water and keeps `focus` (the clicked point) in
/// view.
fn build_at(lat: f64, lon: f64, focus: Option<(f64, f64)>) -> Result<Scene, String> {
    if lat.abs() > 84.0 {
        return Err("Pick a point between 84°S and 84°N.".into());
    }
    let agent: ureq::Agent = http_agent(45);
    let tiles = crate::project::data_dir().join("tiles");
    let cache = seafloor::cache_dir();
    let size = 257usize;
    let spacing = 20.0;
    let half = (size - 1) as f64 * spacing * 0.5;
    let land = terrarium_grid(&agent, &tiles, 13, lat, lon, size, spacing)?;
    let noaa = seafloor::dem(&agent, &cache, lat, lon, size, spacing);
    let (elevations, from_noaa) = merge_elevations(&land, noaa.as_deref().ok());
    let centre = elevations[(size / 2) * size + size / 2];
    // A relocated scene (harbour, coast) may be shallower than open water.
    if centre > if focus.is_some() { -2.5 } else { -6.0 } {
        let what = if centre > 0.0 {
            format!("land ({centre:.0} m)")
        } else {
            format!("only {:.1} m deep", -centre)
        };
        if focus.is_some() {
            return Err(format!("no open water deeper than 6 m here ({what})."));
        }
        // Nearest open water: in this 5 km area, else within ~40 km.
        let shift = nearest_water(&elevations, size, spacing).or_else(|| {
            let wide = seafloor::dem(&agent, &cache, lat, lon, 129, 600.0).ok()?;
            let wide: Vec<f64> = wide.iter().map(|v| *v as f64).collect();
            nearest_water(&wide, 129, 600.0)
        });
        let Some((dx, dy)) = shift else {
            return Err(format!(
                "the point is {what} and there is no open water within 40 km."
            ));
        };
        let nlat = lat + dy / 111_132.0;
        let nlon = lon + dx / (111_320.0 * lat.to_radians().cos().max(0.05));
        return build_at(nlat, nlon, Some((lat, lon)));
    }
    let depth = -centre;
    let grid = pelagic::terrain::HeightGrid::new(size, size, [-half, -half], spacing, elevations)
        .map_err(str::to_string)?;
    // Backdrop: 16 scene cells per backdrop cell, centred on the scene, so
    // the renderer can stitch the two without cracks.
    let backdrop_spacing = spacing * 16.0;
    let backdrop = terrarium_grid(&agent, &tiles, 10, lat, lon, size, backdrop_spacing)
        .ok()
        .and_then(|far_land| {
            let far_sea = seafloor::dem(&agent, &cache, lat, lon, size, backdrop_spacing).ok();
            let (far, _) = merge_elevations(&far_land, far_sea.as_deref());
            let h = (size - 1) as f64 * backdrop_spacing * 0.5;
            pelagic::terrain::HeightGrid::new(size, size, [-h, -h], backdrop_spacing, far).ok()
        });
    // Satellite colours for land (only where there is land to colour).
    let has_land = |g: &pelagic::terrain::HeightGrid| g.elevations.iter().any(|e| *e > 0.5);
    let imagery = has_land(&grid)
        .then(|| sentinel_grid(&agent, &tiles, 14, lat, lon, size, spacing).ok())
        .flatten()
        .and_then(|rgb| {
            pelagic::habitat::Imagery::new(
                size,
                size,
                [-half, -half],
                spacing,
                rgb,
                SENTINEL_CREDIT,
            )
            .ok()
        });
    let backdrop_imagery = backdrop.as_ref().filter(|b| has_land(b)).and_then(|b| {
        let rgb = sentinel_grid(&agent, &tiles, 10, lat, lon, size, backdrop_spacing).ok()?;
        pelagic::habitat::Imagery::new(size, size, b.origin, backdrop_spacing, rgb, SENTINEL_CREDIT)
            .ok()
    });
    // Habitat masks on the scene grid; reef takes precedence.
    let reef = seafloor::habitat_mask(
        &agent,
        &cache,
        seafloor::Habitat::CoralReefs,
        lat,
        lon,
        size,
        spacing,
    );
    let grass = seafloor::habitat_mask(
        &agent,
        &cache,
        seafloor::Habitat::Seagrass,
        lat,
        lon,
        size,
        spacing,
    );
    let habitat_note = match (&reef, &grass) {
        (Err(e), Err(_)) => format!("habitat maps unavailable ({e})"),
        _ => String::new(),
    };
    let classes: Vec<u8> = (0..size * size)
        .map(|i| {
            if reef.as_ref().is_ok_and(|m| m[i]) {
                pelagic::habitat::REEF
            } else if grass.as_ref().is_ok_and(|m| m[i]) {
                pelagic::habitat::SEAGRASS
            } else {
                0
            }
        })
        .collect();
    let habitat = classes
        .iter()
        .any(|c| *c != 0)
        .then(|| {
            pelagic::habitat::Habitat::new(
                size,
                size,
                [-half, -half],
                spacing,
                classes,
                seafloor::HABITAT_CREDIT,
            )
            .ok()
        })
        .flatten();
    let finest = seafloor::finest_source(&agent, lat, lon);
    let (sun_elevation, sun_azimuth, live_sun) = sun_now(lat, lon);
    let vehicle_depth = (depth * 0.45).clamp(2.0, 25.0);
    let reef_fraction = habitat
        .as_ref()
        .map_or(0.0, |h| h.fraction(pelagic::habitat::REEF));
    let seagrass_fraction = habitat
        .as_ref()
        .map_or(0.0, |h| h.fraction(pelagic::habitat::SEAGRASS));
    let mut scene = Scene {
        name: format!(
            "Ocean at {:.3}°{} {:.3}°{}",
            lat.abs(),
            if lat >= 0.0 { "N" } else { "S" },
            lon.abs(),
            if lon >= 0.0 { "E" } else { "W" }
        ),
        terrain: 3,
        grid: Some(Arc::new(grid)),
        backdrop: backdrop.map(Arc::new),
        imagery: imagery.map(Arc::new),
        backdrop_imagery: backdrop_imagery.map(Arc::new),
        habitat: habitat.map(Arc::new),
        bottom_depth: depth.max(3.0),
        vehicle_depth,
        structure: false,
        quality: 1,
        wave_height: (depth * 0.2).min(1.0),
        sun_elevation,
        sun_azimuth,
        clouds: 0.3,
        // Clear tropical reef water; turbid shelf seas elsewhere.
        attenuation: if reef_fraction > 0.0 { 0.025 } else { 0.05 },
        camera: [-120.0, -160.0, 14.0],
        target: [0.0, 0.0, -2.0],
        ..Default::default()
    };
    scene.body_model.hold_depth = Some(vehicle_depth);
    scene.school.center = pelagic::math::Vec3::new(6.0, 4.0, -vehicle_depth);
    // A real place opens as it is: real animals (OBIS species) and live
    // ships are drawn by the viewport; the reference vehicle and simulated
    // school are added only when a tool needs them.
    scene.vehicle = false;
    scene.school.count = 0;
    // Open on the reef if one is mapped, else facing the shore, else on the sea.
    let views = pelagic::landscape::viewpoints(
        scene.grid.as_deref(),
        scene.habitat.as_deref(),
        scene.bottom_depth,
    );
    if let Some(v) = ["Coral reef (underwater)", "Shore", "Sea surface"]
        .iter()
        .find_map(|name| views.iter().find(|v| v.name == *name))
    {
        scene.camera = v.eye;
        scene.target = v.target;
    }
    let resolution = format!(
        "Scene 257×257 at 20 m; backdrop 257×257 at 320 m (82 km). {from_noaa} of {} seabed nodes from NOAA's mosaic{}; land from terrain-tile zoom 13 (~19 m/px). {}{}",
        size * size,
        finest.map_or(String::new(), |c| format!(
            " (finest source at the centre {c:.2}″ ≈ {:.0} m)",
            c * 30.87
        )),
        if reef_fraction + seagrass_fraction > 0.0 {
            format!(
                "Mapped habitat: {:.0}% coral reef, {:.0}% seagrass. ",
                reef_fraction * 100.0,
                seagrass_fraction * 100.0
            )
        } else {
            String::new()
        },
        if live_sun {
            "Sun at the real position now."
        } else {
            "Night there now; sun set to 10:00 local solar time."
        }
    );
    scene.provenance = pelagic::data::Provenance {
        source: format!(
            "{} · AWS Terrain Tiles (Mapzen terrarium){}{}",
            seafloor::DEM_CREDIT,
            if scene.habitat.is_some() {
                format!(" · {}", seafloor::HABITAT_CREDIT)
            } else {
                String::new()
            },
            if habitat_note.is_empty() {
                String::new()
            } else {
                format!(" · {habitat_note}")
            }
        ),
        source_url: "https://www.ncei.noaa.gov/products/etopo-global-relief-model ; https://registry.opendata.aws/terrain-tiles/ ; https://data.unep-wcmc.org/".into(),
        licence: "NOAA: public domain. Terrain tiles: open data, attribution per https://github.com/tilezen/joerd/blob/master/docs/attribution.md. UNEP-WCMC habitat: General Data License, non-commercial use.".into(),
        retrieved_utc: String::new(),
        resolution,
        measured: false,
        classification: "resampled composite terrain".into(),
        origin_latitude: lat,
        origin_longitude: lon,
    };
    if let Some((flat, flon)) = focus {
        // Look from the water at the place that was clicked (port, coast).
        let fx = (flon - lon) * 111_320.0 * lat.to_radians().cos();
        let fy = (flat - lat) * 111_132.0;
        let fz = scene
            .grid
            .as_ref()
            .map_or(0.0, |g| pelagic::ocean::Bathymetry::elevation(&**g, fx, fy))
            .max(0.0);
        let d = fx.hypot(fy).max(1.0);
        let height = (d * 0.03).clamp(12.0, 80.0);
        scene.camera = [0.0, 0.0, height];
        scene.target = [fx, fy, fz + 10.0];
        scene.name = format!("Sea off {flat:.3}°, {flon:.3}°");
        scene.provenance.resolution.push_str(&format!(
            " Clicked point {flat:.4}°, {flon:.4}° is on land or in shallow water; the scene is centred on the nearest open water, {:.1} km away, looking back at it.",
            d / 1000.0
        ));
    }
    scene.validate()?;
    Ok(scene)
}

impl Globe {
    /// Scripted Earth overlays for captures: `reefs`, `seagrass`, `names`,
    /// or `find:<undersea feature name>`.
    pub(crate) fn script(&mut self, command: &str) {
        match command {
            "reefs" => self.reefs = true,
            "seagrass" => self.seagrass = true,
            "names" => {
                self.features_on = true;
                self.load_features();
            }
            other if other.starts_with("layer:") => match &other[6..] {
                "lanes" => self.marine.lanes = true,
                "ports" => self.marine.ports = true,
                "seas" => self.marine.seas = true,
                "buoys" => self.marine.buoys = true,
                "ships" => self.marine.ships = true,
                "life" => self.marine.life = true,
                "fleet" => self.marine.fleet = true,
                "quakes" => self.marine.quakes = true,
                "waves" => self.sea.on = true,
                "seamarks" => self.seamarks = true,
                "boundaries" => self.boundaries = true,
                "islands" => self.islands = true,
                "sst" => self.sst = true,
                "today" => self.layer = Layer::Today,
                _ => {}
            },
            other if other.starts_with("place:") => {
                let v: Vec<f64> = other[6..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                if v.len() == 2 {
                    self.open_place(v[0], v[1]);
                }
            }
            other if other.starts_with("vessel:") => {
                // vessel:<lat>,<lon>,<knots>,<course>,<name>
                let parts: Vec<&str> = other[7..].splitn(5, ',').collect();
                let num = |i: usize| parts.get(i).and_then(|s| s.trim().parse::<f64>().ok());
                if let (Some(la), Some(lo)) = (num(0), num(1)) {
                    self.marine.add_scripted(
                        la,
                        lo,
                        num(2).unwrap_or(0.0),
                        num(3).unwrap_or(0.0),
                        parts.get(4).copied().unwrap_or("Vessel"),
                    );
                }
            }
            other if other.starts_with("find:") => {
                self.goto = other[5..].to_string();
                self.find_feature();
            }
            _ => {}
        }
    }
}
impl Globe {
    pub(crate) fn features_busy(&self) -> bool {
        self.features_loading.is_some()
            || self.marine.busy()
            || self.place.as_ref().is_some_and(|p| p.loading())
    }
}

pub(super) const SENTINEL_CREDIT: &str = "Sentinel-2 cloudless 2016 by EOX IT Services GmbH (contains modified Copernicus Sentinel data 2016), CC BY 4.0; where 2016 masks small islands, Sentinel-2 cloudless 2024 (Copernicus Sentinel data 2024), CC BY-NC-SA 4.0";

/// Satellite imagery on land and in shallow water, bathymetry relief in the
/// deep ocean (blended between 60 m and 8 m depth).
/// A Sentinel-2 cloudless tile as 256×256 RGBA. The 2016 mosaic (CC BY)
/// fills open water, and with it many small islands and atolls, with one
/// flat colour; there the 2024 mosaic (CC BY-NC-SA) is used instead.
fn sentinel_rgba(
    agent: &ureq::Agent,
    cache: &std::path::Path,
    z: u8,
    x: u32,
    y: u32,
) -> Result<Vec<u8>, String> {
    let key = (Layer::Sentinel, z, x, y);
    let (rgba, _) = decode(key, &fetch(agent, cache, key)?)?;
    let first = [rgba[0], rgba[1], rgba[2]];
    let flat = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| p[..3].iter().zip(first).all(|(a, b)| a.abs_diff(b) <= 3));
    if !flat {
        return Ok(rgba);
    }
    let path = cache
        .join("s2cloudless-2024")
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.jpg"));
    let bytes = match std::fs::read(&path) {
        Ok(b) if !b.is_empty() => b,
        _ => {
            let url = format!(
                "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2024_3857/default/g/{z}/{y}/{x}.jpg"
            );
            let mut response = agent.get(&url).call().map_err(|e| e.to_string())?;
            let b = response
                .body_mut()
                .with_config()
                .limit(4 * 1024 * 1024)
                .read_to_vec()
                .map_err(|e| e.to_string())?;
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
                let _ = std::fs::write(&path, &b);
            }
            b
        }
    };
    match decode(key, &bytes) {
        Ok((newer, _)) => Ok(newer),
        Err(_) => Ok(rgba),
    }
}

/// Satellite colour for a real scene's grid and backdrop (as built), for
/// refreshing scenes whose stored colours came from masked tiles.
pub(super) fn scene_imagery(
    lat: f64,
    lon: f64,
    grid: &pelagic::terrain::HeightGrid,
    backdrop: Option<&pelagic::terrain::HeightGrid>,
) -> (
    Option<pelagic::habitat::Imagery>,
    Option<pelagic::habitat::Imagery>,
) {
    let agent = http_agent(45);
    let tiles = crate::project::data_dir().join("tiles");
    let make = |g: &pelagic::terrain::HeightGrid, zoom: u8| {
        if !g.elevations.iter().any(|e| *e > 0.5) || g.columns != g.rows {
            return None;
        }
        let rgb = sentinel_grid(&agent, &tiles, zoom, lat, lon, g.columns, g.spacing).ok()?;
        pelagic::habitat::Imagery::new(g.columns, g.rows, g.origin, g.spacing, rgb, SENTINEL_CREDIT)
            .ok()
    };
    (make(grid, 14), backdrop.and_then(|b| make(b, 10)))
}

fn hybrid(
    agent: &ureq::Agent,
    cache: &std::path::Path,
    key: Key,
) -> Result<(Vec<u8>, Option<Vec<f32>>), String> {
    let (_, z, x, y) = key;
    let elev = relief(agent, cache, z, x, y)?;
    let mut out = shade(&elev, z, y);
    if let Ok(img) = sentinel_rgba(agent, cache, z, x, y) {
        for (i, p) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let e = elev[i];
            let t = ((e + 60.0) / 52.0).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            for c in 0..3 {
                p[c] = (p[c] as f32 * (1.0 - t) + img[i * 4 + c] as f32 * t) as u8;
            }
        }
    }
    Ok((out, Some(elev.to_vec())))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_math_projection_and_pick_round_trip() {
        assert!((tile_lat(0.0, 1.0) - MAX_LAT).abs() < 1e-9);
        assert!((tile_lon(0.5, 1.0) - 0.0).abs() < 1e-9);
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        for (lat, lon, alt) in [
            (15.0, 73.0, 300.0),
            (-40.0, -120.0, 8000.0),
            (60.0, 10.0, 1.0),
        ] {
            let view = View::new(lat, lon, alt, 0.0, 0.0, rect);
            let (p, facing) = view.project(scale(unit(lat, lon), R)).unwrap();
            assert!(facing);
            assert!((p - rect.center()).length() < 0.01);
            let (la, lo) = view.pick(rect.center() + Vec2::new(40.0, -30.0)).unwrap();
            let (q, _) = view.project(scale(unit(la, lo), R)).unwrap();
            assert!((q - (rect.center() + Vec2::new(40.0, -30.0))).length() < 0.05);
        }
        // Far side of the globe is not facing the camera.
        let view = View::new(0.0, 0.0, 1000.0, 0.0, 0.0, rect);
        assert!(!view.project(scale(unit(0.0, 180.0), R)).is_none_or(|p| p.1));
    }
    #[test]
    fn tilted_view_keeps_target_centred_and_faces_heading() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let view = View::new(20.0, -150.0, 50.0, 60.0, 90.0, rect);
        // Target stays at the centre; camera is lower than its distance.
        let (p, facing) = view.project(scale(unit(20.0, -150.0), R)).unwrap();
        assert!(facing && (p - rect.center()).length() < 0.01);
        assert!((view.height() - 25.0).abs() < 0.5);
        // Facing east: a point further east projects above the centre, a
        // point to the north projects left of it.
        let (east, _) = view.project(scale(unit(20.0, -149.9), R)).unwrap();
        let (north, _) = view.project(scale(unit(20.1, -150.0), R)).unwrap();
        assert!(east.y < rect.center().y && (east.x - rect.center().x).abs() < 1.0);
        assert!(north.x < rect.center().x);
        // Pick round-trips below the horizon; near the surface, the top of a
        // steeply tilted frame is sky.
        let q = rect.center() + Vec2::new(-60.0, 80.0);
        let (la, lo) = view.pick(q).unwrap();
        let (back, _) = view.project(scale(unit(la, lo), R)).unwrap();
        assert!((back - q).length() < 0.05);
        let low = View::new(20.0, -150.0, 5.0, 78.0, 90.0, rect);
        assert!(low.pick(Pos2::new(400.0, 2.0)).is_none());
        assert!(low.pick(Pos2::new(400.0, 598.0)).is_some());
        // Elevation sampling is bilinear over pixel centres.
        let mut e = vec![0.0f32; 256 * 256];
        e[0] = -100.0;
        e[1] = -200.0;
        assert_eq!(sample_elevation(&e, 0.5 / 256.0, 0.5 / 256.0), -100.0);
        assert_eq!(sample_elevation(&e, 1.0 / 256.0, 0.5 / 256.0), -150.0);
    }
    #[test]
    fn relief_shading_ramps_depth_and_decodes_terrarium() {
        let deep = ramp(-5000.0);
        let shallow = ramp(-20.0);
        assert!(deep[2] > deep[0] && shallow[1] > deep[1]);
        let elev = vec![-3000.0f32; 256 * 256];
        let rgba = shade(&elev, 5, 10);
        assert_eq!(rgba.len(), 256 * 256 * 4);
        assert!(rgba[2] > rgba[0], "flat deep ocean is blue");
        // Terrarium encoding of -1000 m: v + 32768 = 31768 = 124*256 + 24.
        let mut png_bytes = vec![];
        {
            let mut enc = png::Encoder::new(&mut png_bytes, 256, 256);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let data: Vec<u8> = (0..256 * 256).flat_map(|_| [124u8, 24, 0]).collect();
            enc.write_header().unwrap().write_image_data(&data).unwrap();
        }
        let e = terrarium(&png_bytes).unwrap();
        assert!((e[0] + 1000.0).abs() < 1e-3);
    }
}
#[cfg(test)]
mod live_build_probe {
    #[test]
    #[ignore]
    fn kavaratti() {
        let agent = super::http_agent(45);
        let cache = super::seafloor::cache_dir();
        let d = super::seafloor::dem(&agent, &cache, 10.555, 72.612, 257, 20.0);
        println!(
            "dem: {:?}",
            d.as_ref()
                .map(|v| v[128 * 257 + 128])
                .map_err(|e| e.clone())
        );
        let t = super::terrarium_grid(
            &agent,
            &crate::project::data_dir().join("tiles"),
            13,
            10.555,
            72.612,
            257,
            20.0,
        );
        println!(
            "terrarium: {:?}",
            t.as_ref()
                .map(|v| v[128 * 257 + 128])
                .map_err(|e| e.clone())
        );
        let s = super::build_scene(10.555, 72.612);
        println!("scene: {:?}", s.map(|s| (s.name, s.bottom_depth)));
    }
}
