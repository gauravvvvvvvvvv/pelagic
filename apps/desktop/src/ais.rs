//! Live ship positions (AIS) from public services:
//!
//! * Fintraffic Digitraffic marine API (Finland and the Baltic Sea): open
//!   data, CC BY 4.0, no account needed.
//! * aisstream.io (worldwide terrestrial AIS): free, needs the user's own
//!   API key, streamed over a WebSocket.
//!
//! Public AIS carries identity, position, course, speed, navigational status,
//! type, size and destination. It does not carry the number of people on
//! board, and many naval and some government ships do not transmit.
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub const DIGITRAFFIC_CREDIT: &str = "Ships (Baltic): Fintraffic Digitraffic, CC BY 4.0";
pub const AISSTREAM_CREDIT: &str = "Ships (worldwide): aisstream.io";

/// One vessel as last reported.
#[derive(Clone, Debug, PartialEq)]
pub struct Ship {
    pub mmsi: u64,
    pub name: String,
    /// AIS ship and cargo type (0 = unknown).
    pub kind: u16,
    pub lat: f64,
    pub lon: f64,
    /// Speed over ground (knots) and course over ground (degrees true).
    pub sog: f64,
    pub cog: f64,
    /// True heading (degrees), if reported.
    pub heading: Option<f64>,
    pub status: u8,
    pub destination: String,
    /// Length and beam (m), if reported.
    pub size: Option<(f64, f64)>,
    /// Unix time of the position (s).
    pub time: i64,
    pub source: &'static str,
    /// Recent positions (lat, lon, unix time), oldest first.
    pub track: Vec<(f64, f64, i64)>,
}

/// Broad category of an AIS ship-type code, for colours and filters.
pub fn category(kind: u16) -> &'static str {
    match kind {
        30 => "Fishing",
        31 | 32 | 52 => "Tug",
        33 => "Dredger",
        34 => "Diving",
        35 => "Military",
        36 => "Sailing",
        37 => "Pleasure craft",
        40..=49 => "High-speed craft",
        50 => "Pilot",
        51 => "Search and rescue",
        53 => "Port tender",
        55 => "Law enforcement",
        58 => "Medical",
        60..=69 => "Passenger",
        70..=79 => "Cargo",
        80..=89 => "Tanker",
        _ => "Other",
    }
}

pub fn status_text(status: u8) -> &'static str {
    match status {
        0 => "Under way using engine",
        1 => "At anchor",
        2 => "Not under command",
        3 => "Restricted manoeuvrability",
        4 => "Constrained by draught",
        5 => "Moored",
        6 => "Aground",
        7 => "Engaged in fishing",
        8 => "Under way sailing",
        14 => "AIS-SART active",
        _ => "Not defined",
    }
}

/// Shared live picture, keyed by MMSI.
pub type Picture = Arc<Mutex<HashMap<u64, Ship>>>;

/// Merges a position report into the picture, extending its track.
pub fn merge(picture: &mut HashMap<u64, Ship>, mut ship: Ship) {
    if !(ship.lat.abs() <= 90.0 && ship.lon.abs() <= 180.0) || ship.mmsi == 0 {
        return;
    }
    if let Some(old) = picture.get(&ship.mmsi) {
        let mut track = old.track.clone();
        if track
            .last()
            .is_none_or(|p| (p.0 - ship.lat).abs() + (p.1 - ship.lon).abs() > 1e-5)
        {
            track.push((ship.lat, ship.lon, ship.time));
        }
        if track.len() > 240 {
            track.drain(..track.len() - 240);
        }
        ship.track = track;
        if ship.name.is_empty() {
            ship.name = old.name.clone();
        }
        if ship.kind == 0 {
            ship.kind = old.kind;
        }
        if ship.destination.is_empty() {
            ship.destination = old.destination.clone();
        }
        if ship.size.is_none() {
            ship.size = old.size;
        }
    } else {
        ship.track = vec![(ship.lat, ship.lon, ship.time)];
    }
    picture.insert(ship.mmsi, ship);
}

