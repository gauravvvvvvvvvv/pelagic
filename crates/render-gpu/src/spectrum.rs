//! Short-wave detail for the GPU ocean: the spectral tail of the scene's wind
//! sea above the highest frequency the physics model carries.
//!
//! The physics (`pelagic::waves::SeaState`) keeps a few dozen long, energetic
//! components that vessels respond to. Real seas also carry a continuum of
//! shorter waves (metres to centimetres) that dominate how the surface looks:
//! slope, glitter, choppy crests and breaking. They are synthesised here
//! (Tessendorf 2001) from the same JONSWAP spectrum, normalised so that the
//! physics band [0.5, 2.5] × the peak frequency holds exactly Hs²/16, and
//! continued above the physics band's top edge. Nothing below that edge is
//! added, so the long waves stay exactly the ones the physics simulates.
//!
//! Three FFT cascades of `N`² cells cover disjoint wavenumber bands. Angular
//! frequencies are quantised to multiples of 2π/`REPEAT` so the field can be
//! evaluated at `time mod REPEAT` in f32 without losing phase accuracy.
use pelagic::waves::SeaState;
use std::f64::consts::{PI, TAU};

pub const N: usize = 256;
pub const CASCADES: usize = 3;
/// Cascade tile sizes (m). Non-integer ratios avoid visibly aligned tiling.
pub const SIZES: [f64; CASCADES] = [301.0, 57.3, 10.7];
/// Horizontal displacement scale ("choppiness") of the detail waves.
pub const CHOP: f64 = 1.0;
/// Horizontal displacement scale applied to the physics components on the
/// GPU (visual only; at typical steepness it moves the rendered height by
/// a few centimetres relative to the linear surface the physics uses).
pub const ANALYTIC_CHOP: f64 = 0.6;
/// Period (s) after which the detail field repeats exactly.
pub const REPEAT: f64 = 512.0;
const G: f64 = 9.80665;

/// The wind sea whose spectral tail is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailSea {
    /// Significant height (m) and peak period (s) of the wind sea.
    pub hs: f64,
    pub tp: f64,
    /// Direction the waves travel towards (radians, x east, y north).
    pub direction: f64,
    /// Directional spread (radians, as `SeaState::spectrum`).
    pub spread: f64,
    /// Water depth (m) for the dispersion relation.
    pub depth: f64,
    /// 10 m wind speed (m/s) for whitecap coverage; 0 = derive from Hs.
    pub wind: f64,
    pub seed: u64,
}

/// Initial amplitudes per cascade cell, `[h0(k), conj(h0(-k))]`, and the
/// statistics the shaders need.
pub struct Detail {
    pub h0: Vec<[f32; 4]>,
    /// Mean-square slope carried by each cascade.
    pub mss: [f64; CASCADES],
    /// Height variance carried by each cascade (m²).
    pub variance: [f64; CASCADES],
    /// Slope variance of waves shorter than the finest cascade resolves.
    pub unresolved_mss: f64,
    /// Whitecap coverage fraction for this sea (Monahan & O'Muircheartaigh).
    pub whitecaps: f64,
}

fn jonswap_shape(omega: f64, peak: f64) -> f64 {
    let sigma = if omega <= peak { 0.07 } else { 0.09 };
    let r = (-0.5 * ((omega / peak - 1.0) / sigma).powi(2)).exp();
    G * G * omega.powi(-5) * (-1.25 * (peak / omega).powi(4)).exp() * 3.3f64.powf(r)
}

/// JONSWAP frequency spectrum S(ω) (m² s) with α chosen so that the band
/// the physics model carries, [0.5, 2.5] ωp, holds exactly Hs²/16.
pub struct Jonswap {
    alpha: f64,
    peak: f64,
}
impl Jonswap {
    pub fn new(hs: f64, tp: f64) -> Self {
        let peak = TAU / tp.max(0.5);
        let (lo, hi) = (0.5 * peak, 2.5 * peak);
        let steps = 4000;
        let d = (hi - lo) / steps as f64;
        let band: f64 = (0..steps)
            .map(|i| jonswap_shape(lo + (i as f64 + 0.5) * d, peak) * d)
            .sum();
        Self {
            alpha: if band > 0.0 {
                hs * hs / 16.0 / band
            } else {
                0.0
            },
            peak,
        }
    }
    pub fn density(&self, omega: f64) -> f64 {
        if omega <= 0.0 {
            0.0
        } else {
            self.alpha * jonswap_shape(omega, self.peak)
        }
    }
}

