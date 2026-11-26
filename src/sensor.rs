use crate::{math::Vec3, ocean::Ocean};

#[derive(Clone, Copy, Debug)]
pub struct Echo {
    pub range: f64,
    pub round_trip_seconds: f64,
    pub intensity: f64,
}

/// Reference bottom-only sonar. Marches along a straight ray then bisects the hit.
/// No refraction, multipath, target geometry, beam spread or stochastic speckle.
pub fn bottom_echo(
    ocean: &Ocean,
    origin: Vec3,
    direction: Vec3,
    max_range: f64,
    spacing: f64,
) -> Option<Echo> {
    if !origin.is_finite()
        || !direction.is_finite()
        || direction.length() == 0.0
        || !max_range.is_finite()
        || max_range <= 0.0
        || !spacing.is_finite()
        || spacing <= 0.0
    {
        return None;
    }
    let dir = direction.normalized();
    let above = |r: f64| {
        let p = origin + dir * r;
        p.z - ocean.bottom.elevation(p.x, p.y)
    };
    if above(0.0) <= 0.0 {
        return None;
    }
    let mut low = 0.0;
    while low < max_range {
        let mut high = (low + spacing).min(max_range);
        if above(high) <= 0.0 {
            for _ in 0..32 {
                let mid = (low + high) * 0.5;
                if above(mid) > 0.0 {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            let range = (low + high) * 0.5;
            return Some(Echo {
                range,
                round_trip_seconds: 2.0 * range / ocean.sound_speed,
                intensity: 1.0 / (1.0 + range * range),
            });
        }
        low = high;
    }
    None
}
