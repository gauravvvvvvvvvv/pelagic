//! Real seafloor data for scenes and the Earth view, fetched on demand and
//! cached under `%LOCALAPPDATA%\Pelagic\seafloor`:
//!
//! * Depth: NOAA NCEI DEM Global Mosaic (ETOPO 2022 at 15 arc-seconds,
//!   ~450 m, with NOAA coastal DEMs down to 1/9 arc-second where surveyed).
//!   US Government work, public domain.
//! * Habitat: UNEP-WCMC Global Distribution of Warm-water Coral Reefs
//!   (WCMC-008, v4.1, 2021) and of Seagrasses (WCMC-013/014, v7.1). UNEP-WCMC
//!   General Data License: non-commercial use; commercial use needs
//!   permission from UNEP-WCMC.
//! * Undersea feature names: IHO-IOC GEBCO Gazetteer of Undersea Feature
//!   Names, served by NOAA NCEI.
//!
//! Requests carry only a bounding box or a point; nothing about the user.
use std::path::{Path, PathBuf};

const DEM: &str =
    "https://gis.ngdc.noaa.gov/arcgis/rest/services/DEM_mosaics/DEM_global_mosaic/ImageServer";
const HABITATS: &str = "https://data-gis.unep-wcmc.org/server/rest/services/HabitatsAndBiotopes";
const GAZETTEER: &str = "https://services2.arcgis.com/C8EMgrsFcRFL6LrL/arcgis/rest/services/Undersea_Features/FeatureServer";

pub const DEM_CREDIT: &str =
    "NOAA NCEI DEM Global Mosaic (ETOPO 2022 + coastal DEMs), public domain";
pub const HABITAT_CREDIT: &str = "Coral reefs: UNEP-WCMC, WorldFish, WRI, TNC (2021) v4.1; seagrasses: UNEP-WCMC, Short F.T. (2021) v7.1 — UNEP-WCMC General Data License, non-commercial";
pub const GAZETTEER_CREDIT: &str = "Undersea feature names: IHO-IOC GEBCO Gazetteer (NOAA NCEI)";