fn dispersion(k: f64, depth: f64) -> (f64, f64) {
    let kd = (k * depth).min(20.0);
    let t = kd.tanh();
    let omega = (G * k * t).sqrt();
    let sech2 = 1.0 - t * t;
    let group = if omega > 0.0 {
        G * (t + kd * sech2) / (2.0 * omega)
    } else {
        0.0
    };
    (omega, group)
}

/// Highest angular frequency the physics components already represent: the
/// top component's frequency plus half a band step.
pub fn physics_cutoff(sea: &SeaState) -> f64 {
    sea.components().iter().map(|c| c.omega).fold(0.0, f64::max) * 1.03
}

/// Mean-square slope of the physics components (Σ a²k²/2).
pub fn analytic_mss(sea: &SeaState) -> f64 {
    sea.components()
        .iter()
        .map(|c| 0.5 * (c.amplitude * c.k).powi(2))
        .sum()
}

/// Wavenumber band of each cascade: from four of the next tile's waves per
/// tile up to the next band, the finest stopping well below its Nyquist.
pub fn bands() -> [(f64, f64); CASCADES] {
    let low = |c: usize| 4.0 * TAU / SIZES[c];
    let top = 0.7 * PI * N as f64 / SIZES[CASCADES - 1];
    std::array::from_fn(|c| {
        let lo = if c == 0 { 0.0 } else { low(c) };
        let hi = if c + 1 < CASCADES { low(c + 1) } else { top };
        (lo, hi)
    })
}

fn spread_exponent(spread: f64) -> f64 {
    // cos^2s(θ/2) has an rms spread of about sqrt(2/(s+1)); short waves are
    // spread more widely than the dominant waves, so halve it.
    ((2.0 / spread.max(0.05).powi(2) - 1.0) * 0.5).clamp(1.0, 12.0)
}

/// Whitecap coverage for wind speed `u` (m/s), Monahan & O'Muircheartaigh
/// (1980), capped at 20 %.
pub fn whitecap_fraction(u: f64) -> f64 {
    if !u.is_finite() || u <= 0.0 {
        0.0
    } else {
        (3.84e-6 * u.powf(3.41)).clamp(0.0, 0.2)
    }
}

