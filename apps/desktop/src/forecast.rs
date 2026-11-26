//! Live marine forecasts from Open-Meteo (https://open-meteo.com/): global
//! wave models (ECMWF WAM, Météo-France MFWAM, NOAA GFS-Wave, DWD) for waves,
//! swell, currents and sea temperature, plus 10 m wind from its weather API.
//!
//! Requests carry only coordinates and the variables wanted, with a generic
//! user agent. Responses are cached under `%LOCALAPPDATA%\Pelagic\forecasts`
//! for one 6-hour model cycle, so the map is fetched at most four times a
//! day. Global-map requests are paced to stay inside the free tier
//! (non-commercial use, under 600 locations a minute and 10,000 a day).
//! Commercial users set `PELAGIC_OPEN_METEO_KEY` to their API key, which
//! switches to Open-Meteo's customer servers.
//!
//! Data licence: CC BY 4.0. Attribution must be shown wherever the data are.
use pelagic::metocean::{Forecast, MARINE_HOURLY, WaveField};
use std::path::PathBuf;

/// Shown next to any forecast data (required by the licence).
pub const ATTRIBUTION: &str = "Wave, current and wind data: Open-Meteo.com (CC BY 4.0) · models by ECMWF, Météo-France / Copernicus Marine, NOAA NCEP, DWD";
pub const SOURCE: &str = "Open-Meteo forecast (best-match wave models)";
/// Locations per request and per minute when building the global map.
const BATCH: usize = 100;
const PER_MINUTE: usize = 590;

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

/// (marine base, weather base, key query) — customer servers with a key.
fn servers() -> (&'static str, &'static str, String) {
    match std::env::var("PELAGIC_OPEN_METEO_KEY").ok().filter(|k| {
        !k.trim().is_empty()
            && k.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }) {
        Some(key) => (
            "https://customer-marine-api.open-meteo.com",
            "https://customer-api.open-meteo.com",
            format!("&apikey={key}"),
        ),
        None => (
            "https://marine-api.open-meteo.com",
            "https://api.open-meteo.com",
            String::new(),
        ),
    }
}

fn cache_dir() -> PathBuf {
    crate::project::data_dir().join("forecasts")
}

/// Current 6-hour model cycle, e.g. "2026-11-23T12".
fn cycle() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let hour = pelagic::metocean::utc_hour(now - now.rem_euclid(6 * 3600));
    hour[..13].to_string()
}

/// The current UTC time to the minute ("YYYY-MM-DDTHH:MM").
pub fn now_minute() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    pelagic::metocean::format_utc(now)
}

/// The current UTC hour ("YYYY-MM-DDTHH:00").
pub fn now_hour() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    pelagic::metocean::utc_hour(now)
}

fn get_json(
    agent: &ureq::Agent,
    url: &str,
    cache: Option<&PathBuf>,
) -> Result<serde_json::Value, String> {
    if let Some(path) = cache
        && let Ok(bytes) = std::fs::read(path)
        && let Ok(value) = serde_json::from_slice(&bytes)
    {
        return Ok(value);
    }
    let mut response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(429) => {
            "the forecast service's free daily limit is reached; try later or set PELAGIC_OPEN_METEO_KEY".to_string()
        }
        other => format!("could not reach the forecast service ({other})"),
    })?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("forecast response: {e}"))?;
    if let Some(reason) = value["reason"].as_str() {
        return Err(format!("forecast service: {reason}"));
    }
    if let Some(path) = cache {
        let _ = std::fs::create_dir_all(cache_dir());
        let temp = path.with_extension("partial");
        if std::fs::write(&temp, &bytes).is_ok() {
            let _ = std::fs::rename(&temp, path);
        }
    }
    Ok(value)
}