/// Parses Digitraffic `/api/ais/v1/locations` (GeoJSON).
pub fn parse_digitraffic(text: &str) -> Result<Vec<Ship>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let features = v["features"].as_array().ok_or("no AIS features")?;
    Ok(features
        .iter()
        .filter_map(|f| {
            let c = f["geometry"]["coordinates"].as_array()?;
            let p = &f["properties"];
            let heading = p["heading"].as_f64().filter(|h| *h < 360.0);
            Some(Ship {
                mmsi: f["mmsi"].as_u64().or(p["mmsi"].as_u64())?,
                name: String::new(),
                kind: 0,
                lon: c.first()?.as_f64()?,
                lat: c.get(1)?.as_f64()?,
                sog: p["sog"].as_f64().filter(|s| *s < 102.2).unwrap_or(0.0),
                cog: p["cog"].as_f64().filter(|c| *c < 360.0).unwrap_or(0.0),
                heading,
                status: p["navStat"].as_u64().unwrap_or(15) as u8,
                destination: String::new(),
                size: None,
                time: p["timestampExternal"].as_i64().map_or(0, |t| t / 1000),
                source: "Digitraffic",
                track: vec![],
            })
        })
        .collect())
}

/// Vessel metadata from Digitraffic `/api/ais/v1/vessels`: MMSI → (name,
/// type, destination, size).
/// Vessel name, ship type, destination and size by MMSI.
pub type VesselMeta = HashMap<u64, (String, u16, String, Option<(f64, f64)>)>;

pub fn parse_digitraffic_vessels(text: &str) -> Result<VesselMeta, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let list = v.as_array().ok_or("no vessel list")?;
    Ok(list
        .iter()
        .filter_map(|s| {
            let mmsi = s["mmsi"].as_u64()?;
            let ref_a = s["referencePointA"].as_f64().unwrap_or(0.0);
            let ref_b = s["referencePointB"].as_f64().unwrap_or(0.0);
            let ref_c = s["referencePointC"].as_f64().unwrap_or(0.0);
            let ref_d = s["referencePointD"].as_f64().unwrap_or(0.0);
            let size = (ref_a + ref_b > 0.0).then_some((ref_a + ref_b, ref_c + ref_d));
            Some((
                mmsi,
                (
                    s["name"].as_str().unwrap_or("").trim().to_string(),
                    s["shipType"].as_u64().unwrap_or(0) as u16,
                    s["destination"].as_str().unwrap_or("").trim().to_string(),
                    size,
                ),
            ))
        })
        .collect())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(40)))
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

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let mut response = agent
        .get(url)
        .header("Digitraffic-User", "Pelagic")
        .header("Accept-Encoding", "gzip")
        .call()
        .map_err(|e| e.to_string())?;
    response
        .body_mut()
        .with_config()
        .limit(64 << 20)
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// Background feed from Digitraffic: positions every minute, names and
/// types every half hour. Runs until `stop` is set.
pub fn digitraffic(picture: Picture, stop: Arc<AtomicBool>, status: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let agent = agent();
        let mut meta = HashMap::new();
        let mut meta_age: Option<std::time::Instant> = None;
        while !stop.load(Ordering::Relaxed) {
            if meta_age.is_none_or(|t| t.elapsed().as_secs() > 1800)
                && let Ok(text) = get_text(&agent, "https://meri.digitraffic.fi/api/ais/v1/vessels")
                && let Ok(m) = parse_digitraffic_vessels(&text)
            {
                meta = m;
                meta_age = Some(std::time::Instant::now());
            }
            match get_text(&agent, "https://meri.digitraffic.fi/api/ais/v1/locations")
                .and_then(|t| parse_digitraffic(&t))
            {
                Ok(ships) => {
                    let n = ships.len();
                    let mut p = picture.lock().unwrap();
                    for mut s in ships {
                        if let Some((name, kind, destination, size)) = meta.get(&s.mmsi) {
                            s.name = name.clone();
                            s.kind = *kind;
                            s.destination = destination.clone();
                            s.size = *size;
                        }
                        merge(&mut p, s);
                    }
                    *status.lock().unwrap() = format!("Baltic AIS: {n} ships");
                }
                Err(e) => *status.lock().unwrap() = format!("Baltic AIS unavailable: {e}"),
            }
            for _ in 0..60 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    });
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A masked client WebSocket frame (RFC 6455).
pub fn frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    let n = payload.len();
    if n < 126 {
        out.push(0x80 | n as u8);
    } else if n < 65536 {
        out.push(0x80 | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// Reads one frame: (fin, opcode, payload). Server frames are unmasked.
pub fn read_frame(stream: &mut impl Read) -> Result<(bool, u8, Vec<u8>), String> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).map_err(|e| e.to_string())?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    let mut len = (head[1] & 0x7f) as u64;
    if len == 126 {
        let mut b = [0u8; 2];
        stream.read_exact(&mut b).map_err(|e| e.to_string())?;
        len = u16::from_be_bytes(b) as u64;
    } else if len == 127 {
        let mut b = [0u8; 8];
        stream.read_exact(&mut b).map_err(|e| e.to_string())?;
        len = u64::from_be_bytes(b);
    }
    if len > 16 << 20 {
        return Err("WebSocket frame too large".into());
    }
    let mut mask = [0u8; 4];
    if masked {
        stream.read_exact(&mut mask).map_err(|e| e.to_string())?;
    }
    let mut payload = vec![0u8; len as usize];
    stream.read_exact(&mut payload).map_err(|e| e.to_string())?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok((fin, opcode, payload))
}

