//! Clear-sky atmosphere (single scattering by air molecules and aerosols,
//! ozone absorption), shared by the CPU (sun colour, sky irradiance, sun
//! position) and the shader (`sky_lut` in ocean.wgsl, same constants).
//! Coefficients follow Bruneton & Neyret (2008) and Hillaire (2020); the
//! multiple-scattering term is a small isotropic approximation. Not a
//! radiometrically calibrated sky.
use pelagic::math::Vec3;

pub const EARTH: f64 = 6_360_000.0;
pub const TOP: f64 = 6_460_000.0;
pub const RAYLEIGH: [f64; 3] = [5.802e-6, 13.558e-6, 33.1e-6];
pub const RAYLEIGH_HEIGHT: f64 = 8000.0;
/// Maritime aerosol: about twice the standard continental clear-sky value.
pub const MIE: f64 = 7.0e-6;
pub const MIE_EXTINCTION: f64 = 7.8e-6;
pub const MIE_HEIGHT: f64 = 1200.0;
pub const OZONE: [f64; 3] = [0.650e-6, 1.881e-6, 0.085e-6];
/// Top-of-atmosphere sun irradiance in the renderer's HDR units.
pub const SUN: f64 = 3.6;

fn density(h: f64) -> (f64, f64, f64) {
    let ozone = (1.0 - (h - 25_000.0).abs() / 15_000.0).max(0.0);
    ((-h / RAYLEIGH_HEIGHT).exp(), (-h / MIE_HEIGHT).exp(), ozone)
}

fn extinction(h: f64) -> [f64; 3] {
    let (r, m, o) = density(h.max(0.0));
    std::array::from_fn(|i| RAYLEIGH[i] * r + MIE_EXTINCTION * m + OZONE[i] * o)
}

/// Distance from radius `r` along a ray with cosine `mu` (to the zenith) to
/// the top of the atmosphere, or `None` if the ray hits the ground first.
fn to_top(r: f64, mu: f64) -> Option<f64> {
    let ground = r * r * (mu * mu - 1.0) + EARTH * EARTH;
    if mu < 0.0 && ground >= 0.0 {
        return None;
    }
    let disc = r * r * (mu * mu - 1.0) + TOP * TOP;
    Some(-r * mu + disc.max(0.0).sqrt())
}

/// Transmittance from altitude `h` towards a direction with zenith cosine
/// `mu` (zero if the Earth blocks it).
pub fn transmittance(h: f64, mu: f64) -> [f64; 3] {
    let r = EARTH + h.max(0.0);
    let Some(length) = to_top(r, mu) else {
        return [0.0; 3];
    };
    let steps = 40;
    let mut depth = [0.0; 3];
    for i in 0..steps {
        let t = (i as f64 + 0.5) / steps as f64 * length;
        let rr = (r * r + t * t + 2.0 * r * t * mu).sqrt();
        let e = extinction(rr - EARTH);
        for c in 0..3 {
            depth[c] += e[c] * length / steps as f64;
        }
    }
    depth.map(|d| (-d).exp())
}

/// Sunlight reaching the sea surface (HDR units), including the reddening
/// near the horizon; fades out as the sun sets.
pub fn sun_light(sun: Vec3) -> [f64; 3] {
    transmittance(2.0, sun.z.clamp(-1.0, 1.0)).map(|t| t * SUN)
}

fn rayleigh_phase(mu: f64) -> f64 {
    3.0 / (16.0 * std::f64::consts::PI) * (1.0 + mu * mu)
}

fn mie_phase(mu: f64) -> f64 {
    let g: f64 = 0.8;
    3.0 / (8.0 * std::f64::consts::PI) * (1.0 - g * g) * (1.0 + mu * mu)
        / ((2.0 + g * g) * (1.0 + g * g - 2.0 * g * mu).powf(1.5))
}

/// Sky radiance seen from sea level along `ray` (single scattering plus an
/// isotropic multiple-scattering term).
pub fn radiance(ray: Vec3, sun: Vec3) -> [f64; 3] {
    let r = EARTH + 2.0;
    let mu = ray.z.clamp(-1.0, 1.0);
    let length = to_top(r, mu).unwrap_or_else(|| {
        // Ray towards the ground: march to the ground hit.
        let disc = r * r * (mu * mu - 1.0) + EARTH * EARTH;
        -r * mu - disc.max(0.0).sqrt()
    });
    let steps = 24;
    let nu = ray.dot(sun);
    let (pr, pm) = (rayleigh_phase(nu), mie_phase(nu));
    let mut depth = [0.0; 3];
    let mut light = [0.0; 3];
    for i in 0..steps {
        let t0 = (i as f64 / steps as f64).powi(2) * length;
        let t1 = ((i + 1) as f64 / steps as f64).powi(2) * length;
        let dt = t1 - t0;
        let t = 0.5 * (t0 + t1);
        let p = Vec3::new(0.0, 0.0, r) + ray * t;
        let rr = p.length();
        let h = rr - EARTH;
        let up = p * (1.0 / rr);
        let (dr, dm, _) = density(h.max(0.0));
        let e = extinction(h);
        let to_sun = transmittance(h, up.dot(sun));
        for c in 0..3 {
            let view = (-(depth[c] + e[c] * dt * 0.5)).exp();
            let scatter = RAYLEIGH[c] * dr * pr + MIE * dm * pm;
            let multiple = (RAYLEIGH[c] * dr + MIE * dm) * 0.08 * sun.z.max(0.0).sqrt();
            light[c] += view * (scatter * to_sun[c] + multiple) * dt;
            depth[c] += e[c] * dt;
        }
    }
    light.map(|l| l * SUN)
}