impl Detail {
    /// Deterministic detail spectrum for `sea` added above `cutoff` (rad/s).
    pub fn new(sea: &DetailSea, cutoff: f64) -> Self {
        let n = N as i64;
        let mut h0 = vec![[0f32; 4]; CASCADES * N * N];
        let mut mss = [0.0; CASCADES];
        let mut variance = [0.0; CASCADES];
        let active = sea.hs > 0.0 && sea.tp >= 1.0 && sea.depth > 0.0;
        let spectrum = Jonswap::new(sea.hs, sea.tp);
        let s = spread_exponent(sea.spread);
        // Normalise cos^2s(θ/2) over (-π, π].
        let norm = {
            let steps = 720;
            let d = TAU / steps as f64;
            1.0 / (0..steps)
                .map(|i| {
                    ((-PI + (i as f64 + 0.5) * d) * 0.5)
                        .cos()
                        .abs()
                        .powf(2.0 * s)
                        * d
                })
                .sum::<f64>()
        };
        let bands = bands();
        let mut random = sea.seed ^ 0xd1b5_4a32_d192_ed03;
        let mut unit = || {
            random = random.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = random;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            (((z ^ (z >> 31)) >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        for (c, &(lo, hi)) in bands.iter().enumerate() {
            let dk = TAU / SIZES[c];
            let mut cell = vec![[0f64; 2]; N * N];
            for y in 0..n {
                for x in 0..n {
                    let kx = (if x >= n / 2 { x - n } else { x }) as f64 * dk;
                    let ky = (if y >= n / 2 { y - n } else { y }) as f64 * dk;
                    let k = kx.hypot(ky);
                    // Gaussian pair (Box-Muller) drawn for every cell so the
                    // field does not depend on which cells are active.
                    let (u1, u2) = (unit(), unit());
                    let r = (-2.0 * u1.ln()).sqrt();
                    let (g1, g2) = (r * (TAU * u2).cos(), r * (TAU * u2).sin());
                    if !active || k < lo || k >= hi || k == 0.0 {
                        continue;
                    }
                    let (omega, group) = dispersion(k, sea.depth);
                    if omega <= cutoff {
                        continue;
                    }
                    let theta = ky.atan2(kx) - sea.direction;
                    let spread = ((theta * 0.5).cos().abs()).powf(2.0 * s) * norm;
                    let density = spectrum.density(omega) * group * spread / k;
                    let p = density * dk * dk * 0.5;
                    let a = (p * 0.5).sqrt();
                    cell[(y * n + x) as usize] = [a * g1, a * g2];
                    mss[c] += 2.0 * p * k * k;
                    variance[c] += 2.0 * p;
                }
            }
            for y in 0..N {
                for x in 0..N {
                    let (mx, my) = ((N - x) % N, (N - y) % N);
                    let a = cell[y * N + x];
                    let b = cell[my * N + mx];
                    h0[(c * N + y) * N + x] = [a[0] as f32, a[1] as f32, b[0] as f32, -b[1] as f32];
                }
            }
        }
        // Slope variance above the finest band (deep water, k = ω²/g).
        let unresolved_mss = if active {
            let top = bands[CASCADES - 1].1;
            let w0 = (G * top).sqrt().max(cutoff);
            let steps = 400;
            let w1 = w0 * 8.0;
            let d = (w1 - w0) / steps as f64;
            (0..steps)
                .map(|i| {
                    let w = w0 + (i as f64 + 0.5) * d;
                    let k = w * w / G;
                    k * k * spectrum.density(w) * d
                })
                .sum()
        } else {
            0.0
        };
        let wind = if sea.wind > 0.0 {
            sea.wind
        } else {
            // Fully developed Pierson-Moskowitz: Hs = 0.21 U²/g.
            (sea.hs.max(0.0) * G / 0.21).sqrt()
        };
        Self {
            h0,
            mss,
            variance,
            unresolved_mss,
            whitecaps: if active { whitecap_fraction(wind) } else { 0.0 },
        }
    }
    /// Jacobian below which a surface element is breaking, so that the
    /// fraction of the surface below it equals the whitecap coverage. To
    /// first order the Jacobian is 1 + chop × div(D) with div(D) Gaussian of
    /// variance equal to the mean-square slope `mss`.
    pub fn foam_threshold(&self, mss: f64) -> f64 {
        if self.whitecaps <= 0.0 || mss <= 0.0 {
            return -1e3;
        }
        1.0 + CHOP * mss.sqrt() * normal_quantile(self.whitecaps)
    }
}

/// Inverse standard normal CDF (Acklam's rational approximation, |error| <
/// 1.2e-9 relative).
pub fn normal_quantile(p: f64) -> f64 {
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    const A: [f64; 6] = [
        -3.969683028665376e+01,
        2.209460984245205e+02,
        -2.759285104469687e+02,
        1.383_577_518_672_69e2,
        -3.066479806614716e+01,
        2.506628277459239e+00,
    ];
    const B: [f64; 5] = [
        -5.447609879822406e+01,
        1.615858368580409e+02,
        -1.556989798598866e+02,
        6.680131188771972e+01,
        -1.328068155288572e+01,
    ];
    const C: [f64; 6] = [
        -7.784894002430293e-03,
        -3.223964580411365e-01,
        -2.400758277161838e+00,
        -2.549732539343734e+00,
        4.374664141464968e+00,
        2.938163982698783e+00,
    ];
    const D: [f64; 4] = [
        7.784695709041462e-03,
        3.224671290700398e-01,
        2.445134137142996e+00,
        3.754408661907416e+00,
    ];
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    if p < 0.02425 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p > 1.0 - 0.02425 {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sea(hs: f64, tp: f64) -> DetailSea {
        DetailSea {
            hs,
            tp,
            direction: 0.4,
            spread: 25f64.to_radians(),
            depth: 200.0,
            wind: 0.0,
            seed: 7,
        }
    }
    #[test]
    fn physics_band_holds_the_significant_height() {
        let j = Jonswap::new(2.0, 8.0);
        let peak = TAU / 8.0;
        let steps = 20000;
        let (lo, hi) = (0.5 * peak, 2.5 * peak);
        let d = (hi - lo) / steps as f64;
        let m0: f64 = (0..steps)
            .map(|i| j.density(lo + (i as f64 + 0.5) * d) * d)
            .sum();
        assert!((4.0 * m0.sqrt() - 2.0).abs() < 1e-3, "{}", 4.0 * m0.sqrt());
    }
    #[test]
    fn cascades_carry_the_expected_tail_energy_and_nothing_below_the_cut() {
        let s = sea(2.0, 8.0);
        let physics = SeaState::spectrum(2.0, 8.0, 0.4, s.spread, s.depth, 7).unwrap();
        let cut = physics_cutoff(&physics);
        assert!(cut > 2.5 * TAU / 8.0 && cut < 2.7 * TAU / 8.0, "{cut}");
        let detail = Detail::new(&s, cut);
        // Expected tail variance from the 1-D spectrum between the cut and
        // the finest band's top (deep water).
        let j = Jonswap::new(2.0, 8.0);
        let top = (G * bands()[CASCADES - 1].1).sqrt();
        let steps = 20000;
        let d = (top - cut) / steps as f64;
        let expected: f64 = (0..steps)
            .map(|i| j.density(cut + (i as f64 + 0.5) * d) * d)
            .sum();
        let got: f64 = detail.variance.iter().sum();
        assert!(
            // Grid cells straddling the cut-off and band edges are counted
            // whole, so the discrete sum is within ~10 % of the integral.
            (got / expected - 1.0).abs() < 0.12,
            "got {got} expected {expected}"
        );
        // The tail is a few per cent of the total energy but most of the slope.
        assert!(got < 0.1 * 0.25);
        let total_mss: f64 = detail.mss.iter().sum::<f64>() + analytic_mss(&physics);
        assert!(detail.mss.iter().sum::<f64>() > 0.5 * total_mss);
        // Mean-square slope of a 2 m sea is a few hundredths (Cox & Munk
        // measured 0.02-0.05 for 5-14 m/s winds).
        assert!(total_mss > 0.01 && total_mss < 0.08, "{total_mss}");
        // Hermitian pairs: h0(-k) stored with each cell.
        let c = 1;
        let (x, y) = (5, 250);
        let a = detail.h0[(c * N + y) * N + x];
        let b = detail.h0[(c * N + (N - y)) * N + (N - x)];
        assert_eq!(a[2], b[0]);
        assert_eq!(a[3], -b[1]);
    }
    #[test]
    fn calm_sea_has_no_detail_or_foam() {
        let d = Detail::new(&sea(0.0, 6.0), 0.0);
        assert!(d.h0.iter().all(|v| *v == [0.0; 4]));
        assert_eq!(d.whitecaps, 0.0);
        assert!(d.foam_threshold(0.03) < -100.0);
    }
    #[test]
    fn foam_threshold_matches_whitecap_coverage() {
        assert!((normal_quantile(0.5)).abs() < 1e-9);
        assert!((normal_quantile(0.975) - 1.959964).abs() < 1e-5);
        assert!((normal_quantile(0.01) + 2.326348).abs() < 1e-5);
        let mut s = sea(3.0, 9.0);
        s.wind = 12.0;
        let d = Detail::new(&s, 1.8);
        let w = d.whitecaps;
        assert!(w > 0.01 && w < 0.04, "{w}");
        // Fraction of a Gaussian Jacobian below the threshold equals w.
        let mss = 0.03;
        let t = d.foam_threshold(mss);
        let sigma = CHOP * mss.sqrt();
        let z = (t - 1.0) / sigma;
        assert!((z - normal_quantile(w)).abs() < 1e-9);
    }
}