/// Parses one aisstream.io message into a position update.
pub fn parse_aisstream(text: &str) -> Option<Ship> {
    let v: Value = serde_json::from_str(text).ok()?;
    let meta = &v["MetaData"];
    let kind = v["MessageType"].as_str()?;
    let body = &v["Message"][kind];
    let mmsi = meta["MMSI"].as_u64()?;
    let time = meta["time_utc"]
        .as_str()
        .and_then(|t| pelagic::metocean::parse_utc(&t.get(..16)?.replace(' ', "T")))
        .unwrap_or(0);
    let name = meta["ShipName"].as_str().unwrap_or("").trim().to_string();
    let lat = meta["latitude"].as_f64()?;
    let lon = meta["longitude"].as_f64()?;
    let mut ship = Ship {
        mmsi,
        name,
        kind: 0,
        lat,
        lon,
        sog: 0.0,
        cog: 0.0,
        heading: None,
        status: 15,
        destination: String::new(),
        size: None,
        time,
        source: "aisstream.io",
        track: vec![],
    };
    match kind {
        "PositionReport" | "StandardClassBPositionReport" | "ExtendedClassBPositionReport" => {
            ship.sog = body["Sog"].as_f64().filter(|s| *s < 102.2).unwrap_or(0.0);
            ship.cog = body["Cog"].as_f64().filter(|c| *c < 360.0).unwrap_or(0.0);
            ship.heading = body["TrueHeading"].as_f64().filter(|h| *h < 360.0);
            ship.status = body["NavigationalStatus"].as_u64().unwrap_or(15) as u8;
            if let Some(t) = body["Type"].as_u64() {
                ship.kind = t as u16;
            }
        }
        "ShipStaticData" => {
            ship.kind = body["Type"].as_u64().unwrap_or(0) as u16;
            ship.destination = body["Destination"]
                .as_str()
                .unwrap_or("")
                .trim()
                .to_string();
            let d = &body["Dimension"];
            let length = d["A"].as_f64().unwrap_or(0.0) + d["B"].as_f64().unwrap_or(0.0);
            let beam = d["C"].as_f64().unwrap_or(0.0) + d["D"].as_f64().unwrap_or(0.0);
            ship.size = (length > 0.0).then_some((length, beam));
            if let Some(n) = body["Name"].as_str() {
                ship.name = n.trim().to_string();
            }
        }
        _ => return None,
    }
    Some(ship)
}

/// Worldwide stream from aisstream.io with the user's key. Reconnects with
/// back-off until `stop` is set.
pub fn aisstream(key: String, picture: Picture, stop: Arc<AtomicBool>, status: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut wait = 5;
        while !stop.load(Ordering::Relaxed) {
            match stream_once(&key, &picture, &stop, &status) {
                Ok(()) => wait = 5,
                Err(e) => {
                    *status.lock().unwrap() = format!("Worldwide AIS: {e} (retrying in {wait} s)");
                }
            }
            for _ in 0..wait {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            wait = (wait * 2).min(120);
        }
    });
}

