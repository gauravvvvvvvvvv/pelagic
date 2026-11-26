//! Bounded regular current grids for adapters. SI units; Z up. Out-of-domain
//! queries clamp to the nearest sample in space and time, never extrapolate.
use crate::{math::Vec3, ocean::CurrentField};

#[derive(Clone, Debug)]
pub struct CurrentGrid {
    shape: [usize; 3],
    origin: Vec3,
    spacing: Vec3,
    times: Vec<f64>,
    /// X varies fastest, then Y, Z, time.
    values: Vec<Vec3>,
}
impl CurrentGrid {
    pub fn new(
        shape: [usize; 3],
        origin: Vec3,
        spacing: Vec3,
        times: Vec<f64>,
        values: Vec<Vec3>,
    ) -> Result<Self, &'static str> {
        let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        let count = shape.iter().try_fold(times.len(), |n, &s| n.checked_mul(s));
        if shape.iter().any(|&s| s < 2)
            || count != Some(values.len())
            || values.len() > 4_194_304
            || !finite(origin)
            || !finite(spacing)
            || spacing.x <= 0.0
            || spacing.y <= 0.0
            || spacing.z <= 0.0
            || times.is_empty()
            || times.iter().any(|t| !t.is_finite())
            || times.windows(2).any(|t| t[0] >= t[1])
            || values.iter().any(|&v| !finite(v))
        {
            return Err("invalid current grid dimensions, coordinates, timestamps or velocities");
        }
        Ok(Self {
            shape,
            origin,
            spacing,
            times,
            values,
        })
    }
    fn spatial(&self, p: Vec3, t: usize) -> Vec3 {
        let axis = |p: f64, o: f64, s: f64, n: usize| {
            let q = ((p - o) / s).clamp(0.0, (n - 1) as f64);
            let i = (q as usize).min(n - 2);
            (i, q - i as f64)
        };
        let (x, fx) = axis(p.x, self.origin.x, self.spacing.x, self.shape[0]);
        let (y, fy) = axis(p.y, self.origin.y, self.spacing.y, self.shape[1]);
        let (z, fz) = axis(p.z, self.origin.z, self.spacing.z, self.shape[2]);
        let mut v = Vec3::ZERO;
        for dz in 0..2 {
            for dy in 0..2 {
                for dx in 0..2 {
                    let weight = if dx == 0 { 1.0 - fx } else { fx }
                        * if dy == 0 { 1.0 - fy } else { fy }
                        * if dz == 0 { 1.0 - fz } else { fz };
                    let i = (((t * self.shape[2] + z + dz) * self.shape[1] + y + dy)
                        * self.shape[0])
                        + x
                        + dx;
                    v = v + self.values[i] * weight;
                }
            }
        }
        v
    }
}
impl CurrentField for CurrentGrid {
    fn velocity(&self, p: Vec3, time: f64) -> Vec3 {
        if ![p.x, p.y, p.z, time].iter().all(|v| v.is_finite()) {
            return Vec3::ZERO;
        }
        let hi = self
            .times
            .partition_point(|t| *t < time)
            .min(self.times.len() - 1);
        if hi == 0 {
            return self.spatial(p, 0);
        }
        let lo = hi - 1;
        let f = ((time - self.times[lo]) / (self.times[hi] - self.times[lo])).clamp(0.0, 1.0);
        self.spatial(p, lo) * (1.0 - f) + self.spatial(p, hi) * f
    }
}
