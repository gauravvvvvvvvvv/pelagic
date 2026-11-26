//! Marine layers on the Earth view, each switchable from the Layers menu:
//!
//! * Shipping lanes (major, middle, minor): Benden, P. (2022), "Shipping
//!   Lanes", CC BY 4.0, derived from the CIA World Factbook.
//! * Ports and sea/ocean names: Natural Earth, public domain.
//! * Wave buoys (live): NOAA National Data Buoy Center latest observations,
//!   public domain.
//! * Ships (live AIS): see `crate::ais`.
//! * Marine life: occurrence records from OBIS (Ocean Biodiversity
//!   Information System), each record under its dataset's licence (mostly
//!   CC0 / CC BY). Records are sightings and specimens collected over years,
//!   not live positions.
//! * Regional live waves: Open-Meteo marine forecast on a fine grid over the
//!   view when zoomed in.
//! * My fleet: vessels the user adds (or follows by MMSI), with recorded
//!   tracks, dead-reckoned predictions and closest-approach warnings.
use super::{Globe, R, View, http_agent, theme, unit};
use crate::ais::{self, Ship};
use egui::{Color32, Pos2, Rect, RichText, Stroke, Vec2};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};

pub const LANES_CREDIT: &str = "Shipping lanes: Benden (2022), CC BY 4.0";
pub const NATURAL_EARTH_CREDIT: &str = "Ports, sea names: Natural Earth";
pub const BUOY_CREDIT: &str = "Buoys: NOAA NDBC";
pub const OBIS_CREDIT: &str = "Marine life: OBIS occurrence records";
pub const SEAMARK_CREDIT: &str = "Seamarks: OpenSeaMap, CC BY-SA 2.0 · OpenStreetMap ODbL";
pub const BOUNDARY_CREDIT: &str =
    "Maritime boundaries: Flanders Marine Institute (2023) Marine Regions, CC BY 4.0";
pub const ISLANDS_CREDIT: &str = "Islands: USGS/Esri/UNEP-WCMC Global Islands";

const LANES_URL: &str = "https://raw.githubusercontent.com/newzealandpaul/Shipping-Lanes/main/data/Shipping_Lanes_v1.geojson";
const PORTS_URL: &str = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_10m_ports.geojson";
const SEAS_URL: &str = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_10m_geography_marine_polys.geojson";
const BUOYS_URL: &str = "https://www.ndbc.noaa.gov/data/latest_obs/latest_obs.txt";

/// Marine-life groups offered (label, scientific name for OBIS, colour).
pub const LIFE_GROUPS: [(&str, &str, [u8; 3]); 8] = [
    ("Sharks", "Selachii", [255, 90, 80]),
    ("Rays and skates", "Batoidea", [255, 160, 60]),
    ("Whales and dolphins", "Cetacea", [120, 190, 255]),
    ("Sea turtles", "Cheloniidae", [120, 230, 140]),
    ("Seals and sea lions", "Pinnipedia", [220, 200, 160]),
    ("Bony fish", "Actinopterygii", [250, 230, 90]),
    ("Corals", "Anthozoa", [255, 120, 200]),
    ("Seabirds (tubenoses)", "Procellariiformes", [230, 230, 240]),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// 0 major, 1 middle, 2 minor.
    pub rank: u8,
    pub points: Vec<(f64, f64)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Smaller = more important.
    pub rank: f64,
    pub kind: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Buoy {
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub time: String,
    pub wave_height: Option<f64>,
    pub period: Option<f64>,
    pub wave_from: Option<f64>,
    pub wind: Option<f64>,
    pub wind_from: Option<f64>,
    pub water_temp: Option<f64>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Sighting {
    pub species: String,
    pub common: String,
    pub lat: f64,
    pub lon: f64,
    pub date: String,
    pub group: usize,
}
/// A vessel in the user's own fleet.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vessel {
    pub name: String,
    /// MMSI to follow on live AIS (0 = positions entered by hand).
    pub mmsi: u64,
    pub kind: String,
    pub lat: f64,
    pub lon: f64,
    /// Speed (knots) and course (degrees true) at the last report.
    pub sog: f64,
    pub cog: f64,
    /// Unix time of the last report.
    pub reported: i64,
    pub track: Vec<(f64, f64, i64)>,
    pub notes: String,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn data_dir() -> std::path::PathBuf {
    crate::project::data_dir().join("marine")
}

/// Download once and keep (static reference layers).
fn cached_text(url: &str, file: &str) -> Result<String, String> {
    let path = data_dir().join(file);
    if let Ok(text) = std::fs::read_to_string(&path)
        && text.len() > 100
    {
        return Ok(text);
    }
    let text = get_text(url, 64 << 20)?;
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(&path, &text);
    Ok(text)
}
/// The latest NOAA NDBC buoy observations (fresh download).
pub fn latest_buoys() -> Option<Vec<Buoy>> {
    get_text(BUOYS_URL, 16 << 20).ok().map(|t| parse_buoys(&t))
}
fn get_text(url: &str, limit: u64) -> Result<String, String> {
    let agent = http_agent(60);
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// Great-circle points between two positions, about one per degree.
fn densify(a: (f64, f64), b: (f64, f64), out: &mut Vec<(f64, f64)>) {
    let (pa, pb) = (unit(a.0, a.1), unit(b.0, b.1));
    let dot = (pa[0] * pb[0] + pa[1] * pb[1] + pa[2] * pb[2]).clamp(-1.0, 1.0);
    let angle = dot.acos();
    let steps = ((angle.to_degrees() - 1e-6).ceil() as usize).clamp(1, 180);
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        if angle < 1e-9 {
            out.push(b);
            continue;
        }
        let (s0, s1) = (((1.0 - t) * angle).sin(), (t * angle).sin());
        let s = angle.sin();
        let p = [
            (pa[0] * s0 + pb[0] * s1) / s,
            (pa[1] * s0 + pb[1] * s1) / s,
            (pa[2] * s0 + pb[2] * s1) / s,
        ];
        let lat = p[2].clamp(-1.0, 1.0).asin().to_degrees();
        let lon = p[1].atan2(p[0]).to_degrees();
        out.push((lat, lon));
    }
}

pub fn parse_lanes(text: &str) -> Result<Vec<Lane>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut out = vec![];
    for f in v["features"].as_array().ok_or("no lanes")? {
        let rank = match f["properties"]["Type"].as_str().unwrap_or("") {
            "Major" => 0,
            "Middle" => 1,
            _ => 2,
        };
        let g = &f["geometry"];
        let lines: Vec<&serde_json::Value> = match g["type"].as_str() {
            Some("MultiLineString") => g["coordinates"]
                .as_array()
                .ok_or("bad lane")?
                .iter()
                .collect(),
            Some("LineString") => vec![&g["coordinates"]],
            _ => continue,
        };
        for line in lines {
            let pts: Vec<(f64, f64)> = line
                .as_array()
                .ok_or("bad lane")?
                .iter()
                .filter_map(|p| Some((p[1].as_f64()?, p[0].as_f64()?)))
                .collect();
            if pts.len() < 2 {
                continue;
            }
            let mut dense = vec![pts[0]];
            for pair in pts.windows(2) {
                // The dataset breaks lanes at the antimeridian.
                if (pair[0].1 - pair[1].1).abs() > 180.0 {
                    if dense.len() > 1 {
                        out.push(Lane {
                            rank,
                            points: std::mem::take(&mut dense),
                        });
                    }
                    dense = vec![pair[1]];
                    continue;
                }
                densify(pair[0], pair[1], &mut dense);
            }
            if dense.len() > 1 {
                out.push(Lane {
                    rank,
                    points: dense,
                });
            }
        }
    }
    Ok(out)
}

pub fn parse_ports(text: &str) -> Result<Vec<Place>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(v["features"]
        .as_array()
        .ok_or("no ports")?
        .iter()
        .filter_map(|f| {
            let c = f["geometry"]["coordinates"].as_array()?;
            Some(Place {
                name: f["properties"]["name"].as_str()?.trim().to_string(),
                lon: c.first()?.as_f64()?,
                lat: c.get(1)?.as_f64()?,
                rank: f["properties"]["scalerank"].as_f64().unwrap_or(8.0),
                kind: "Port".into(),
            })
        })
        .filter(|p| !p.name.is_empty())
        .collect())
}