pub fn cache_dir() -> PathBuf {
    crate::project::data_dir().join("seafloor")
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

/// GET with an on-disk cache keyed by the URL. `valid` rejects error pages
/// so they are never cached.
fn cached(
    agent: &ureq::Agent,
    cache: &Path,
    url: &str,
    limit: u64,
    valid: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    let path = cache.join(format!("{:016x}.bin", fnv(url)));
    if let Ok(bytes) = std::fs::read(&path)
        && valid(&bytes)
    {
        return Ok(bytes);
    }
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    if !valid(&bytes) {
        let start = String::from_utf8_lossy(&bytes[..bytes.len().min(160)]).into_owned();
        return Err(format!("unexpected response: {start}"));
    }
    let _ = std::fs::create_dir_all(cache);
    let tmp = path.with_extension("part");
    if std::fs::write(&tmp, &bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    Ok(bytes)
}

/// Bounding box [west, south, east, north] (degrees) whose `size`×`size`
/// pixel centres fall exactly on a node-centred grid of `spacing` metres
/// around (lat, lon).
pub fn pixel_box(lat: f64, lon: f64, size: usize, spacing: f64) -> [f64; 4] {
    let half = ((size - 1) as f64 * 0.5 + 0.5) * spacing;
    let dlat = half / 111_132.0;
    let dlon = half / (111_320.0 * lat.to_radians().cos().max(0.05));
    [lon - dlon, lat - dlat, lon + dlon, lat + dlat]
}

/// Little-endian f32 raster (band-sequential, one band), row 0 north.
pub fn parse_bsq(bytes: &[u8], width: usize, height: usize) -> Result<Vec<f32>, String> {
    if bytes.len() < width * height * 4 {
        return Err(format!(
            "elevation raster too short ({} bytes for {width}×{height})",
            bytes.len()
        ));
    }
    Ok(bytes[..width * height * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

/// Elevations (m, negative below sea level) on a `size`² grid of `spacing`
/// metres centred on (lat, lon), row 0 south. NaN where the mosaic has no
/// data.
pub fn dem(
    agent: &ureq::Agent,
    cache: &Path,
    lat: f64,
    lon: f64,
    size: usize,
    spacing: f64,
) -> Result<Vec<f32>, String> {
    let b = pixel_box(lat, lon, size, spacing);
    let url = format!(
        "{DEM}/exportImage?bbox={:.7},{:.7},{:.7},{:.7}&bboxSR=4326&imageSR=4326&size={size},{size}&format=bsq&pixelType=F32&interpolation=RSP_BilinearInterpolation&f=image",
        b[0], b[1], b[2], b[3]
    );
    // The service sometimes answers with an all-zero raster; never accept
    // or cache one (real depths are almost never exactly 0.0).
    let bytes = cached(agent, cache, &url, 8 << 20, |b| {
        b.len() >= size * size * 4 && zero_fraction(&b[..size * size * 4]) < 0.05
    })?;
    let north_first = parse_bsq(&bytes, size, size)?;
    let mut out = Vec::with_capacity(size * size);
    for row in (0..size).rev() {
        out.extend(north_first[row * size..(row + 1) * size].iter().map(|v| {
            if v.is_finite() && v.abs() < 12_000.0 {
                *v
            } else {
                f32::NAN
            }
        }));
    }
    Ok(out)
}

/// Fraction of exactly-zero values in a little-endian f32 raster.
fn zero_fraction(bytes: &[u8]) -> f64 {
    let n = bytes.len() / 4;
    if n == 0 {
        return 1.0;
    }
    let zeros = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|b| f32::from_le_bytes(**b) == 0.0)
        .count();
    zeros as f64 / n as f64
}

/// Finest source cell size (arc-seconds) in the mosaic at a point.
pub fn finest_source(agent: &ureq::Agent, lat: f64, lon: f64) -> Option<f64> {
    let url = format!(
        "{DEM}/identify?geometry=%7B%22x%22%3A{lon:.6}%2C%22y%22%3A{lat:.6}%2C%22spatialReference%22%3A%7B%22wkid%22%3A4326%7D%7D&geometryType=esriGeometryPoint&returnCatalogItems=true&returnGeometry=false&f=json"
    );
    let mut response = agent.get(&url).call().ok()?;
    let text = response
        .body_mut()
        .with_config()
        .limit(4 << 20)
        .read_to_string()
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v["catalogItems"]["features"]
        .as_array()?
        .iter()
        .filter_map(|f| f["attributes"]["CellsizeArcseconds"].as_f64())
        .filter(|c| *c > 0.0)
        .reduce(f64::min)
}

/// Seabed habitat map service and polygon layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Habitat {
    CoralReefs,
    Seagrass,
}
impl Habitat {
    fn service(self) -> (&'static str, u32) {
        match self {
            Habitat::CoralReefs => ("Global_Distribution_of_Coral_Reefs", 1),
            Habitat::Seagrass => ("Global_Distribution_of_Seagrasses", 1),
        }
    }
    /// Map image (transparent PNG) of the habitat over a lon/lat box.
    pub fn export_url(self, bbox: [f64; 4], bbox_sr: u32, width: usize, height: usize) -> String {
        let (service, layer) = self.service();
        format!(
            "{HABITATS}/{service}/MapServer/export?bbox={:.7},{:.7},{:.7},{:.7}&bboxSR={bbox_sr}&imageSR={bbox_sr}&size={width},{height}&layers=show:{layer}&format=png32&transparent=true&dpi=96&f=image",
            bbox[0], bbox[1], bbox[2], bbox[3]
        )
    }
}

fn png_valid(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G'])
}

/// Decodes a PNG to RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err("unexpanded palette".into()),
    };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in buffer[..w * h * channels].chunks_exact(channels) {
        rgba.extend_from_slice(&match channels {
            4 => [p[0], p[1], p[2], p[3]],
            3 => [p[0], p[1], p[2], 255],
            2 => [p[0], p[0], p[0], p[1]],
            _ => [p[0], p[0], p[0], 255],
        });
    }
    Ok((rgba, w, h))
}

/// Habitat presence on the same node-centred grid as `dem` (row 0 south).
pub fn habitat_mask(
    agent: &ureq::Agent,
    cache: &Path,
    habitat: Habitat,
    lat: f64,
    lon: f64,
    size: usize,
    spacing: f64,
) -> Result<Vec<bool>, String> {
    let url = habitat.export_url(pixel_box(lat, lon, size, spacing), 4326, size, size);
    let bytes = cached(agent, cache, &url, 8 << 20, png_valid)?;
    let (rgba, w, h) = decode_png(&bytes)?;
    if (w, h) != (size, size) {
        return Err(format!("habitat map is {w}×{h}, expected {size}²"));
    }
    let mut out = Vec::with_capacity(size * size);
    for row in (0..size).rev() {
        out.extend((0..size).map(|x| rgba[(row * size + x) * 4 + 3] > 40));
    }
    Ok(out)
}

/// A named undersea feature (point label).
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub name: String,
    pub kind: String,
    pub lat: f64,
    pub lon: f64,
}
impl Feature {
    /// Rough size rank for label decluttering (0 = largest features).
    pub fn rank(&self) -> u8 {
        match self.kind.as_str() {
            "Abyssal Plain" | "Basin" | "Fracture Zone" | "Plateau" | "Ridge" | "Ridges"
            | "Rise" | "Trench" | "Trough" | "Seamount Province" | "Continental Slope"
            | "Borderland" | "Fan" | "Shelf" | "Plain" | "Deep" => 0,
            "Seamount" | "Seamounts" | "Guyot" | "Guyots" | "Tablemount" | "Bank" | "Banks"
            | "Canyon" | "Canyons" | "Escarpment" | "Sea Channel" | "Seachannel" | "Channel"
            | "Valley" | "Spur" | "Terrace" | "Reef" | "Reefs" | "Shoal" | "Shoals" | "Caldera"
            | "Saddle" | "Sill" | "Gap" | "Passage" | "Pass" => 1,
            _ => 2,
        }
    }
    pub fn label(&self) -> String {
        if self.name.to_lowercase().contains(&self.kind.to_lowercase()) {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.kind)
        }
    }
}