fn stream_once(
    key: &str,
    picture: &Picture,
    stop: &AtomicBool,
    status: &Mutex<String>,
) -> Result<(), String> {
    let host = "stream.aisstream.io";
    let tcp = std::net::TcpStream::connect((host, 443)).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(60)))
        .map_err(|e| e.to_string())?;
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let mut s = tls.connect(host, tcp).map_err(|e| e.to_string())?;
    let nonce: [u8; 16] = std::array::from_fn(|i| {
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(7, |d| d.subsec_nanos())
            >> (i % 4 * 8)) as u8
            ^ (i as u8).wrapping_mul(37)
    });
    write!(
        s,
        "GET /v0/stream HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: Pelagic-ocean-engine/1.0\r\n\r\n",
        base64(&nonce)
    )
    .map_err(|e| e.to_string())?;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut byte).map_err(|e| e.to_string())?;
        head.push(byte[0]);
        if head.len() > 8192 {
            return Err("bad WebSocket handshake".into());
        }
    }
    let first = String::from_utf8_lossy(&head);
    if !first.starts_with("HTTP/1.1 101") {
        return Err(first
            .lines()
            .next()
            .unwrap_or("handshake refused")
            .to_string());
    }
    let subscribe = serde_json::json!({
        "APIKey": key,
        "BoundingBoxes": [[[-90.0, -180.0], [90.0, 180.0]]],
        "FilterMessageTypes": ["PositionReport", "StandardClassBPositionReport", "ShipStaticData"]
    })
    .to_string();
    let mask = [nonce[0], nonce[5], nonce[9], nonce[13]];
    *status.lock().unwrap() = "Worldwide AIS: connected, waiting for ships".into();
    s.write_all(&frame(1, subscribe.as_bytes(), mask))
        .map_err(|e| e.to_string())?;
    let mut message = Vec::new();
    let mut count = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let (fin, opcode, payload) = read_frame(&mut s)?;
        match opcode {
            0..=2 => {
                message.extend_from_slice(&payload);
                if !fin {
                    continue;
                }
                let text = String::from_utf8_lossy(&message).into_owned();
                message.clear();
                if let Some(ship) = parse_aisstream(&text) {
                    let mut p = picture.lock().unwrap();
                    merge(&mut p, ship);
                    count += 1;
                    if count.is_multiple_of(50) {
                        *status.lock().unwrap() = format!("Worldwide AIS: live, {} ships", p.len());
                    }
                } else if text.contains("error") {
                    return Err(text.chars().take(160).collect());
                }
            }
            8 => return Err("stream closed by server".into()),
            9 => s
                .write_all(&frame(10, &payload, mask))
                .map_err(|e| e.to_string())?,
            _ => {}
        }
    }
    Ok(())
}