/// Sea and ocean names, labelled at the vertex average of each feature's
/// largest ring (a fair label point for compact seas).
pub fn parse_seas(text: &str) -> Result<Vec<Place>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(v["features"]
        .as_array()
        .ok_or("no seas")?
        .iter()
        .filter_map(|f| {
            let p = &f["properties"];
            let name = p["name"].as_str()?.trim().to_string();
            let g = &f["geometry"];
            let rings: Vec<&serde_json::Value> = match g["type"].as_str()? {
                "Polygon" => vec![g["coordinates"].as_array()?.first()?],
                "MultiPolygon" => g["coordinates"]
                    .as_array()?
                    .iter()
                    .filter_map(|poly| poly.as_array()?.first())
                    .collect(),
                _ => return None,
            };
            let ring = rings
                .into_iter()
                .max_by_key(|r| r.as_array().map_or(0, |a| a.len()))?
                .as_array()?;
            let n = ring.len() as f64;
            let (mut lat, mut x, mut y) = (0.0, 0.0, 0.0);
            for q in ring {
                let lo = q[0].as_f64()?.to_radians();
                lat += q[1].as_f64()?;
                x += lo.cos();
                y += lo.sin();
            }
            Some(Place {
                name,
                lat: lat / n,
                lon: y.atan2(x).to_degrees(),
                rank: p["min_label"]
                    .as_f64()
                    .or(p["scalerank"].as_f64())
                    .unwrap_or(5.0),
                kind: p["featurecla"].as_str().unwrap_or("sea").to_string(),
            })
        })
        .filter(|p| !p.name.is_empty())
        .collect())
}

/// NDBC `latest_obs.txt`: one line per station, "MM" for missing values.
pub fn parse_buoys(text: &str) -> Vec<Buoy> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 19 {
                return None;
            }
            let num = |i: usize| f.get(i).and_then(|s| s.parse::<f64>().ok());
            Some(Buoy {
                id: f[0].to_string(),
                lat: num(1)?,
                lon: num(2)?,
                time: format!("{}-{}-{} {}:{} UTC", f[3], f[4], f[5], f[6], f[7]),
                wind_from: num(8),
                wind: num(9),
                wave_height: num(11),
                period: num(12),
                wave_from: num(14),
                water_temp: num(18),
            })
        })
        .collect()
}

pub fn parse_obis(text: &str, group: usize) -> Result<Vec<Sighting>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(v["results"]
        .as_array()
        .ok_or("no OBIS results")?
        .iter()
        .filter_map(|r| {
            Some(Sighting {
                species: r["scientificName"].as_str()?.to_string(),
                common: r["vernacularName"].as_str().unwrap_or("").to_string(),
                lat: r["decimalLatitude"].as_f64()?,
                lon: r["decimalLongitude"].as_f64()?,
                date: r["eventDate"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(10)
                    .collect(),
                group,
            })
        })
        .collect())
}

