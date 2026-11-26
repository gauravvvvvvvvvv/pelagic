//! Shared SI geometry; rendering and future contact/sensor backends consume it.
use crate::math::Det;
use crate::math::Vec3;
#[derive(Clone, Debug, PartialEq)]
pub struct StaticBox {
    pub center: Vec3,
    pub half: Vec3,
    pub color: [f32; 3],
}
impl StaticBox {
    pub fn ray_distance(&self, origin: Vec3, direction: Vec3) -> Option<f64> {
        let mut near: f64 = 0.0;
        let mut far = f64::INFINITY;
        for (o, d, c, h) in [
            (origin.x, direction.x, self.center.x, self.half.x),
            (origin.y, direction.y, self.center.y, self.half.y),
            (origin.z, direction.z, self.center.z, self.half.z),
        ] {
            if d.abs() < 1e-12 {
                if o < c - h || o > c + h {
                    return None;
                }
                continue;
            }
            let a = (c - h - o) / d;
            let b = (c + h - o) / d;
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
        (far >= 0.0).then_some(near)
    }
}
/// Spherical local tangent approximation. Suitable for bounded regional scenes,
/// not a replacement for a geodetic projection library over large distances.
pub struct GeoReference {
    pub latitude: f64,
    pub longitude: f64,
    pub elevation: f64,
}
impl GeoReference {
    pub fn local_to_geographic(&self, p: Vec3) -> [f64; 3] {
        [
            self.latitude + (p.y / 6371000.0).to_degrees(),
            self.longitude + (p.x / (6371000.0 * self.latitude.to_radians().dcos())).to_degrees(),
            self.elevation + p.z,
        ]
    }
    pub fn geographic_to_local(&self, p: [f64; 3]) -> Vec3 {
        Vec3::new(
            (p[1] - self.longitude).to_radians() * 6371000.0 * self.latitude.to_radians().dcos(),
            (p[0] - self.latitude).to_radians() * 6371000.0,
            p[2] - self.elevation,
        )
    }
}
