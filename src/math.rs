use std::ops::{Add, Div, Mul, Neg, Sub};

/// Bit-reproducible elementary functions for the simulation. The platform
/// maths library can differ in the last bit between executables, linkers
/// and operating systems, which breaks exact re-simulation of recordings;
/// these use the portable musl implementations (`libm`) instead. Square
/// root and basic arithmetic are already exactly specified by IEEE 754.
/// Rendering code keeps the faster platform functions.
pub trait Det: Copy {
    fn dsin(self) -> f64;
    fn dcos(self) -> f64;
    fn dsin_cos(self) -> (f64, f64);
    fn dtan(self) -> f64;
    fn dexp(self) -> f64;
    fn dln(self) -> f64;
    fn dpowf(self, exponent: f64) -> f64;
    fn datan2(self, x: f64) -> f64;
    fn dacos(self) -> f64;
    fn dasin(self) -> f64;
    fn dtanh(self) -> f64;
}
impl Det for f64 {
    fn dsin(self) -> f64 {
        libm::sin(self)
    }
    fn dcos(self) -> f64 {
        libm::cos(self)
    }
    fn dsin_cos(self) -> (f64, f64) {
        libm::sincos(self)
    }
    fn dtan(self) -> f64 {
        libm::tan(self)
    }
    fn dexp(self) -> f64 {
        libm::exp(self)
    }
    fn dln(self) -> f64 {
        libm::log(self)
    }
    fn dpowf(self, exponent: f64) -> f64 {
        libm::pow(self, exponent)
    }
    fn datan2(self, x: f64) -> f64 {
        libm::atan2(self, x)
    }
    fn dacos(self) -> f64 {
        libm::acos(self)
    }
    fn dasin(self) -> f64 {
        libm::asin(self)
    }
    fn dtanh(self) -> f64 {
        libm::tanh(self)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    pub fn dot(self, b: Self) -> f64 {
        self.x * b.x + self.y * b.y + self.z * b.z
    }
    pub fn cross(self, b: Self) -> Self {
        Self::new(
            self.y * b.z - self.z * b.y,
            self.z * b.x - self.x * b.z,
            self.x * b.y - self.y * b.x,
        )
    }
    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn normalized(self) -> Self {
        let n = self.length();
        if n > 0.0 { self / n } else { Self::ZERO }
    }
    pub fn component_mul(self, b: Self) -> Self {
        Self::new(self.x * b.x, self.y * b.y, self.z * b.z)
    }
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs())
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}
impl Add for Vec3 {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::new(self.x + b.x, self.y + b.y, self.z + b.z)
    }
}
impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self::new(self.x - b.x, self.y - b.y, self.z - b.z)
    }
}
impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, b: f64) -> Self {
        Self::new(self.x * b, self.y * b, self.z * b)
    }
}
impl Div<f64> for Vec3 {
    type Output = Self;
    fn div(self, b: f64) -> Self {
        self * (1.0 / b)
    }
}
impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        self * -1.0
    }
}

/// Unit quaternion, body to world rotation.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Quat {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}
impl Quat {
    pub const IDENTITY: Self = Self {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub fn normalized(self) -> Self {
        let n = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        Self {
            w: self.w / n,
            x: self.x / n,
            y: self.y / n,
            z: self.z / n,
        }
    }
    pub fn conjugate(self) -> Self {
        Self {
            w: self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
    pub fn rotate(self, v: Vec3) -> Vec3 {
        let q = Vec3::new(self.x, self.y, self.z);
        let t = q.cross(v) * 2.0;
        v + t * self.w + q.cross(t)
    }
    pub fn from_rotation_vector(v: Vec3) -> Self {
        let angle = v.length();
        if angle < 1e-12 {
            return Self::IDENTITY;
        }
        let s = (angle * 0.5).dsin() / angle;
        Self {
            w: (angle * 0.5).dcos(),
            x: v.x * s,
            y: v.y * s,
            z: v.z * s,
        }
    }
}
impl Mul for Quat {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        Self {
            w: self.w * b.w - self.x * b.x - self.y * b.y - self.z * b.z,
            x: self.w * b.x + self.x * b.w + self.y * b.z - self.z * b.y,
            y: self.w * b.y - self.x * b.z + self.y * b.w + self.z * b.x,
            z: self.w * b.z + self.x * b.y - self.y * b.x + self.z * b.w,
        }
    }
}