/// Representative point of an Esri JSON geometry (points, multipoint,
/// polyline middle vertex, polygon centroid or ring average).
fn label_point(f: &serde_json::Value) -> Option<(f64, f64)> {
    if let Some(c) = f.get("centroid") {
        return Some((c["y"].as_f64()?, c["x"].as_f64()?));
    }
    let g = &f["geometry"];
    if let (Some(x), Some(y)) = (g["x"].as_f64(), g["y"].as_f64()) {
        return Some((y, x));
    }
    let list: Vec<&serde_json::Value> = if let Some(p) = g["points"].as_array() {
        p.iter().collect()
    } else if let Some(paths) = g["paths"].as_array() {
        let longest = paths
            .iter()
            .max_by_key(|p| p.as_array().map_or(0, |a| a.len()))?;
        let a = longest.as_array()?;
        return a
            .get(a.len() / 2)
            .and_then(|p| Some((p[1].as_f64()?, p[0].as_f64()?)));
    } else {
        g["rings"].as_array()?.first()?.as_array()?.iter().collect()
    };
    let n = list.len() as f64;
    if n == 0.0 {
        return None;
    }
    let (mut lat, mut lon) = (0.0, 0.0);
    for p in list {
        lon += p[0].as_f64()?;
        lat += p[1].as_f64()?;
    }
    Some((lat / n, lon / n))
}

/// Parses one page of an Esri JSON feature query.
pub fn parse_features(text: &str) -> Result<(Vec<Feature>, bool), String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if let Some(e) = v.get("error") {
        return Err(format!("gazetteer: {e}"));
    }
    let more = v["exceededTransferLimit"].as_bool().unwrap_or(false);
    let features = v["features"]
        .as_array()
        .ok_or("gazetteer: no features")?
        .iter()
        .filter_map(|f| {
            let (lat, lon) = label_point(f)?;
            let name = f["attributes"]["NAME"].as_str()?.trim().to_string();
            let kind = f["attributes"]["TYPE"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            (!name.is_empty() && lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0).then_some(
                Feature {
                    name,
                    kind,
                    lat,
                    lon: (lon + 180.0).rem_euclid(360.0) - 180.0,
                },
            )
        })
        .collect();
    Ok((features, more))
}

