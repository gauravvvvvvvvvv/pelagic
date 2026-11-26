//! Seeded, directional linear gravity waves. Discrete JONSWAP-shaped energy is
//! normalised to m0=Hs²/16. No breaking, refraction or CFD is implied.
use crate::math::Det;
use crate::math::Vec3;
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy, Debug)]
pub struct Component {
    pub amplitude: f64,
    pub k: f64,
    pub omega: f64,
    pub direction: f64,
    pub phase: f64,
    kx: f64,
    ky: f64,
}
#[derive(Clone, Debug, Default)]
pub struct SeaState {
    components: Vec<Component>,
    pub depth: f64,
    /// Wind speed 10 m above the sea, m/s, when known (0 = unknown). Used
    /// for whitecaps; it does not change the wave kinematics.
    pub wind_speed: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct SurfaceSample {
    pub height: f64,
    pub normal: Vec3,
    pub vertical_speed: f64,
}
impl SeaState {
    /// One regular Airy wave. Height is crest-to-trough, direction in radians.
    pub fn regular(
        height: f64,
        period: f64,
        direction: f64,
        depth: f64,
    ) -> Result<Self, &'static str> {
        if ![height, period, direction, depth]
            .iter()
            .all(|v| v.is_finite())
            || height < 0.0
            || period < 1.0
            || depth <= 0.0
        {
            return Err("invalid regular-wave parameters");
        }
        let omega = TAU / period;
        let k = wave_number(omega, depth);
        Ok(Self {
            depth,
            wind_speed: 0.0,
            components: vec![Component {
                amplitude: height * 0.5,
                k,
                omega,
                direction,
                phase: 0.0,
                kx: k * direction.dcos(),
                ky: k * direction.dsin(),
            }],
        })
    }
    pub fn spectrum(
        hs: f64,
        period: f64,
        direction: f64,
        spread: f64,
        depth: f64,
        seed: u64,
    ) -> Result<Self, &'static str> {
        Self::jonswap(
            hs, period, direction, spread, depth, seed, 32, 3.3, 0.5, 5.0,
        )
    }
    /// Swell: a narrow-banded JONSWAP (peak enhancement 7) of 16 components
    /// between 0.75 and 1.6 times the peak frequency. Use a small `spread`
    /// (swell is nearly long-crested). Combine with [`SeaState::with`].
    pub fn swell(
        hs: f64,
        period: f64,
        direction: f64,
        spread: f64,
        depth: f64,
        seed: u64,
    ) -> Result<Self, &'static str> {
        Self::jonswap(
            hs,
            period,
            direction,
            spread,
            depth,
            seed ^ 0x5be1_1d5e_a5ea_0001,
            16,
            7.0,
            0.75,
            1.6 / 0.75,
        )
    }
    /// Both sea states at once (linear superposition): wind sea plus swell.
    pub fn with(mut self, other: SeaState) -> Self {
        self.components.extend(other.components);
        self.wind_speed = self.wind_speed.max(other.wind_speed);
        self
    }
    /// JONSWAP-shaped energy in `count` components over `band_low` ×
    /// `band_ratio^(i/count)` times the peak frequency, normalised to Hs.
    #[allow(clippy::too_many_arguments)]
    fn jonswap(
        hs: f64,
        period: f64,
        direction: f64,
        spread: f64,
        depth: f64,
        seed: u64,
        count: usize,
        gamma: f64,
        band_low: f64,
        band_ratio: f64,
    ) -> Result<Self, &'static str> {
        if ![hs, period, direction, spread, depth]
            .iter()
            .all(|v| v.is_finite())
            || hs < 0.0
            || period < 1.0
            || depth <= 0.0
            || !(0.0..=PI).contains(&spread)
        {
            return Err("invalid sea-state parameters");
        }
        if hs == 0.0 {
            return Ok(Self {
                components: vec![],
                depth,
                wind_speed: 0.0,
            });
        }
        let peak = TAU / period;
        let mut random = seed ^ 0x9e3779b97f4a7c15;
        let mut unit = || {
            random = random.wrapping_add(0x9e3779b97f4a7c15);
            let mut z = random;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
            ((z ^ (z >> 31)) >> 11) as f64 / ((1_u64 << 53) as f64)
        };
        let mut components = Vec::with_capacity(count);
        let mut energy = 0.0;
        let n = count as f64;
        for i in 0..count {
            let lo = band_low * band_ratio.dpowf(i as f64 / n);
            let hi = band_low * band_ratio.dpowf((i + 1) as f64 / n);
            let omega = peak * (lo + hi) * 0.5;
            let sigma: f64 = if omega <= peak { 0.07 } else { 0.09 };
            let r = (-0.5 * ((omega / peak - 1.0) / sigma).powi(2)).dexp();
            let weight = omega.powi(-5)
                * (-1.25 * (peak / omega).powi(4)).dexp()
                * gamma.dpowf(r)
                * (hi - lo)
                * peak;
            energy += weight;
            let direction = direction + (unit() + unit() - 1.0) * spread;
            let k = wave_number(omega, depth);
            components.push(Component {
                amplitude: weight,
                omega,
                k,
                direction,
                phase: unit() * TAU,
                kx: k * direction.dcos(),
                ky: k * direction.dsin(),
            });
        }
        for c in &mut components {
            c.amplitude = (2.0 * (hs * hs / 16.0) * c.amplitude / energy).sqrt();
        }
        Ok(Self {
            components,
            depth,
            wind_speed: 0.0,
        })
    }
    pub fn sample(&self, x: f64, y: f64, time: f64) -> SurfaceSample {
        let mut height = 0.0;
        let mut dx = 0.0;
        let mut dy = 0.0;
        let mut vertical_speed = 0.0;
        for c in &self.components {
            let (sn, cs) = (c.kx * x + c.ky * y - c.omega * time + c.phase).dsin_cos();
            height += c.amplitude * cs;
            dx -= c.amplitude * c.kx * sn;
            dy -= c.amplitude * c.ky * sn;
            vertical_speed += c.amplitude * c.omega * sn;
        }
        SurfaceSample {
            height,
            normal: Vec3::new(-dx, -dy, 1.0).normalized(),
            vertical_speed,
        }
    }
    /// Same analytic surface on a bounded regular grid. Phase recurrence avoids
    /// repeating trigonometry at every water-mesh vertex; it changes no physics.
    pub fn surface_grid(
        &self,
        origin: [f64; 2],
        spacing: f64,
        columns: usize,
        rows: usize,
        time: f64,
    ) -> Result<Vec<SurfaceSample>, &'static str> {
        if !(1..=257).contains(&columns)
            || !(1..=257).contains(&rows)
            || ![origin[0], origin[1], spacing, time]
                .iter()
                .all(|v| v.is_finite())
            || spacing <= 0.0
        {
            return Err("invalid surface grid");
        }
        let mut samples = vec![
            SurfaceSample {
                height: 0.0,
                normal: Vec3::ZERO,
                vertical_speed: 0.0
            };
            columns * rows
        ];
        for c in &self.components {
            let (step_sin, step_cos) = (c.kx * spacing).dsin_cos();
            for y in 0..rows {
                let (mut sn, mut cs) = (c.kx * origin[0] + c.ky * (origin[1] + y as f64 * spacing)
                    - c.omega * time
                    + c.phase)
                    .dsin_cos();
                for sample in &mut samples[y * columns..(y + 1) * columns] {
                    sample.height += c.amplitude * cs;
                    sample.normal.x += c.amplitude * c.kx * sn;
                    sample.normal.y += c.amplitude * c.ky * sn;
                    sample.vertical_speed += c.amplitude * c.omega * sn;
                    (sn, cs) = (sn * step_cos + cs * step_sin, cs * step_cos - sn * step_sin);
                }
            }
        }
        for sample in &mut samples {
            sample.normal.z = 1.0;
            sample.normal = sample.normal.normalized();
        }
        Ok(samples)
    }
    pub fn orbital_velocity(&self, p: Vec3, time: f64) -> Vec3 {
        if self.depth <= 0.0 || p.z < -self.depth {
            return Vec3::ZERO;
        }
        let z = p.z.min(0.0);
        let mut result = Vec3::ZERO;
        for c in &self.components {
            let (sn, cs) = (c.kx * p.x + c.ky * p.y - c.omega * time + c.phase).dsin_cos();
            let denominator = 1.0 - (-2.0 * c.k * self.depth).dexp();
            let a = (c.k * z).dexp() / denominator;
            let reflected = (-2.0 * c.k * (z + self.depth)).dexp();
            let horizontal = c.amplitude * c.omega * a * (1.0 + reflected) * cs;
            result = result
                + Vec3::new(
                    c.kx / c.k * horizontal,
                    c.ky / c.k * horizontal,
                    c.amplitude * c.omega * a * (1.0 - reflected) * sn,
                );
        }
        result
    }
    pub fn significant_height(&self) -> f64 {
        4.0 * (self
            .components
            .iter()
            .map(|c| c.amplitude * c.amplitude * 0.5)
            .sum::<f64>())
        .sqrt()
    }
    pub fn components(&self) -> &[Component] {
        &self.components
    }
}
pub fn wave_number(omega: f64, depth: f64) -> f64 {
    let mut low = 0.0;
    let mut high = (omega * omega / 9.80665 + omega / (9.80665 * depth).sqrt()) * 2.0;
    for _ in 0..64 {
        let mid = (low + high) * 0.5;
        if 9.80665 * mid * (mid * depth).dtanh() > omega * omega {
            high = mid;
        } else {
            low = mid;
        }
    }
    (low + high) * 0.5
}