/// Irradiance on a horizontal surface from the whole sky (no direct sun).
pub fn sky_irradiance(sun: Vec3) -> [f64; 3] {
    let (rings, segments) = (6, 12);
    let mut sum = [0.0; 3];
    for i in 0..rings {
        // Elevation bands of equal width in sin(elevation) (cosine-weighted).
        let s0 = i as f64 / rings as f64;
        let s1 = (i + 1) as f64 / rings as f64;
        let z = ((s0 + s1) * 0.5).sqrt();
        let band = std::f64::consts::PI * (s1 - s0) / segments as f64;
        for j in 0..segments {
            let a = j as f64 / segments as f64 * std::f64::consts::TAU;
            let h = (1.0 - z * z).max(0.0).sqrt();
            let l = radiance(Vec3::new(h * a.cos(), h * a.sin(), z), sun);
            for c in 0..3 {
                sum[c] += l[c] * band;
            }
        }
    }
    sum
}

/// Unit vector towards the sun from its elevation and azimuth (degrees;
/// azimuth clockwise from north, scene axes X east, Y north, Z up).
pub fn sun_direction(elevation_deg: f64, azimuth_deg: f64) -> Vec3 {
    let (e, a) = (elevation_deg.to_radians(), azimuth_deg.to_radians());
    Vec3::new(e.cos() * a.sin(), e.cos() * a.cos(), e.sin()).normalized()
}

/// Solar elevation and azimuth (degrees, azimuth clockwise from north) at a
/// place and UTC instant: NOAA's general solar position equations
/// (Spencer's Fourier series), accurate to a fraction of a degree.
pub fn solar_position(lat: f64, lon: f64, day_of_year: f64, utc_hours: f64) -> (f64, f64) {
    use std::f64::consts::PI;
    let gamma = 2.0 * PI / 365.0 * (day_of_year - 1.0 + (utc_hours - 12.0) / 24.0);
    let eqtime = 229.18
        * (0.000075 + 0.001868 * gamma.cos()
            - 0.032077 * gamma.sin()
            - 0.014615 * (2.0 * gamma).cos()
            - 0.040849 * (2.0 * gamma).sin());
    let decl = 0.006918 - 0.399912 * gamma.cos() + 0.070257 * gamma.sin()
        - 0.006758 * (2.0 * gamma).cos()
        + 0.000907 * (2.0 * gamma).sin()
        - 0.002697 * (3.0 * gamma).cos()
        + 0.00148 * (3.0 * gamma).sin();
    let true_solar = utc_hours * 60.0 + eqtime + 4.0 * lon;
    let ha = (true_solar / 4.0 - 180.0).to_radians();
    let phi = lat.to_radians();
    let cos_zenith = (phi.sin() * decl.sin() + phi.cos() * decl.cos() * ha.cos()).clamp(-1.0, 1.0);
    let zenith = cos_zenith.acos();
    let elevation = 90.0 - zenith.to_degrees();
    // Azimuth clockwise from north.
    let az = (-ha.sin()).atan2(phi.cos() * decl.tan() - phi.sin() * ha.cos());
    (elevation, az.to_degrees().rem_euclid(360.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sun_reddens_and_dims_towards_the_horizon() {
        let high = sun_light(sun_direction(60.0, 180.0));
        let low = sun_light(sun_direction(3.0, 180.0));
        assert!(high[0] > 2.5 && high[2] > 1.8, "{high:?}");
        // Blue is lost faster than red near the horizon.
        assert!(low[2] / low[0] < 0.5 * high[2] / high[0], "{low:?}");
        assert_eq!(sun_light(sun_direction(-5.0, 180.0)), [0.0; 3]);
    }
    #[test]
    fn clear_sky_is_blue_and_dimmer_than_the_sun() {
        let sun = sun_direction(45.0, 135.0);
        let zenith = radiance(Vec3::new(0.0, 0.0, 1.0), sun);
        assert!(zenith[2] > zenith[1] && zenith[1] > zenith[0], "{zenith:?}");
        let e = sky_irradiance(sun);
        let direct = sun_light(sun)[1] * sun.z;
        // Diffuse sky light is roughly 10-35 % of the direct beam at 45°.
        assert!(
            e[1] > 0.08 * direct && e[1] < 0.4 * direct,
            "{e:?} {direct}"
        );
    }
    #[test]
    fn solar_position_matches_reference_values() {
        // Equator, March equinox (day 80), local solar noon at 0° longitude:
        // the sun is within a couple of degrees of the zenith.
        let (e, _) = solar_position(0.0, 0.0, 80.0, 12.1);
        assert!(e > 87.0, "{e}");
        // Greenwich (51.48 N) at noon UTC on 21 June: elevation ≈ 62°,
        // azimuth ≈ 180°.
        let (e, a) = solar_position(51.48, 0.0, 172.0, 12.0);
        assert!((e - 62.0).abs() < 1.0, "{e}");
        assert!((a - 180.0).abs() < 3.0, "{a}");
        // Sydney (33.87 S, 151.21 E) at 00:00 UTC on 1 January (11:00 local
        // daylight time): high sun, azimuth in the north-east quadrant.
        let (e, a) = solar_position(-33.87, 151.21, 1.0, 0.0);
        assert!(e > 55.0 && e < 70.0, "{e}");
        assert!(a > 20.0 && a < 110.0, "{a}");
        let d = sun_direction(30.0, 90.0);
        assert!((d.x - 30f64.to_radians().cos()).abs() < 1e-9 && d.y.abs() < 1e-9);
    }
}