/// Closest point of approach between two vessels moving in straight lines:
/// (distance in nautical miles, time in minutes, negative if past). Local
/// flat-earth geometry, adequate within a few tens of miles.
pub fn cpa(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> (f64, f64) {
    // (lat, lon, sog kn, cog deg) → position in nm and velocity in nm/min.
    let k = a.0.to_radians().cos();
    let rel = ((b.1 - a.1) * 60.0 * k, (b.0 - a.0) * 60.0);
    let v = |s: (f64, f64, f64, f64)| {
        let c = s.3.to_radians();
        (s.2 * c.sin() / 60.0, s.2 * c.cos() / 60.0)
    };
    let (va, vb) = (v(a), v(b));
    let dv = (vb.0 - va.0, vb.1 - va.1);
    let speed2 = dv.0 * dv.0 + dv.1 * dv.1;
    let t = if speed2 < 1e-12 {
        0.0
    } else {
        -(rel.0 * dv.0 + rel.1 * dv.1) / speed2
    };
    let d = ((rel.0 + dv.0 * t).powi(2) + (rel.1 + dv.1 * t).powi(2)).sqrt();
    (d, t)
}

/// Dead-reckoned position after `minutes` along a course (lat, lon).
pub fn dead_reckon(lat: f64, lon: f64, sog: f64, cog: f64, minutes: f64) -> (f64, f64) {
    let nm = sog * minutes / 60.0;
    let c = cog.to_radians();
    let dlat = nm * c.cos() / 60.0;
    let dlon = nm * c.sin() / (60.0 * lat.to_radians().cos().max(0.05));
    (
        (lat + dlat).clamp(-89.9, 89.9),
        (lon + dlon + 540.0).rem_euclid(360.0) - 180.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_feeds_and_frames() {
        let gj = r#"{"type":"FeatureCollection","features":[{"mmsi":230123000,"type":"Feature","geometry":{"type":"Point","coordinates":[24.9,60.1]},"properties":{"mmsi":230123000,"sog":12.3,"cog":271.0,"navStat":0,"heading":270,"timestampExternal":1764000000000}}]}"#;
        let ships = parse_digitraffic(gj).unwrap();
        assert_eq!(ships.len(), 1);
        assert_eq!(
            (ships[0].lat, ships[0].lon, ships[0].sog),
            (60.1, 24.9, 12.3)
        );
        assert_eq!(ships[0].time, 1_764_000_000);
        let meta = parse_digitraffic_vessels(r#"[{"mmsi":230123000,"name":"VIKING GRACE ","shipType":60,"destination":"TURKU","referencePointA":150,"referencePointB":68,"referencePointC":17,"referencePointD":18}]"#).unwrap();
        assert_eq!(meta[&230123000].0, "VIKING GRACE");
        assert_eq!(meta[&230123000].3, Some((218.0, 35.0)));
        let msg = r#"{"MessageType":"PositionReport","MetaData":{"MMSI":244123456,"ShipName":"EVER GIVEN ","latitude":51.9,"longitude":4.1,"time_utc":"2026-11-24 07:00:05.123 +0000 UTC"},"Message":{"PositionReport":{"Cog":88.5,"Sog":14.2,"TrueHeading":90,"NavigationalStatus":0}}}"#;
        let s = parse_aisstream(msg).unwrap();
        assert_eq!(
            (s.mmsi, s.name.as_str(), s.cog, s.sog),
            (244123456, "EVER GIVEN", 88.5, 14.2)
        );
        assert!(s.time > 1_700_000_000, "{}", s.time);
        let st = r#"{"MessageType":"ShipStaticData","MetaData":{"MMSI":244123456,"ShipName":"X","latitude":51.9,"longitude":4.1,"time_utc":"2026-11-24 07:00:05"},"Message":{"ShipStaticData":{"Type":70,"Destination":"ROTTERDAM","Dimension":{"A":300,"B":100,"C":30,"D":29},"Name":"EVER GIVEN"}}}"#;
        let st = parse_aisstream(st).unwrap();
        assert_eq!((st.kind, st.size), (70, Some((400.0, 59.0))));
        let mut picture = HashMap::new();
        merge(&mut picture, st);
        merge(&mut picture, s.clone());
        let merged = &picture[&244123456];
        assert_eq!(
            (merged.kind, merged.destination.as_str()),
            (70, "ROTTERDAM")
        );
        assert_eq!(merged.track.len(), 1, "same position not duplicated");
        assert_eq!(category(55), "Law enforcement");
        // Frames round-trip (client masks; reader unmasks).
        let f = frame(1, b"hello", [1, 2, 3, 4]);
        let (fin, op, payload) = read_frame(&mut std::io::Cursor::new(f)).unwrap();
        assert!(fin && op == 1 && payload == b"hello");
        let long = vec![7u8; 70_000];
        let (_, _, p) =
            read_frame(&mut std::io::Cursor::new(frame(2, &long, [9, 8, 7, 6]))).unwrap();
        assert_eq!(p, long);
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
    }
    #[test]
    fn closest_approach_and_dead_reckoning() {
        // Head-on at 10 kn each, 4 nm apart: meet in 12 minutes.
        let a = (50.0, 0.0, 10.0, 90.0);
        let b = (50.0, 4.0 / (60.0 * 50f64.to_radians().cos()), 10.0, 270.0);
        let (d, t) = cpa(a, b);
        assert!(d < 0.01 && (t - 12.0).abs() < 0.01, "{d} {t}");
        // Parallel courses 1 nm apart never get closer.
        let c = (50.0 + 1.0 / 60.0, 0.0, 10.0, 90.0);
        let (d, _) = cpa(a, c);
        assert!((d - 1.0).abs() < 1e-6);
        // 60 minutes north at 6 kn is 0.1° of latitude.
        let (lat, lon) = dead_reckon(10.0, 20.0, 6.0, 0.0, 60.0);
        assert!((lat - 10.1).abs() < 1e-9 && (lon - 20.0).abs() < 1e-9);
    }
}
#[cfg(test)]
mod live_probe {
    #[test]
    #[ignore]
    fn aisstream_delivers() {
        let key =
            std::fs::read_to_string(crate::project::data_dir().join("aisstream.key")).unwrap();
        let picture: super::Picture = Default::default();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let status = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        super::aisstream(
            key.trim().to_string(),
            picture.clone(),
            stop.clone(),
            status.clone(),
        );
        std::thread::sleep(std::time::Duration::from_secs(30));
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        println!(
            "ships {} status {:?}",
            picture.lock().unwrap().len(),
            status.lock().unwrap()
        );
    }
}