/// Hourly forecast at one location for `days` (1–8) days.
pub fn point(latitude: f64, longitude: f64, days: u32) -> Result<Forecast, String> {
    if !(-90.0..=90.0).contains(&latitude) || !longitude.is_finite() {
        return Err("Pick a valid latitude and longitude.".into());
    }
    let longitude = (longitude + 540.0).rem_euclid(360.0) - 180.0;
    let days = days.clamp(1, 8);
    let (marine, weather, key) = servers();
    let agent = agent();
    let stamp = format!("{}-{latitude:.2}_{longitude:.2}-{days}", cycle());
    let marine_url = format!(
        "{marine}/v1/marine?latitude={latitude:.4}&longitude={longitude:.4}&hourly={MARINE_HOURLY}&forecast_days={days}&timezone=GMT{key}"
    );
    let weather_url = format!(
        "{weather}/v1/forecast?latitude={latitude:.4}&longitude={longitude:.4}&hourly=wind_speed_10m,wind_direction_10m&wind_speed_unit=ms&forecast_days={days}&timezone=GMT{key}"
    );
    let m = get_json(
        &agent,
        &marine_url,
        Some(&cache_dir().join(format!("marine-{stamp}.json"))),
    )?;
    // Wind is optional: the sea state stands without it.
    let w = get_json(
        &agent,
        &weather_url,
        Some(&cache_dir().join(format!("wind-{stamp}.json"))),
    )
    .ok();
    let mut forecast = Forecast::parse_open_meteo(&m, w.as_ref(), SOURCE)?;
    // Keep the requested location (the service snaps to its grid).
    if !forecast.latitude.is_finite() {
        forecast.latitude = latitude;
        forecast.longitude = longitude;
    }
    if forecast.hours.iter().all(|h| h.wave_height.is_none()) {
        return Err("No wave forecast here (land or sea ice). Pick a point at sea.".into());
    }
    Ok(forecast)
}

/// Hourly wind and surface current on a 5×5 grid (about 22 km apart)
/// around a last known position, from two days ago to three days ahead,
/// for search-and-rescue drift. Locations snap to the service's nearest sea
/// cell; duplicates are merged.
pub fn drift_area(latitude: f64, longitude: f64) -> Result<Vec<Forecast>, String> {
    if !(-80.0..=80.0).contains(&latitude) || !longitude.is_finite() {
        return Err("Pick a last known position between 80°S and 80°N.".into());
    }
    let longitude = (longitude + 540.0).rem_euclid(360.0) - 180.0;
    let (marine, weather, key) = servers();
    let agent = agent();
    let step = 0.2;
    let lon_step = step / latitude.to_radians().cos().max(0.2);
    let mut lats = vec![];
    let mut lons = vec![];
    for i in -2..=2 {
        for j in -2..=2 {
            lats.push(format!("{:.3}", latitude + i as f64 * step));
            let lon = longitude + j as f64 * lon_step;
            lons.push(format!("{:.3}", (lon + 540.0).rem_euclid(360.0) - 180.0));
        }
    }
    let (lats, lons) = (lats.join(","), lons.join(","));
    let stamp = format!("{}-{latitude:.2}_{longitude:.2}", cycle());
    let marine_url = format!(
        "{marine}/v1/marine?latitude={lats}&longitude={lons}&hourly=ocean_current_velocity,ocean_current_direction,wave_height&past_days=2&forecast_days=3&timezone=GMT{key}"
    );
    let weather_url = format!(
        "{weather}/v1/forecast?latitude={lats}&longitude={lons}&hourly=wind_speed_10m,wind_direction_10m&wind_speed_unit=ms&past_days=2&forecast_days=3&timezone=GMT{key}"
    );
    let m = get_json(
        &agent,
        &marine_url,
        Some(&cache_dir().join(format!("drift-marine-{stamp}.json"))),
    )?;
    let w = get_json(
        &agent,
        &weather_url,
        Some(&cache_dir().join(format!("drift-wind-{stamp}.json"))),
    )?;
    let (Some(m), Some(w)) = (m.as_array(), w.as_array()) else {
        return Err("Unexpected forecast response for the drift area.".into());
    };
    let mut out: Vec<Forecast> = vec![];
    for (marine, weather) in m.iter().zip(w) {
        let f = Forecast::parse_open_meteo(marine, Some(weather), SOURCE)?;
        if !out.iter().any(|o| {
            (o.latitude - f.latitude).abs() < 1e-4 && (o.longitude - f.longitude).abs() < 1e-4
        }) {
            out.push(f);
        }
    }
    if out.is_empty() {
        return Err("No forecast for the drift area.".into());
    }
    Ok(out)
}