enum Loaded {
    Lanes(Result<Vec<Lane>, String>),
    Ports(Result<Vec<Place>, String>),
    Seas(Result<Vec<Place>, String>),
    Buoys(Result<Vec<Buoy>, String>),
    Life([f64; 4], Result<Vec<Sighting>, String>),
    Waves([f64; 4], Result<pelagic::metocean::WaveField, String>),
    Quakes(Result<Vec<Quake>, String>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Picked {
    Ship(u64),
    Buoy(usize),
    Port(usize),
    Life(usize),
    Fleet(usize),
    Quake(usize),
}

/// Toggles, data and live feeds for the marine layers.
pub(in crate::studio) struct Marine {
    pub lanes: bool,
    pub ports: bool,
    pub seas: bool,
    pub buoys: bool,
    pub ships: bool,
    pub life: bool,
    pub life_group: usize,
    pub fleet: bool,
    /// The 3D viewport shows live ships near the scene.
    pub viewport_ships: bool,
    /// Live earthquakes (USGS, magnitude 2.5+ in the past week).
    pub quakes: bool,
    quakes_data: Option<Arc<Vec<Quake>>>,
    quakes_at: Option<Instant>,
    pub predict_minutes: f64,
    lanes_data: Option<Arc<Vec<Lane>>>,
    ports_data: Option<Arc<Vec<Place>>>,
    seas_data: Option<Arc<Vec<Place>>>,
    buoys_data: Option<Arc<Vec<Buoy>>>,
    buoys_at: Option<Instant>,
    life_data: Option<(usize, [f64; 4], Arc<Vec<Sighting>>)>,
    regional: Option<([f64; 4], Arc<pelagic::metocean::WaveField>)>,
    loading: Vec<(&'static str, mpsc::Receiver<Loaded>)>,
    requested: HashMap<&'static str, Instant>,
    picture: ais::Picture,
    feeds: Option<Arc<AtomicBool>>,
    world_feed: Option<Arc<AtomicBool>>,
    feed_status: Arc<Mutex<String>>,
    world_status: Arc<Mutex<String>>,
    pub ais_key: String,
    vessels: Vec<Vessel>,
    fleet_saved: i64,
    form: (String, String, String, String, String, String),
    picked: Option<Picked>,
    pub status: String,
    last_view: Option<([f64; 4], Instant)>,
}

impl Default for Marine {
    fn default() -> Self {
        let vessels = std::fs::read_to_string(crate::project::data_dir().join("fleet.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let ais_key = std::fs::read_to_string(crate::project::data_dir().join("aisstream.key"))
            .map(|k| k.trim().to_string())
            .unwrap_or_default();
        Self {
            lanes: false,
            ports: false,
            seas: false,
            buoys: false,
            ships: false,
            life: false,
            life_group: 0,
            fleet: false,
            viewport_ships: false,
            quakes: false,
            quakes_data: None,
            quakes_at: None,
            predict_minutes: 30.0,
            lanes_data: None,
            ports_data: None,
            seas_data: None,
            buoys_data: None,
            buoys_at: None,
            life_data: None,
            regional: None,
            loading: vec![],
            requested: HashMap::new(),
            picture: Arc::new(Mutex::new(HashMap::new())),
            feeds: None,
            world_feed: None,
            feed_status: Arc::new(Mutex::new(String::new())),
            world_status: Arc::new(Mutex::new(String::new())),
            ais_key,
            vessels,
            fleet_saved: 0,
            form: Default::default(),
            picked: None,
            status: String::new(),
            last_view: None,
        }
    }
}

impl Marine {
    /// Starts or stops the live AIS feeds as the layers and the viewport
    /// need them.
    pub fn run_feeds(&mut self) {
        // Live AIS feeds run while ships (or fleet following) are on.
        let want_feeds = self.ships
            || self.viewport_ships
            || (self.fleet && self.vessels.iter().any(|v| v.mmsi != 0));
        if want_feeds && self.feeds.is_none() {
            let stop = Arc::new(AtomicBool::new(false));
            ais::digitraffic(self.picture.clone(), stop.clone(), self.feed_status.clone());
            self.feeds = Some(stop);
        }
        if want_feeds && self.world_feed.is_none() && !self.ais_key.is_empty() {
            let stop = Arc::new(AtomicBool::new(false));
            ais::aisstream(
                self.ais_key.clone(),
                self.picture.clone(),
                stop.clone(),
                self.world_status.clone(),
            );
            self.world_feed = Some(stop);
        }
        if !want_feeds {
            for stop in [self.feeds.take(), self.world_feed.take()]
                .into_iter()
                .flatten()
            {
                stop.store(true, Ordering::Relaxed);
            }
        }
    }
    /// Live ships within `km` of a place, positions as last reported.
    pub fn ships_near(&self, lat: f64, lon: f64, km: f64) -> Vec<ais::Ship> {
        let dlat = km / 111.0;
        let dlon = km / (111.0 * lat.to_radians().cos().max(0.05));
        self.picture
            .lock()
            .map(|p| {
                p.values()
                    .filter(|s| (s.lat - lat).abs() < dlat && (s.lon - lon).abs() < dlon)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Worldwide ship feed status (aisstream.io).
    pub fn world_text(&self) -> String {
        self.world_status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
    pub fn busy(&self) -> bool {
        !self.loading.is_empty()
    }
    fn spawn(&mut self, what: &'static str, job: impl FnOnce() -> Loaded + Send + 'static) {
        if self.loading.iter().any(|(w, _)| *w == what) {
            return;
        }
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(job());
        });
        self.loading.push((what, receive));
        self.requested.insert(what, Instant::now());
    }
    fn retry_ok(&self, what: &str) -> bool {
        self.requested
            .get(what)
            .is_none_or(|t| t.elapsed().as_secs() > 60)
    }
    /// Starts any downloads the switched-on layers need and collects
    /// finished ones.
    pub fn update(&mut self, view_box: Option<[f64; 4]>, height_km: f64, waves_on: bool) {
        let mut finished = vec![];
        self.loading.retain(|(_, rx)| match rx.try_recv() {
            Ok(v) => {
                finished.push(v);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for f in finished {
            match f {
                Loaded::Lanes(r) => match r {
                    Ok(v) => self.lanes_data = Some(Arc::new(v)),
                    Err(e) => self.status = format!("Shipping lanes unavailable: {e}"),
                },
                Loaded::Ports(r) => match r {
                    Ok(v) => self.ports_data = Some(Arc::new(v)),
                    Err(e) => self.status = format!("Ports unavailable: {e}"),
                },
                Loaded::Seas(r) => match r {
                    Ok(v) => self.seas_data = Some(Arc::new(v)),
                    Err(e) => self.status = format!("Sea names unavailable: {e}"),
                },
                Loaded::Buoys(r) => match r {
                    Ok(v) => {
                        self.status = format!("{} buoys reporting (NOAA NDBC)", v.len());
                        self.buoys_data = Some(Arc::new(v));
                        self.buoys_at = Some(Instant::now());
                    }
                    Err(e) => self.status = format!("Buoys unavailable: {e}"),
                },
                Loaded::Life(b, r) => match r {
                    Ok(v) => {
                        self.status = format!(
                            "{} {} records in view (OBIS)",
                            v.len(),
                            LIFE_GROUPS[self.life_group].0.to_lowercase()
                        );
                        self.life_data = Some((self.life_group, b, Arc::new(v)));
                    }
                    Err(e) => self.status = format!("Marine life unavailable: {e}"),
                },
                Loaded::Quakes(r) => match r {
                    Ok(v) => {
                        self.status =
                            format!("{} earthquakes M2.5+ in the past week (USGS)", v.len());
                        self.quakes_data = Some(Arc::new(v));
                        self.quakes_at = Some(Instant::now());
                    }
                    Err(e) => self.status = format!("Earthquakes unavailable: {e}"),
                },
                Loaded::Waves(b, r) => match r {
                    Ok(v) => self.regional = Some((b, Arc::new(v))),
                    Err(e) => self.status = format!("Regional waves unavailable: {e}"),
                },
            }
        }
        if self.lanes && self.lanes_data.is_none() && self.retry_ok("lanes") {
            self.spawn("lanes", || {
                Loaded::Lanes(
                    cached_text(LANES_URL, "shipping-lanes.geojson").and_then(|t| parse_lanes(&t)),
                )
            });
        }
        if self.ports && self.ports_data.is_none() && self.retry_ok("ports") {
            self.spawn("ports", || {
                Loaded::Ports(cached_text(PORTS_URL, "ports.geojson").and_then(|t| parse_ports(&t)))
            });
        }
        if self.seas && self.seas_data.is_none() && self.retry_ok("seas") {
            self.spawn("seas", || {
                Loaded::Seas(cached_text(SEAS_URL, "seas.geojson").and_then(|t| parse_seas(&t)))
            });
        }
        if self.buoys
            && self.buoys_at.is_none_or(|t| t.elapsed().as_secs() > 600)
            && self.retry_ok("buoys")
        {
            self.spawn("buoys", || {
                Loaded::Buoys(get_text(BUOYS_URL, 8 << 20).map(|t| parse_buoys(&t)))
            });
        }
        if self.quakes
            && self.quakes_at.is_none_or(|t| t.elapsed().as_secs() > 300)
            && self.retry_ok("quakes")
        {
            self.spawn("quakes", || {
                Loaded::Quakes(get_text(QUAKES_URL, 16 << 20).and_then(|t| parse_quakes(&t)))
            });
        }
        self.run_feeds();
        // View-dependent layers refresh once the view has settled.
        if let Some(b) = view_box {
            let moved = self
                .last_view
                .is_none_or(|(old, _)| old.iter().zip(&b).any(|(x, y)| (x - y).abs() > 0.02));
            if moved {
                self.last_view = Some((b, Instant::now()));
            }
            let settled = self
                .last_view
                .is_some_and(|(_, t)| t.elapsed().as_millis() > 900);
            let covers = |old: &[f64; 4]| {
                old[0] <= b[0] && old[1] <= b[1] && old[2] >= b[2] && old[3] >= b[3]
            };
            if settled && self.life && height_km < 2500.0 {
                let fresh = self
                    .life_data
                    .as_ref()
                    .is_some_and(|(g, old, _)| *g == self.life_group && covers(old));
                if !fresh && self.retry_ok_short("life") {
                    let group = self.life_group;
                    let wide = widen(b, 0.25);
                    self.spawn("life", move || {
                        let url = format!(
                            "https://api.obis.org/v3/occurrence?scientificname={}&geometry=POLYGON(({w:.3}%20{s:.3},{e:.3}%20{s:.3},{e:.3}%20{n:.3},{w:.3}%20{n:.3},{w:.3}%20{s:.3}))&size=1500&fields=decimalLatitude,decimalLongitude,scientificName,vernacularName,eventDate",
                            LIFE_GROUPS[group].1,
                            w = wide[1],
                            s = wide[0],
                            e = wide[3],
                            n = wide[2],
                        );
                        Loaded::Life(wide, get_text(&url, 32 << 20).and_then(|t| parse_obis(&t, group)))
                    });
                }
            }
            if settled && waves_on && height_km < 2500.0 {
                let fresh = self.regional.as_ref().is_some_and(|(old, _)| covers(old));
                if !fresh && self.retry_ok_short("waves") {
                    let wide = widen(b, 0.1);
                    self.spawn("waves", move || {
                        let mut points = vec![];
                        for j in 0..10 {
                            for i in 0..10 {
                                points.push((
                                    wide[0] + (wide[2] - wide[0]) * (j as f64 + 0.5) / 10.0,
                                    wide[1] + (wide[3] - wide[1]) * (i as f64 + 0.5) / 10.0,
                                ));
                            }
                        }
                        Loaded::Waves(wide, crate::forecast::region(&points))
                    });
                }
            }
        }
        // Fleet vessels that follow an MMSI take positions from live AIS.
        if self.fleet {
            let picture = self.picture.lock().unwrap();
            let mut changed = false;
            for v in self.vessels.iter_mut().filter(|v| v.mmsi != 0) {
                if let Some(s) = picture.get(&v.mmsi)
                    && s.time > v.reported
                {
                    v.lat = s.lat;
                    v.lon = s.lon;
                    v.sog = s.sog;
                    v.cog = s.cog;
                    v.reported = s.time;
                    v.track.push((s.lat, s.lon, s.time));
                    changed = true;
                }
            }
            drop(picture);
            if changed && now() - self.fleet_saved > 60 {
                self.save_fleet();
            }
        }
    }
    fn retry_ok_short(&self, what: &str) -> bool {
        self.requested
            .get(what)
            .is_none_or(|t| t.elapsed().as_secs() > 4)
    }
    fn save_fleet(&mut self) {
        for v in &mut self.vessels {
            if v.track.len() > 2000 {
                v.track.drain(..v.track.len() - 2000);
            }
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.vessels) {
            let path = crate::project::data_dir().join("fleet.json");
            let tmp = path.with_extension("part");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        self.fleet_saved = now();
    }
    /// Adds a fleet vessel without saving it (scripted captures and tests).
    pub fn add_scripted(&mut self, lat: f64, lon: f64, sog: f64, cog: f64, name: &str) {
        let t = now();
        self.vessels.push(Vessel {
            name: name.trim().to_string(),
            mmsi: 0,
            kind: String::new(),
            lat,
            lon,
            sog,
            cog,
            reported: t,
            track: vec![
                ais::dead_reckon(lat, lon, sog, cog + 180.0, 20.0),
                ais::dead_reckon(lat, lon, sog, cog + 170.0, 10.0),
                (lat, lon),
            ]
            .into_iter()
            .map(|(a, b)| (a, b, t))
            .collect(),
            notes: String::new(),
        });
        self.fleet = true;
    }
    pub fn feed_status(&self) -> String {
        let a = self.feed_status.lock().unwrap().clone();
        let b = self.world_status.lock().unwrap().clone();
        match (a.is_empty(), b.is_empty()) {
            (true, true) => String::new(),
            (false, true) => a,
            (true, false) => b,
            _ => format!("{a} · {b}"),
        }
    }
    /// Ships as displayed now: last report advanced by dead reckoning for
    /// up to ten minutes (so moving ships glide between reports).
    fn ships_now(&self) -> Vec<Ship> {
        let t = now();
        self.picture
            .lock()
            .unwrap()
            .values()
            .filter(|s| t - s.time < 3 * 3600 || s.time == 0)
            .map(|s| {
                let mut s = s.clone();
                let age = ((t - s.time) as f64 / 60.0).clamp(0.0, 10.0);
                if s.sog > 0.5 && s.time > 0 {
                    (s.lat, s.lon) = ais::dead_reckon(s.lat, s.lon, s.sog, s.cog, age);
                }
                s
            })
            .collect()
    }
    /// Estimated position of a fleet vessel now (dead reckoning from its
    /// last report, at most 12 hours).
    fn vessel_now(v: &Vessel) -> (f64, f64) {
        let minutes = ((now() - v.reported) as f64 / 60.0).clamp(0.0, 720.0);
        ais::dead_reckon(v.lat, v.lon, v.sog, v.cog, minutes)
    }
    /// Closest approaches between fleet vessels and live ships within the
    /// prediction horizon: (fleet vessel, other name, CPA nm, TCPA min).
    pub fn warnings(&self) -> Vec<(String, String, f64, f64)> {
        let ships = self.ships_now();
        let mut out = vec![];
        for (i, v) in self.vessels.iter().enumerate() {
            let (lat, lon) = Self::vessel_now(v);
            let own = (lat, lon, v.sog, v.cog);
            let others = ships
                .iter()
                .filter(|s| s.mmsi != v.mmsi)
                .map(|s| {
                    (
                        if s.name.is_empty() {
                            format!("MMSI {}", s.mmsi)
                        } else {
                            s.name.clone()
                        },
                        (s.lat, s.lon, s.sog, s.cog),
                    )
                })
                .chain(
                    self.vessels
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .map(|(_, o)| {
                            let (la, lo) = Self::vessel_now(o);
                            (o.name.clone(), (la, lo, o.sog, o.cog))
                        }),
                );
            for (name, other) in others {
                if (other.0 - lat).abs() > 0.6 || (other.1 - lon).abs() > 0.6 {
                    continue;
                }
                let (d, t) = ais::cpa(own, other);
                if d < 1.0 && (0.0..=self.predict_minutes).contains(&t) {
                    out.push((v.name.clone(), name, d, t));
                }
            }
        }
        out.sort_by(|a, b| a.3.total_cmp(&b.3));
        out
    }
}

fn widen(b: [f64; 4], f: f64) -> [f64; 4] {
    let (dl, dn) = ((b[2] - b[0]) * f, (b[3] - b[1]) * f);
    [
        (b[0] - dl).max(-85.0),
        (b[1] - dn).max(-180.0),
        (b[2] + dl).min(85.0),
        (b[3] + dn).min(180.0),
    ]
}

fn ship_colour(kind: u16) -> Color32 {
    match ais::category(kind) {
        "Cargo" => Color32::from_rgb(120, 200, 110),
        "Tanker" => Color32::from_rgb(230, 80, 70),
        "Passenger" => Color32::from_rgb(80, 150, 255),
        "Fishing" => Color32::from_rgb(255, 170, 60),
        "Tug" | "Pilot" | "Port tender" | "Dredger" | "Diving" => Color32::from_rgb(70, 220, 220),
        "Search and rescue" | "Law enforcement" | "Medical" => Color32::from_rgb(255, 90, 200),
        "Sailing" | "Pleasure craft" => Color32::from_rgb(200, 120, 255),
        "High-speed craft" => Color32::from_rgb(255, 240, 100),
        _ => Color32::from_rgb(190, 196, 206),
    }
}

impl Globe {
    fn at(view: &View, lat: f64, lon: f64) -> Option<Pos2> {
        let p = unit(lat, lon);
        view.project([p[0] * R, p[1] * R, p[2] * R])
            .filter(|(_, f)| *f)
            .map(|(q, _)| q)
    }

    /// Geographic box of the view (south, west, north, east) when zoomed in
    /// enough for regional layers.
    fn view_box(view: &View) -> Option<[f64; 4]> {
        let r = view.rect;
        let corners: Vec<(f64, f64)> = [
            r.left_top(),
            r.right_top(),
            r.left_bottom(),
            r.right_bottom(),
            r.center(),
            r.center_top(),
            r.center_bottom(),
        ]
        .iter()
        .filter_map(|p| view.pick(*p))
        .collect();
        if corners.len() < 5 {
            return None;
        }
        let (mut s, mut w, mut n, mut e) = (90.0f64, 180.0f64, -90.0f64, -180.0f64);
        for (la, lo) in &corners {
            s = s.min(*la);
            n = n.max(*la);
            w = w.min(*lo);
            e = e.max(*lo);
        }
        (e - w < 180.0).then_some([s, w, n, e])
    }

    pub(super) fn update_marine(&mut self, view: &View) {
        let b = Self::view_box(view);
        let h = view.height();
        self.marine.update(b, h, self.sea.on);
    }

    /// Lanes, places, buoys, life, ships, fleet and regional waves.
    pub(super) fn draw_marine(&mut self, view: &View, painter: &egui::Painter) {
        let m = &self.marine;
        let height = view.height();
        let mut placed: Vec<Rect> = vec![];
        if let Some((_, field)) = &m.regional
            && self.sea.on
            && height < 2500.0
        {
            let hour = field
                .times
                .iter()
                .position(|t| *t >= crate::forecast::now_hour())
                .unwrap_or(0);
            for p in &field.points {
                let (Some(Some(hs)), Some(Some(from))) =
                    (p.height.get(hour), p.direction.get(hour))
                else {
                    continue;
                };
                let Some(q) = Self::at(view, p.latitude, p.longitude) else {
                    continue;
                };
                let towards = (*from as f64 + 180.0).to_radians() - self.heading.to_radians();
                let d = Vec2::new(towards.sin() as f32, -(towards.cos() as f32));
                let colour = super::sea_layer::height_colour(*hs);
                let len = 9.0 + hs.min(6.0) * 2.0;
                let a = q - d * len;
                let b = q + d * len;
                let stroke = Stroke::new(2.0, colour);
                painter.line_segment([a, b], stroke);
                let side = Vec2::new(-d.y, d.x);
                painter.line_segment([b, b - d * 6.0 + side * 4.0], stroke);
                painter.line_segment([b, b - d * 6.0 - side * 4.0], stroke);
                if height < 900.0 {
                    let period = p.period.get(hour).copied().flatten().unwrap_or(0.0);
                    put_label(
                        painter,
                        view,
                        &mut placed,
                        q + Vec2::new(4.0, 8.0),
                        format!("{hs:.1} m {period:.0} s"),
                        Color32::from_white_alpha(210),
                    );
                }
            }
        }
        if m.lanes
            && let Some(lanes) = &m.lanes_data
        {
            for lane in lanes.iter() {
                if lane.rank == 2 && height > 6000.0 {
                    continue;
                }
                let (w, c) = match lane.rank {
                    0 => (2.2, Color32::from_rgba_unmultiplied(255, 205, 90, 210)),
                    1 => (1.5, Color32::from_rgba_unmultiplied(255, 205, 90, 150)),
                    _ => (1.0, Color32::from_rgba_unmultiplied(255, 205, 90, 100)),
                };
                let mut run: Vec<Pos2> = vec![];
                for (la, lo) in &lane.points {
                    match Self::at(view, *la, *lo) {
                        Some(q) => run.push(q),
                        None => {
                            if run.len() > 1 {
                                painter.add(egui::Shape::line(
                                    std::mem::take(&mut run),
                                    Stroke::new(w, c),
                                ));
                            }
                            run.clear();
                        }
                    }
                }
                if run.len() > 1 {
                    painter.add(egui::Shape::line(run, Stroke::new(w, c)));
                }
            }
        }
        let zoom = (40_000.0 / height.max(1.0)).log2();
        if m.seas
            && let Some(seas) = &m.seas_data
        {
            let mut list: Vec<&Place> = seas.iter().filter(|p| p.rank <= zoom + 1.5).collect();
            list.sort_by(|a, b| a.rank.total_cmp(&b.rank));
            for p in list.into_iter().take(120) {
                if let Some(q) = Self::at(view, p.lat, p.lon) {
                    let galley = painter.layout_no_wrap(
                        p.name.clone(),
                        egui::FontId::proportional(if p.rank < 2.0 { 15.0 } else { 12.0 }),
                        Color32::from_rgba_unmultiplied(170, 215, 255, 200),
                    );
                    let rect = Rect::from_center_size(q, galley.size());
                    if view.rect.contains_rect(rect) && !placed.iter().any(|r| r.intersects(rect)) {
                        placed.push(rect);
                        painter.galley(rect.min, galley, Color32::WHITE);
                    }
                }
            }
        }
        if m.ports
            && let Some(ports) = &m.ports_data
        {
            let max_rank = if height > 8000.0 {
                3.0
            } else if height > 3000.0 {
                5.0
            } else if height > 800.0 {
                7.0
            } else {
                10.0
            };
            for p in ports.iter().filter(|p| p.rank <= max_rank) {
                if let Some(q) = Self::at(view, p.lat, p.lon) {
                    painter.rect_filled(
                        Rect::from_center_size(q, Vec2::splat(6.0)),
                        1.0,
                        Color32::from_rgb(70, 200, 255),
                    );
                    painter.rect_stroke(
                        Rect::from_center_size(q, Vec2::splat(6.0)),
                        1.0,
                        Stroke::new(1.0, Color32::BLACK),
                        egui::StrokeKind::Outside,
                    );
                    if height < 2500.0 && (p.rank < max_rank - 1.0 || height < 800.0) {
                        put_label(
                            painter,
                            view,
                            &mut placed,
                            q,
                            p.name.clone(),
                            Color32::from_rgb(160, 225, 255),
                        );
                    }
                }
            }
        }
        if m.buoys
            && let Some(buoys) = &m.buoys_data
        {
            for b in buoys.iter() {
                let Some(q) = Self::at(view, b.lat, b.lon) else {
                    continue;
                };
                let colour = b.wave_height.map_or(Color32::from_gray(200), |h| {
                    super::sea_layer::height_colour(h as f32)
                });
                painter.circle_filled(q, 4.0, colour);
                painter.circle_stroke(q, 4.0, Stroke::new(1.0, Color32::BLACK));
                if height < 3000.0
                    && let Some(h) = b.wave_height
                {
                    put_label(
                        painter,
                        view,
                        &mut placed,
                        q,
                        format!("{} · {h:.1} m", b.id),
                        Color32::WHITE,
                    );
                }
            }
        }
        if m.quakes
            && let Some(quakes) = &m.quakes_data
        {
            let t = now();
            for q in quakes.iter() {
                let Some(p) = Self::at(view, q.lat, q.lon) else {
                    continue;
                };
                let age_h = (t - q.time) as f64 / 3600.0;
                let colour = if age_h < 1.0 {
                    Color32::from_rgb(255, 40, 40)
                } else if age_h < 24.0 {
                    Color32::from_rgb(255, 140, 40)
                } else {
                    Color32::from_rgb(255, 220, 80)
                };
                let r = (q.magnitude as f32 - 1.5).max(1.0).powf(1.6) * 1.6;
                painter.circle(p, r, colour.gamma_multiply(0.35), Stroke::new(1.5, colour));
                if q.magnitude >= 5.5 || (height < 2500.0 && q.magnitude >= 4.0) {
                    put_label(
                        painter,
                        view,
                        &mut placed,
                        p + Vec2::new(r, 0.0),
                        format!("M{:.1}", q.magnitude),
                        colour,
                    );
                }
            }
        }
        if m.life
            && let Some((group, _, list)) = &m.life_data
        {
            let c = LIFE_GROUPS[*group].2;
            let colour = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 200);
            for s in list.iter() {
                if let Some(q) = Self::at(view, s.lat, s.lon) {
                    painter.circle_filled(q, 2.6, colour);
                }
            }
        }
        if m.ships {
            let ships = m.ships_now();
            let names = height < 250.0;
            let mut drawn = 0;
            for s in &ships {
                let Some(q) = Self::at(view, s.lat, s.lon) else {
                    continue;
                };
                if !view.rect.contains(q) {
                    continue;
                }
                drawn += 1;
                if drawn > 8000 {
                    break;
                }
                let course = s.heading.unwrap_or(s.cog).to_radians() - self.heading.to_radians();
                let d = Vec2::new(course.sin() as f32, -(course.cos() as f32));
                let side = Vec2::new(-d.y, d.x);
                let size = if height < 60.0 { 7.0 } else { 5.0 };
                let colour = ship_colour(s.kind);
                if s.sog > 0.5 {
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            q + d * size,
                            q - d * size * 0.7 + side * size * 0.55,
                            q - d * size * 0.7 - side * size * 0.55,
                        ],
                        colour,
                        Stroke::new(0.8, Color32::BLACK),
                    ));
                } else {
                    painter.circle_filled(q, size * 0.45, colour);
                }
                if names && !s.name.is_empty() {
                    put_label(
                        painter,
                        view,
                        &mut placed,
                        q,
                        s.name.clone(),
                        Color32::from_white_alpha(220),
                    );
                }
            }
        }
        // Selection extras: track history and prediction.
        let predict = m.predict_minutes;
        let picked = m.picked;
        if let Some(Picked::Ship(mmsi)) = picked
            && let Some(s) = m.picture.lock().unwrap().get(&mmsi).cloned()
        {
            Self::draw_track(view, painter, &s.track, Color32::from_rgb(255, 255, 255));
            Self::draw_prediction(
                view,
                painter,
                (s.lat, s.lon, s.sog, s.cog),
                predict,
                Color32::from_rgb(255, 230, 120),
            );
        }
        if m.fleet {
            for (i, v) in m.vessels.iter().enumerate() {
                let (la, lo) = Marine::vessel_now(v);
                Self::draw_track(view, painter, &v.track, Color32::from_rgb(120, 255, 170));
                Self::draw_prediction(
                    view,
                    painter,
                    (la, lo, v.sog, v.cog),
                    predict,
                    Color32::from_rgb(120, 255, 170),
                );
                if let Some(q) = Self::at(view, la, lo) {
                    let estimated = v.mmsi == 0 && now() - v.reported > 300;
                    painter.circle_filled(
                        q,
                        6.0,
                        if estimated {
                            Color32::TRANSPARENT
                        } else {
                            Color32::from_rgb(40, 220, 130)
                        },
                    );
                    painter.circle_stroke(
                        q,
                        6.0,
                        Stroke::new(2.0, Color32::from_rgb(40, 220, 130)),
                    );
                    let mut text = v.name.clone();
                    if Some(Picked::Fleet(i)) == picked {
                        text = format!("▶ {text}");
                    }
                    put_label(
                        painter,
                        view,
                        &mut placed,
                        q,
                        text,
                        Color32::from_rgb(150, 255, 190),
                    );
                }
            }
        }
    }

    fn draw_track(
        view: &View,
        painter: &egui::Painter,
        track: &[(f64, f64, i64)],
        colour: Color32,
    ) {
        let points: Vec<Pos2> = track
            .iter()
            .filter_map(|p| Self::at(view, p.0, p.1))
            .collect();
        if points.len() > 1 {
            painter.add(egui::Shape::line(
                points,
                Stroke::new(1.5, colour.gamma_multiply(0.7)),
            ));
        }
    }

    /// Dead-reckoned path with ticks every ten minutes.
    fn draw_prediction(
        view: &View,
        painter: &egui::Painter,
        s: (f64, f64, f64, f64),
        minutes: f64,
        colour: Color32,
    ) {
        if s.2 < 0.3 || minutes <= 0.0 {
            return;
        }
        let steps = (minutes / 2.0).ceil().max(1.0) as usize;
        let mut last: Option<Pos2> = Self::at(view, s.0, s.1);
        for i in 1..=steps {
            let t = minutes * i as f64 / steps as f64;
            let (la, lo) = ais::dead_reckon(s.0, s.1, s.2, s.3, t);
            let q = Self::at(view, la, lo);
            if let (Some(a), Some(b)) = (last, q)
                && i % 2 == 0
            {
                painter.line_segment([a, b], Stroke::new(1.5, colour));
            }
            if let Some(b) = q
                && (t % 10.0).abs() < minutes / steps as f64 * 0.5
            {
                painter.circle_filled(b, 2.0, colour);
            }
            last = q;
        }
    }

    /// Picks the nearest marine object under a click.
    pub(super) fn pick_marine(&mut self, view: &View, at: Pos2) -> bool {
        let m = &self.marine;
        let mut best: Option<(f32, Picked)> = None;
        let mut consider = |q: Option<Pos2>, p: Picked| {
            if let Some(q) = q {
                let d = q.distance(at);
                if d < 10.0 && best.is_none_or(|b| d < b.0) {
                    best = Some((d, p));
                }
            }
        };
        if m.ships {
            for s in m.ships_now() {
                consider(Self::at(view, s.lat, s.lon), Picked::Ship(s.mmsi));
            }
        }
        if m.buoys
            && let Some(b) = &m.buoys_data
        {
            for (i, x) in b.iter().enumerate() {
                consider(Self::at(view, x.lat, x.lon), Picked::Buoy(i));
            }
        }
        if m.ports
            && let Some(p) = &m.ports_data
        {
            for (i, x) in p.iter().enumerate() {
                consider(Self::at(view, x.lat, x.lon), Picked::Port(i));
            }
        }
        if m.life
            && let Some((_, _, l)) = &m.life_data
        {
            for (i, x) in l.iter().enumerate() {
                consider(Self::at(view, x.lat, x.lon), Picked::Life(i));
            }
        }
        if m.quakes
            && let Some(q) = &m.quakes_data
        {
            for (i, x) in q.iter().enumerate() {
                consider(Self::at(view, x.lat, x.lon), Picked::Quake(i));
            }
        }
        if m.fleet {
            for (i, v) in m.vessels.iter().enumerate() {
                let (la, lo) = Marine::vessel_now(v);
                consider(Self::at(view, la, lo), Picked::Fleet(i));
            }
        }
        self.marine.picked = best.map(|b| b.1);
        best.is_some()
    }

    /// Details of the picked object.
    pub(super) fn marine_details(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(picked) = self.marine.picked else {
            return;
        };
        let mut open = true;
        let mut rows: Vec<(String, String)> = vec![];
        let mut title = String::new();
        let m = &self.marine;
        match picked {
            Picked::Ship(mmsi) => {
                if let Some(s) = m.picture.lock().unwrap().get(&mmsi) {
                    title = if s.name.is_empty() {
                        format!("MMSI {mmsi}")
                    } else {
                        s.name.clone()
                    };
                    rows = vec![
                        ("MMSI".into(), mmsi.to_string()),
                        (
                            "Type".into(),
                            format!("{} ({})", ais::category(s.kind), s.kind),
                        ),
                        ("Status".into(), ais::status_text(s.status).into()),
                        (
                            "Speed / course".into(),
                            format!("{:.1} kn · {:.0}°", s.sog, s.cog),
                        ),
                        ("Position".into(), format!("{:.4}°, {:.4}°", s.lat, s.lon)),
                        (
                            "Destination".into(),
                            if s.destination.is_empty() {
                                "—".into()
                            } else {
                                s.destination.clone()
                            },
                        ),
                        (
                            "Size".into(),
                            s.size
                                .map_or("—".into(), |(l, b)| format!("{l:.0} × {b:.0} m")),
                        ),
                        (
                            "Reported".into(),
                            if s.time > 0 {
                                format!("{} min ago", (now() - s.time) / 60)
                            } else {
                                "—".into()
                            },
                        ),
                        ("Source".into(), s.source.into()),
                        ("People on board".into(), "not in public AIS".into()),
                    ];
                }
            }
            Picked::Buoy(i) => {
                if let Some(b) = m.buoys_data.as_ref().and_then(|v| v.get(i)) {
                    title = format!("Buoy {}", b.id);
                    let f = |v: Option<f64>, unit: &str| {
                        v.map_or("—".into(), |x| format!("{x:.1} {unit}"))
                    };
                    rows = vec![
                        ("Wave height".into(), f(b.wave_height, "m")),
                        ("Dominant period".into(), f(b.period, "s")),
                        ("Waves from".into(), f(b.wave_from, "°")),
                        (
                            "Wind".into(),
                            format!("{} from {}", f(b.wind, "m/s"), f(b.wind_from, "°")),
                        ),
                        ("Water temperature".into(), f(b.water_temp, "°C")),
                        ("Observed".into(), b.time.clone()),
                        ("Source".into(), "NOAA NDBC".into()),
                    ];
                }
            }
            Picked::Port(i) => {
                if let Some(p) = m.ports_data.as_ref().and_then(|v| v.get(i)) {
                    title = p.name.clone();
                    rows = vec![
                        ("Port".into(), format!("{:.4}°, {:.4}°", p.lat, p.lon)),
                        ("Source".into(), "Natural Earth".into()),
                    ];
                }
            }
            Picked::Life(i) => {
                if let Some((g, _, l)) = &m.life_data
                    && let Some(s) = l.get(i)
                {
                    title = if s.common.is_empty() {
                        s.species.clone()
                    } else {
                        s.common.clone()
                    };
                    rows = vec![
                        ("Species".into(), s.species.clone()),
                        ("Group".into(), LIFE_GROUPS[*g].0.into()),
                        (
                            "Recorded".into(),
                            if s.date.is_empty() {
                                "—".into()
                            } else {
                                s.date.clone()
                            },
                        ),
                        ("Position".into(), format!("{:.4}°, {:.4}°", s.lat, s.lon)),
                        (
                            "Source".into(),
                            "OBIS occurrence record (not a live position)".into(),
                        ),
                    ];
                }
            }
            Picked::Quake(i) => {
                if let Some(q) = m.quakes_data.as_ref().and_then(|v| v.get(i)) {
                    title = format!("M{:.1} earthquake", q.magnitude);
                    rows = vec![
                        ("Where".into(), q.place.clone()),
                        ("Depth".into(), format!("{:.0} km", q.depth_km)),
                        (
                            "Time".into(),
                            format!(
                                "{} UTC ({} h ago)",
                                pelagic::metocean::format_utc(q.time),
                                (now() - q.time) / 3600
                            ),
                        ),
                        (
                            "Tsunami flag".into(),
                            if q.tsunami {
                                "yes — check official warnings".into()
                            } else {
                                "no".into()
                            },
                        ),
                        (
                            "Alert level".into(),
                            if q.alert.is_empty() {
                                "—".into()
                            } else {
                                q.alert.clone()
                            },
                        ),
                        ("Status".into(), q.status.clone()),
                        ("Source".into(), "USGS Earthquake Hazards Program".into()),
                    ];
                }
            }
            Picked::Fleet(i) => {
                if let Some(v) = m.vessels.get(i) {
                    title = v.name.clone();
                    let (la, lo) = Marine::vessel_now(v);
                    rows = vec![
                        ("Estimated now".into(), format!("{la:.4}°, {lo:.4}°")),
                        (
                            "Speed / course".into(),
                            format!("{:.1} kn · {:.0}°", v.sog, v.cog),
                        ),
                        (
                            "Last report".into(),
                            format!("{} min ago", (now() - v.reported) / 60),
                        ),
                        ("Track points".into(), v.track.len().to_string()),
                        (
                            "Following".into(),
                            if v.mmsi == 0 {
                                "manual reports".into()
                            } else {
                                format!("AIS MMSI {}", v.mmsi)
                            },
                        ),
                    ];
                }
            }
        }
        if title.is_empty() {
            self.marine.picked = None;
            return;
        }
        egui::Window::new(RichText::new(title).strong())
            .id(egui::Id::NULL.with("marine-details"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(rect.right_top() + Vec2::new(-330.0, 50.0))
            .show(ctx, |ui| {
                egui::Grid::new("marine-detail-grid")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (k, v) in rows {
                            ui.label(RichText::new(k).color(theme::MUTED));
                            ui.label(v);
                            ui.end_row();
                        }
                    });
            });
        if !open {
            self.marine.picked = None;
        }
    }

    /// The "My fleet" window: add vessels, report positions, follow by MMSI,
    /// predictions and closest-approach warnings.
    pub(super) fn fleet_window(&mut self, ctx: &egui::Context, rect: Rect) {
        if !self.marine.fleet {
            return;
        }
        let mut open = true;
        let (centre_lat, centre_lon) = (self.lat, self.lon);
        let mut go: Option<(f64, f64)> = None;
        egui::Window::new("My fleet")
            .id(egui::Id::NULL.with("fleet-window"))
            .open(&mut open)
            .default_pos(rect.left_top() + Vec2::new(12.0, 60.0))
            .default_width(360.0)
            .show(ctx, |ui| {
                let m = &mut self.marine;
                ui.label(RichText::new("Your own vessels: manual position reports or live AIS by MMSI. Predictions are dead reckoning on the last course and speed.").small().color(theme::MUTED));
                ui.add(egui::Slider::new(&mut m.predict_minutes, 0.0..=240.0).text("prediction (min)").step_by(5.0));
                let mut remove = None;
                let mut report = None;
                for (i, v) in m.vessels.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&v.name).strong());
                        ui.label(RichText::new(format!("{:.1} kn {:.0}°", v.sog, v.cog)).small());
                        if ui.small_button("Go").clicked() {
                            go = Some(Marine::vessel_now(v));
                        }
                        if v.mmsi == 0 && ui.small_button("Report at view centre").on_hover_text("Record a position report at the map centre").clicked() {
                            report = Some(i);
                        }
                        if ui.small_button("✕").on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                let mut changed = false;
                if let Some(i) = report {
                    let v = &mut m.vessels[i];
                    let t = now();
                    // Course and speed from the previous report.
                    let minutes = (t - v.reported) as f64 / 60.0;
                    if minutes > 0.5 {
                        let k = v.lat.to_radians().cos();
                        let dx = (centre_lon - v.lon) * 60.0 * k;
                        let dy = (centre_lat - v.lat) * 60.0;
                        let dist = dx.hypot(dy);
                        v.sog = (dist / (minutes / 60.0)).min(60.0);
                        v.cog = dx.atan2(dy).to_degrees().rem_euclid(360.0);
                    }
                    v.lat = centre_lat;
                    v.lon = centre_lon;
                    v.reported = t;
                    v.track.push((centre_lat, centre_lon, t));
                    changed = true;
                }
                if let Some(i) = remove {
                    m.vessels.remove(i);
                    m.picked = None;
                    changed = true;
                }
                ui.separator();
                ui.label(RichText::new("Add vessel").strong());
                let f = &mut m.form;
                egui::Grid::new("fleet-form").num_columns(2).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut f.0);
                    ui.end_row();
                    ui.label("MMSI (optional)");
                    ui.text_edit_singleline(&mut f.1);
                    ui.end_row();
                    ui.label("Latitude, longitude");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut f.2).desired_width(70.0));
                        ui.add(egui::TextEdit::singleline(&mut f.3).desired_width(70.0));
                        if ui.small_button("view centre").clicked() {
                            f.2 = format!("{centre_lat:.5}");
                            f.3 = format!("{centre_lon:.5}");
                        }
                    });
                    ui.end_row();
                    ui.label("Speed kn, course °");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut f.4).desired_width(70.0));
                        ui.add(egui::TextEdit::singleline(&mut f.5).desired_width(70.0));
                    });
                    ui.end_row();
                });
                if ui.button("Add").clicked() {
                    let num = |s: &str| s.trim().parse::<f64>().ok();
                    let lat = num(&f.2).unwrap_or(centre_lat);
                    let lon = num(&f.3).unwrap_or(centre_lon);
                    if f.0.trim().is_empty() || lat.abs() > 90.0 || lon.abs() > 180.0 {
                        m.status = "Give the vessel a name and a valid position.".into();
                    } else {
                        let t = now();
                        m.vessels.push(Vessel {
                            name: f.0.trim().chars().take(60).collect(),
                            mmsi: f.1.trim().parse().unwrap_or(0),
                            kind: String::new(),
                            lat,
                            lon,
                            sog: num(&f.4).unwrap_or(0.0).clamp(0.0, 60.0),
                            cog: num(&f.5).unwrap_or(0.0).rem_euclid(360.0),
                            reported: t,
                            track: vec![(lat, lon, t)],
                            notes: String::new(),
                        });
                        *f = Default::default();
                        changed = true;
                    }
                }
                let warnings = m.warnings();
                if !warnings.is_empty() {
                    ui.separator();
                    ui.label(RichText::new("Closest-approach warnings (< 1 nm)").strong().color(Color32::from_rgb(255, 170, 80)));
                    for (own, other, d, t) in warnings.iter().take(12) {
                        ui.label(format!("{own} – {other}: {d:.2} nm in {t:.0} min"));
                    }
                }
                if changed {
                    m.save_fleet();
                }
            });
        if let Some((la, lo)) = go {
            self.lat = la;
            self.lon = lo;
            self.altitude = self.altitude.min(40.0);
        }
        if !open {
            self.marine.fleet = false;
        }
    }

    /// Adds a fleet vessel at a map point (context menu).
    pub(super) fn add_vessel_at(&mut self, lat: f64, lon: f64) {
        let m = &mut self.marine;
        let t = now();
        let n = m.vessels.len() + 1;
        m.vessels.push(Vessel {
            name: format!("Vessel {n}"),
            mmsi: 0,
            kind: String::new(),
            lat,
            lon,
            sog: 0.0,
            cog: 0.0,
            reported: t,
            track: vec![(lat, lon, t)],
            notes: String::new(),
        });
        m.fleet = true;
        m.save_fleet();
    }

    /// The Layers menu: every overlay can be switched on or off.
    pub(super) fn layers_menu(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Seabed and habitat")
                .small()
                .color(theme::MUTED),
        );
        ui.checkbox(&mut self.reefs, "Coral reefs (UNEP-WCMC)");
        ui.checkbox(&mut self.seagrass, "Seagrass (UNEP-WCMC)");
        if ui
            .checkbox(&mut self.features_on, "Undersea feature names (GEBCO)")
            .changed()
            && self.features_on
        {
            self.load_features();
        }
        ui.checkbox(&mut self.islands, "Islands (Global Islands)");
        ui.separator();
        ui.label(RichText::new("Navigation").small().color(theme::MUTED));
        ui.checkbox(&mut self.marine.lanes, "Shipping lanes");
        ui.checkbox(&mut self.marine.ports, "Ports");
        ui.checkbox(
            &mut self.seamarks,
            "Lighthouses, buoys, harbours, traffic schemes (OpenSeaMap, zoom in)",
        );
        ui.checkbox(
            &mut self.boundaries,
            "Maritime boundaries / EEZ (Marine Regions)",
        );
        ui.checkbox(&mut self.marine.seas, "Sea and ocean names");
        ui.separator();
        ui.label(RichText::new("Live").small().color(theme::MUTED));
        ui.checkbox(&mut self.sea.on, "Live waves (zoom in for a finer grid)");
        ui.checkbox(&mut self.marine.buoys, "Wave buoys (NOAA NDBC)");
        ui.checkbox(&mut self.marine.quakes, "Earthquakes (USGS, live)");
        ui.checkbox(&mut self.sst, "Sea surface temperature (GHRSST, daily)");
        ui.checkbox(&mut self.marine.ships, "Ships (AIS)");
        ui.horizontal(|ui| {
            ui.label(RichText::new("aisstream.io key").small());
            let key = ui.add(
                egui::TextEdit::singleline(&mut self.marine.ais_key)
                    .password(true)
                    .desired_width(150.0)
                    .hint_text("for worldwide ships"),
            );
            if key.lost_focus() {
                let _ = std::fs::create_dir_all(crate::project::data_dir());
                let _ = std::fs::write(
                    crate::project::data_dir().join("aisstream.key"),
                    self.marine.ais_key.trim(),
                );
                if let Some(stop) = self.marine.world_feed.take() {
                    stop.store(true, Ordering::Relaxed);
                }
            }
        });
        ui.label(
            RichText::new("Without a key: Baltic Sea only (Digitraffic). Free keys: aisstream.io.")
                .small()
                .color(theme::MUTED),
        );
        ui.separator();
        ui.label(
            RichText::new("Marine life records")
                .small()
                .color(theme::MUTED),
        );
        ui.checkbox(
            &mut self.marine.life,
            "Show records in view (OBIS, zoom in)",
        );
        egui::ComboBox::from_id_salt("life-group")
            .selected_text(LIFE_GROUPS[self.marine.life_group].0)
            .show_ui(ui, |ui| {
                for (i, g) in LIFE_GROUPS.iter().enumerate() {
                    ui.selectable_value(&mut self.marine.life_group, i, g.0);
                }
            });
        ui.separator();
        ui.checkbox(
            &mut self.marine.fleet,
            "My fleet (tracks, predictions, warnings)",
        );
    }

    pub(super) fn marine_credits(&self) -> Vec<&'static str> {
        let m = &self.marine;
        let mut out = vec![];
        if m.lanes {
            out.push(LANES_CREDIT);
        }
        if m.ports || m.seas {
            out.push(NATURAL_EARTH_CREDIT);
        }
        if m.buoys {
            out.push(BUOY_CREDIT);
        }
        if m.quakes {
            out.push(QUAKE_CREDIT);
        }
        if m.life {
            out.push(OBIS_CREDIT);
        }
        if m.ships {
            out.push(ais::DIGITRAFFIC_CREDIT);
            if !m.ais_key.is_empty() {
                out.push(ais::AISSTREAM_CREDIT);
            }
        }
        if self.seamarks {
            out.push(SEAMARK_CREDIT);
        }
        if self.boundaries {
            out.push(BOUNDARY_CREDIT);
        }
        if self.islands {
            out.push(ISLANDS_CREDIT);
        }
        out
    }
}