/// Every named undersea feature (points, lines and areas), cached after the
/// first download (about 5,400 features, a few hundred kilobytes).
pub fn gazetteer(agent: &ureq::Agent, cache: &Path) -> Result<Vec<Feature>, String> {
    let file = cache.join("gazetteer.json");
    if let Ok(text) = std::fs::read_to_string(&file)
        && let Ok(v) = serde_json::from_str::<Vec<(String, String, f64, f64)>>(&text)
        && v.len() > 1000
    {
        return Ok(v
            .into_iter()
            .map(|(name, kind, lat, lon)| Feature {
                name,
                kind,
                lat,
                lon,
            })
            .collect());
    }
    let mut all = Vec::new();
    for layer in 0..3 {
        let mut offset = 0;
        loop {
            let geometry = match layer {
                0 => "returnGeometry=true",
                1 => "returnGeometry=true&maxAllowableOffset=0.05&geometryPrecision=3",
                _ => "returnGeometry=false&returnCentroid=true",
            };
            let url = format!(
                "{GAZETTEER}/{layer}/query?where=1%3D1&outFields=NAME,TYPE&outSR=4326&{geometry}&resultOffset={offset}&resultRecordCount=2000&f=json"
            );
            let mut response = agent.get(&url).call().map_err(|e| e.to_string())?;
            let text = response
                .body_mut()
                .with_config()
                .limit(32 << 20)
                .read_to_string()
                .map_err(|e| e.to_string())?;
            let (page, more) = parse_features(&text)?;
            let n = page.len();
            all.extend(page);
            if !more || n == 0 || offset > 20_000 {
                break;
            }
            offset += n;
        }
    }
    if all.len() > 1000 {
        let rows: Vec<(String, String, f64, f64)> = all
            .iter()
            .map(|f| (f.name.clone(), f.kind.clone(), f.lat, f.lon))
            .collect();
        let _ = std::fs::create_dir_all(cache);
        if let Ok(text) = serde_json::to_string(&rows) {
            let _ = std::fs::write(&file, text);
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raster_and_feature_parsing() {
        let mut bytes = vec![];
        for v in [-1.0f32, -2.0, -3.0, -4.0, 5.0, 6.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(
            parse_bsq(&bytes, 3, 2).unwrap(),
            vec![-1.0, -2.0, -3.0, -4.0, 5.0, 6.0]
        );
        assert!(parse_bsq(&bytes, 4, 2).is_err());
        // Pixel centres land on the grid nodes: the box spans size × spacing.
        let b = pixel_box(0.0, 10.0, 257, 20.0);
        let width_m = (b[2] - b[0]) * 111_320.0;
        assert!((width_m - 257.0 * 20.0).abs() < 1.0, "{width_m}");
        let page = r#"{"exceededTransferLimit":true,"features":[
            {"attributes":{"NAME":"Satsuma","TYPE":"Seamount"},"geometry":{"points":[[134.0,27.0],[134.2,27.2]]}},
            {"attributes":{"NAME":"Mid-Atlantic Ridge","TYPE":"Ridge"},"geometry":{"paths":[[[-30,10],[-31,20],[-32,30]]]}},
            {"attributes":{"NAME":"Kerguelen","TYPE":"Plateau"},"centroid":{"x":75.0,"y":-55.0}},
            {"attributes":{"NAME":"","TYPE":"Hill"},"geometry":{"x":1,"y":2}}]}"#;
        let (features, more) = parse_features(page).unwrap();
        assert!(more);
        assert_eq!(features.len(), 3);
        assert_eq!(features[0].label(), "Satsuma Seamount");
        assert!((features[0].lat - 27.1).abs() < 1e-9);
        assert_eq!((features[1].lat, features[1].lon), (20.0, -31.0));
        assert_eq!(features[1].label(), "Mid-Atlantic Ridge");
        assert_eq!(features[2].rank(), 0);
        assert!(parse_features(r#"{"error":{"code":400}}"#).is_err());
    }
}