/// Grid locations for the global map, `step` degrees apart (sea and land;
/// land comes back without values).
pub fn grid(step: f64) -> Vec<(f64, f64)> {
    let step = step.clamp(2.0, 20.0);
    let mut points = vec![];
    let mut lat = -70.0;
    while lat <= 80.0 + 1e-9 {
        let mut lon = -180.0;
        while lon < 180.0 - 1e-9 {
            points.push((lat, lon));
            lon += step;
        }
        lat += step;
    }
    points
}

/// Wave height, direction and period for `days` days over the global grid.
/// `progress(done, total, partial)` reports each batch; it returns false to
/// cancel. Batches are paced to at most 590 locations a minute.
pub fn field(
    step: f64,
    days: u32,
    mut progress: impl FnMut(usize, usize, &WaveField) -> bool,
) -> Result<WaveField, String> {
    let points = grid(step);
    let days = days.clamp(1, 8);
    let (marine, _, key) = servers();
    let agent = agent();
    let total = points.len();
    let mut field = WaveField::default();
    let mut window: std::collections::VecDeque<(std::time::Instant, usize)> = Default::default();
    for (index, batch) in points.chunks(BATCH).enumerate() {
        let cache = cache_dir().join(format!("field-{}-{step}-{days}-{index}.json", cycle()));
        let cached = cache.exists();
        if !cached {
            // Pace: wait until the last minute's locations leave room.
            loop {
                let now = std::time::Instant::now();
                while window
                    .front()
                    .is_some_and(|(t, _)| now.duration_since(*t).as_secs() >= 60)
                {
                    window.pop_front();
                }
                if window.iter().map(|(_, n)| n).sum::<usize>() + batch.len() <= PER_MINUTE {
                    break;
                }
                if !progress(field.points.len(), total, &field) {
                    return Err("Cancelled.".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            window.push_back((std::time::Instant::now(), batch.len()));
        }
        let lats: Vec<String> = batch.iter().map(|p| format!("{:.2}", p.0)).collect();
        let lons: Vec<String> = batch.iter().map(|p| format!("{:.2}", p.1)).collect();
        let url = format!(
            "{marine}/v1/marine?latitude={}&longitude={}&hourly=wave_height,wave_direction,wave_period&forecast_days={days}&timezone=GMT&cell_selection=nearest{key}",
            lats.join(","),
            lons.join(",")
        );
        let value = get_json(&agent, &url, Some(&cache))?;
        field.extend(WaveField::parse_open_meteo(&value, batch)?);
        if !progress(field.points.len(), total, &field) {
            return Err("Cancelled.".into());
        }
    }
    Ok(field)
}

/// Wave height, direction and period over a regional grid (at most 100
/// points, one request) for the next day: the zoomed-in live sea.
pub fn region(points: &[(f64, f64)]) -> Result<WaveField, String> {
    if points.is_empty() || points.len() > BATCH {
        return Err("regional wave grid must have 1-100 points".into());
    }
    let (marine, _, key) = servers();
    let lats: Vec<String> = points.iter().map(|p| format!("{:.3}", p.0)).collect();
    let lons: Vec<String> = points.iter().map(|p| format!("{:.3}", p.1)).collect();
    let url = format!(
        "{marine}/v1/marine?latitude={}&longitude={}&hourly=wave_height,wave_direction,wave_period&forecast_days=1&timezone=GMT&cell_selection=sea{key}",
        lats.join(","),
        lons.join(",")
    );
    let value = get_json(&agent(), &url, None)?;
    WaveField::parse_open_meteo(&value, points)
}
#[cfg(test)]
mod tests {
    #[test]
    fn global_grid_covers_the_open_ocean_band() {
        let g = super::grid(10.0);
        assert_eq!(g.len(), 16 * 36);
        assert!(
            g.iter()
                .all(|p| (-70.0..=80.0).contains(&p.0) && (-180.0..180.0).contains(&p.1))
        );
        assert_eq!(super::cycle().len(), 13);
    }
}