/// Draws a label beside `q` unless it would overlap one already placed.
fn put_label(
    painter: &egui::Painter,
    view: &View,
    placed: &mut Vec<Rect>,
    q: Pos2,
    text: String,
    colour: Color32,
) -> bool {
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(10.5), colour);
    let rect = Rect::from_min_size(
        q + Vec2::new(6.0, -galley.size().y * 0.5 - 1.0),
        galley.size() + Vec2::new(4.0, 2.0),
    );
    if !view.rect.contains_rect(rect) || placed.iter().any(|r| r.intersects(rect)) {
        return false;
    }
    placed.push(rect);
    painter.rect_filled(rect, 3.0, Color32::from_black_alpha(110));
    painter.galley(rect.min + Vec2::new(2.0, 1.0), galley, colour);
    true
}

/// Nearest port in Natural Earth's list (downloaded once and cached).
pub fn nearest_port(lat: f64, lon: f64) -> Option<(String, f64, f64)> {
    let ports = cached_text(PORTS_URL, "ports.geojson")
        .ok()
        .and_then(|t| parse_ports(&t).ok())?;
    let k = lat.to_radians().cos();
    let d = |p: &Place| {
        let dlon = ((p.lon - lon + 540.0).rem_euclid(360.0) - 180.0) * k;
        (p.lat - lat).powi(2) + dlon * dlon
    };
    ports
        .into_iter()
        .min_by(|a, b| d(a).total_cmp(&d(b)))
        .map(|p| (p.name, p.lat, p.lon))
}
/// Species recorded within about 50 km of a place (OBIS checklists for bony
/// fish, sharks and rays, sea turtles, whales and dolphins, sirenians and
/// seals): (scientific name, family, records), most recorded first.
pub fn checklist_here(lat: f64, lon: f64) -> Vec<(String, String, u64)> {
    let (s, n, w, e) = (lat - 0.45, lat + 0.45, lon - 0.45, lon + 0.45);
    let geometry = format!(
        "POLYGON(({w:.3}%20{s:.3},{e:.3}%20{s:.3},{e:.3}%20{n:.3},{w:.3}%20{n:.3},{w:.3}%20{s:.3}))"
    );
    let mut out: Vec<(String, String, u64)> = vec![];
    for (taxon, size) in [
        ("Actinopterygii", 400),
        ("Elasmobranchii", 120),
        ("Testudines", 20),
        ("Cetacea", 60),
        ("Sirenia", 5),
        ("Pinnipedia", 20),
    ] {
        let url = format!(
            "https://api.obis.org/v3/checklist?scientificname={taxon}&geometry={geometry}&size={size}"
        );
        let Ok(text) = get_text(&url, 32 << 20) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        for r in v["results"].as_array().into_iter().flatten() {
            if r["taxonRank"].as_str() != Some("Species") {
                continue;
            }
            let (Some(name), Some(family)) = (r["scientificName"].as_str(), r["family"].as_str())
            else {
                continue;
            };
            out.push((
                name.to_string(),
                family.to_string(),
                r["records"].as_u64().unwrap_or(0),
            ));
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.2));
    out
}
pub const QUAKE_CREDIT: &str = "Earthquakes: USGS Earthquake Hazards Program (public domain)";
const QUAKES_URL: &str =
    "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/2.5_week.geojson";

/// One earthquake from the USGS live feed.
#[derive(Clone, Debug, PartialEq)]
pub struct Quake {
    pub magnitude: f64,
    pub place: String,
    pub lat: f64,
    pub lon: f64,
    pub depth_km: f64,
    /// Unix time (s).
    pub time: i64,
    pub tsunami: bool,
    pub alert: String,
    pub status: String,
}

/// USGS GeoJSON summary feed, newest first.
pub fn parse_quakes(text: &str) -> Result<Vec<Quake>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut out: Vec<Quake> = v["features"]
        .as_array()
        .ok_or("no earthquake features")?
        .iter()
        .filter_map(|f| {
            let p = &f["properties"];
            let c = f["geometry"]["coordinates"].as_array()?;
            Some(Quake {
                magnitude: p["mag"].as_f64()?,
                place: p["place"].as_str().unwrap_or("").to_string(),
                lon: c.first()?.as_f64()?,
                lat: c.get(1)?.as_f64()?,
                depth_km: c.get(2).and_then(|d| d.as_f64()).unwrap_or(0.0),
                time: p["time"].as_i64().unwrap_or(0) / 1000,
                tsunami: p["tsunami"].as_i64() == Some(1),
                alert: p["alert"].as_str().unwrap_or("").to_string(),
                status: p["status"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect();
    out.sort_by_key(|q| std::cmp::Reverse(q.time));
    Ok(out)
}

impl Globe {
    /// Seismic activity list: the strongest and latest earthquakes.
    pub(super) fn seismic_window(&mut self, ctx: &egui::Context, rect: Rect) {
        if !self.marine.quakes {
            return;
        }
        let Some(quakes) = self.marine.quakes_data.clone() else {
            return;
        };
        let mut open = true;
        let mut go = None;
        let t = now();
        egui::Window::new("Seismic activity")
            .id(egui::Id::NULL.with("seismic-window"))
            .open(&mut open)
            .default_pos(rect.left_bottom() + Vec2::new(12.0, -330.0))
            .default_width(330.0)
            .show(ctx, |ui| {
                let day = quakes.iter().filter(|q| t - q.time < 86_400).count();
                let strong = quakes.iter().filter(|q| q.magnitude >= 5.0).count();
                let tsunami = quakes.iter().filter(|q| q.tsunami).count();
                ui.label(format!(
                    "{} earthquakes M2.5+ this week · {day} in the last 24 h · {strong} of M5+ · {tsunami} with a tsunami flag",
                    quakes.len()
                ));
                ui.label(RichText::new("Red: last hour · orange: last day · yellow: this week. Circle size grows with magnitude.").small().color(theme::MUTED));
                ui.separator();
                let mut list: Vec<&Quake> = quakes.iter().collect();
                list.sort_by(|a, b| b.magnitude.total_cmp(&a.magnitude));
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    egui::Grid::new("quake-list").num_columns(3).striped(true).show(ui, |ui| {
                        for q in list.into_iter().take(40) {
                            ui.label(RichText::new(format!("M{:.1}", q.magnitude)).strong());
                            ui.label(format!(
                                "{} · {} h ago{}",
                                q.place,
                                (t - q.time) / 3600,
                                if q.tsunami { " · tsunami flag" } else { "" }
                            ));
                            if ui.small_button("Go").clicked() {
                                go = Some((q.lat, q.lon));
                            }
                            ui.end_row();
                        }
                    });
                });
            });
        if let Some((la, lo)) = go {
            self.lat = la;
            self.lon = lo;
            self.altitude = self.altitude.min(800.0);
        }
        if !open {
            self.marine.quakes = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_reference_and_live_layers() {
        let lanes = parse_lanes(r#"{"features":[{"properties":{"Type":"Major"},"geometry":{"type":"MultiLineString","coordinates":[[[0,0],[10,0]],[[179,1],[-179,1]]]}}]}"#).unwrap();
        assert_eq!(lanes.len(), 1, "antimeridian jump split off");
        assert_eq!(lanes[0].rank, 0);
        assert_eq!(lanes[0].points.len(), 11, "densified to ~1°");
        let ports = parse_ports(r#"{"features":[{"properties":{"name":"Rotterdam","scalerank":2},"geometry":{"type":"Point","coordinates":[4.3,51.9]}}]}"#).unwrap();
        assert_eq!((ports[0].name.as_str(), ports[0].lat), ("Rotterdam", 51.9));
        let seas = parse_seas(r#"{"features":[{"properties":{"name":"North Sea","min_label":3,"featurecla":"sea"},"geometry":{"type":"Polygon","coordinates":[[[0,50],[8,50],[8,60],[0,60],[0,50]]]}}]}"#).unwrap();
        assert!((seas[0].lat - 54.0).abs() < 1e-9 && seas[0].rank == 3.0);
        let buoys = parse_buoys(
            "#STN LAT LON\n#text\n41001  34.79 -72.63 2026 11 24 06 00 200  8.0  10.0  2.1  9.0  6.2 190 1015.0 MM 20.1 22.3 15.0 MM MM\n",
        );
        assert_eq!(buoys.len(), 1);
        assert_eq!(
            (buoys[0].wave_height, buoys[0].period, buoys[0].water_temp),
            (Some(2.1), Some(9.0), Some(22.3))
        );
        assert_eq!(buoys[0].wind, Some(8.0));
        let life = parse_obis(r#"{"results":[{"decimalLatitude":12.19,"decimalLongitude":-68.29,"scientificName":"Carcharhinus perezii","vernacularName":"Caribbean reef shark","eventDate":"2025-11-09T10:00"}]}"#, 0).unwrap();
        assert_eq!(
            (life[0].common.as_str(), life[0].date.as_str()),
            ("Caribbean reef shark", "2025-11-09")
        );
        assert_eq!(widen([0.0, 0.0, 10.0, 10.0], 0.1), [-1.0, -1.0, 11.0, 11.0]);
        let quakes = parse_quakes(r#"{"features":[{"properties":{"mag":4.1,"place":"Off Sumatra","time":1795500000000,"tsunami":0,"status":"reviewed"},"geometry":{"coordinates":[96.1,2.3,30.5]}},{"properties":{"mag":6.2,"place":"Tonga","time":1795508000000,"tsunami":1,"alert":"yellow","status":"automatic"},"geometry":{"coordinates":[-174.0,-20.1,10.0]}}]}"#).unwrap();
        assert_eq!(quakes.len(), 2);
        assert_eq!(quakes[0].place, "Tonga", "newest first");
        assert!(quakes[0].tsunami && quakes[0].alert == "yellow");
        assert_eq!((quakes[1].depth_km, quakes[1].time), (30.5, 1_795_500_000));
    }
}
