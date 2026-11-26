//! Lifting surfaces: fixed fins, keels and skegs, rudders, hydroplanes and
//! roll fins. Each surface is a flat plate whose chord lies along body X.
//! Its normal force follows the lift-curve slope up to stall and a flat-plate
//! normal-force coefficient beyond it:
//!
//!   N = ½·ρ·S·U²·C_N(α),  C_N = a·sin α (|α| ≤ stall), else
//!   max(0.6·a·sin α_stall, 1.2·|sin α|)·sign α,
//!
//! where U is the flow speed in the plate's chord–normal plane, α the angle
//! of attack and a the lift-curve slope. Deflecting a control surface rotates
//! its plate about the span axis. Fins make streamlined hulls directionally
//! stable (against the Munk moment) and let rudders steer as on real craft.
//! Not modelled: interaction between surfaces and with the hull or propeller
//! wash, induced drag beyond the deflected normal force, and dynamic stall.
use crate::math::{Det, Vec3};
use serde::{Deserialize, Serialize};

/// Control channel of a surface.
pub const FIXED: u8 = 0;
pub const YAW: u8 = 1;
pub const PITCH: u8 = 2;
pub const ROLL: u8 = 3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fin {
    /// Centre of pressure, body frame, m.
    pub position: Vec3,
    /// Unit normal, perpendicular to body X: the direction of lift at a
    /// positive angle of attack. ±Y for a vertical fin (rudder, keel),
    /// ±Z for a horizontal fin (hydroplane).
    pub normal: Vec3,
    /// Planform area, m².
    pub area: f64,
    /// Lift-curve slope dC_L/dα, per radian.
    pub lift_slope: f64,
    /// Stall angle of attack, degrees.
    pub stall_deg: f64,
    /// 0 fixed, 1 yaw (rudder), 2 pitch (hydroplane), 3 roll.
    pub channel: u8,
    /// Largest deflection for a control surface, degrees.
    pub max_deflection_deg: f64,
}
impl Default for Fin {
    fn default() -> Self {
        Self {
            position: Vec3::new(-0.5, 0.0, 0.0),
            normal: Vec3::new(0.0, 1.0, 0.0),
            area: 0.02,
            lift_slope: lift_slope(2.0),
            stall_deg: 22.0,
            channel: FIXED,
            max_deflection_deg: 30.0,
        }
    }
}

/// Lift-curve slope of a low-aspect-ratio plate (Helmbold), per radian.
pub fn lift_slope(aspect_ratio: f64) -> f64 {
    let a = aspect_ratio.max(0.0);
    std::f64::consts::TAU * a / (2.0 + (a * a + 4.0).sqrt())
}

impl Fin {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.position.is_finite()
            || self.position.length() > 20.0
            || !self.normal.is_finite()
            || (self.normal.length() - 1.0).abs() > 1e-6
            || self.normal.x.abs() > 1e-6
            || !(1e-4..=100.0).contains(&self.area)
            || !(0.0..=10.0).contains(&self.lift_slope)
            || !(1.0..=90.0).contains(&self.stall_deg)
            || self.channel > ROLL
            || !(0.0..=60.0).contains(&self.max_deflection_deg)
        {
            return Err(
                "invalid fin: position within 20 m, unit normal perpendicular to body X, area 0.0001–100 m², lift slope 0–10 /rad, stall 1–90°, channel 0–3, deflection 0–60°",
            );
        }
        Ok(())
    }

    /// Normal-force coefficient at angle of attack `alpha` (radians).
    pub fn coefficient(&self, alpha: f64) -> f64 {
        let stall = self.stall_deg.to_radians();
        if alpha.abs() <= stall {
            self.lift_slope * alpha.dsin()
        } else {
            (0.6 * self.lift_slope * stall.dsin()).max(1.2 * alpha.dsin().abs()) * alpha.signum()
        }
    }

    /// Axis the surface's channel turns the vehicle about (zero when fixed).
    fn axis(&self) -> Vec3 {
        match self.channel {
            YAW => Vec3::new(0.0, 0.0, 1.0),
            PITCH => Vec3::new(0.0, 1.0, 0.0),
            ROLL => Vec3::new(1.0, 0.0, 0.0),
            _ => Vec3::ZERO,
        }
    }

    /// Deflection (radians) for a (roll, pitch, yaw) command in −1..1. The
    /// sign is chosen so a positive command turns the vehicle positively
    /// about the channel's axis at forward speed.
    pub fn deflection(&self, command: Vec3) -> f64 {
        let c = match self.channel {
            YAW => command.z,
            PITCH => command.y,
            ROLL => command.x,
            _ => 0.0,
        }
        .clamp(-1.0, 1.0);
        let arm = self.position.cross(self.normal).dot(self.axis());
        let sign = if arm > 1e-12 {
            1.0
        } else if arm < -1e-12 {
            -1.0
        } else {
            0.0
        };
        c * sign * self.max_deflection_deg.to_radians()
    }
}

/// Force and moment (body frame) from all surfaces. `relative` is the body
/// velocity through the water in the body frame, `omega` the body angular
/// velocity, `mass` and `inertia` the effective mass and moments of inertia
/// per body axis. A surface whose damping is too strong to integrate stably
/// at step `dt` on some axis is scaled back (a numerical safeguard that only
/// acts on extreme user values).
#[allow(clippy::too_many_arguments)]
pub fn wrench(
    fins: &[Fin],
    relative: Vec3,
    omega: Vec3,
    density: f64,
    command: Vec3,
    mass: Vec3,
    inertia: Vec3,
    dt: f64,
) -> (Vec3, Vec3) {
    let mut force = Vec3::ZERO;
    let mut moment = Vec3::ZERO;
    let chord = Vec3::new(1.0, 0.0, 0.0);
    for fin in fins {
        let (s, c) = fin.deflection(command).dsin_cos();
        let normal = fin.normal * c - chord * s;
        let span = fin.normal.cross(chord);
        let flow = relative + omega.cross(fin.position);
        // Only flow in the chord–normal plane makes lift.
        let plane = flow - span * flow.dot(span);
        let speed2 = plane.dot(plane);
        if speed2 < 1e-12 {
            continue;
        }
        let speed = speed2.sqrt();
        let alpha = (-plane.dot(normal) / speed).clamp(-1.0, 1.0).dasin();
        // Damping the plate adds per axis: linear·n_i² in translation and
        // linear·(p×n)_i² in rotation; keep each below half the stable step.
        let linear = 0.5 * density * fin.area * fin.lift_slope * speed * dt;
        let arm = fin.position.cross(normal);
        let mut scale: f64 = 1.0;
        for (k, m) in [
            (normal.x * normal.x, mass.x),
            (normal.y * normal.y, mass.y),
            (normal.z * normal.z, mass.z),
            (arm.x * arm.x, inertia.x),
            (arm.y * arm.y, inertia.y),
            (arm.z * arm.z, inertia.z),
        ] {
            if linear * k > 0.0 {
                scale = scale.min(0.5 * m / (linear * k));
            }
        }
        let f = normal * (0.5 * density * fin.area * speed2 * fin.coefficient(alpha) * scale);
        force = force + f;
        moment = moment + fin.position.cross(f);
    }
    (force, moment)
}

/// Yaw moment the yaw-channel surfaces give at full deflection when moving
/// straight ahead at `speed` (m/s) through water of `density`.
pub fn yaw_authority(fins: &[Fin], speed: f64, density: f64) -> f64 {
    let (_, moment) = wrench(
        fins,
        Vec3::new(speed, 0.0, 0.0),
        Vec3::ZERO,
        density,
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        0.0,
    );
    let (_, neutral) = wrench(
        fins,
        Vec3::new(speed, 0.0, 0.0),
        Vec3::ZERO,
        density,
        Vec3::ZERO,
        Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        0.0,
    );
    (moment.z - neutral.z).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_tail_fin_turns_the_bow_into_the_flow_and_a_rudder_steers() {
        let fin = Fin {
            position: Vec3::new(-1.0, 0.0, 0.0),
            area: 0.1,
            lift_slope: 2.0,
            ..Fin::default()
        };
        // Moving forward and sliding to port: the tail is pushed to
        // starboard, which yaws the bow to port, back into the flow.
        let (f, m) = wrench(
            std::slice::from_ref(&fin),
            Vec3::new(2.0, 0.2, 0.0),
            Vec3::ZERO,
            1025.0,
            Vec3::ZERO,
            Vec3::new(1e9, 1e9, 1e9),
            Vec3::new(1e9, 1e9, 1e9),
            0.01,
        );
        assert!(f.y < 0.0 && m.z > 0.0, "{f:?} {m:?}");
        // Small angle: N ≈ ½ρSU²·a·α.
        let alpha = (0.2_f64).atan2(2.0);
        let expected = 0.5 * 1025.0 * 0.1 * (4.0 + 0.04) * 2.0 * alpha.sin();
        assert!((f.y.abs() - expected).abs() < 1e-6 * expected);
        // A positive yaw command turns to port wherever the rudder is.
        let rudder = Fin {
            channel: YAW,
            ..fin.clone()
        };
        for x in [-1.0, 1.0] {
            let r = Fin {
                position: Vec3::new(x, 0.0, 0.0),
                normal: Vec3::new(0.0, -1.0, 0.0),
                ..rudder.clone()
            };
            assert!(yaw_authority(&[r], 2.0, 1025.0) > 0.0);
        }
        // Spanwise flow alone makes no lift; stall caps the coefficient.
        let (f, _) = wrench(
            std::slice::from_ref(&fin),
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::ZERO,
            1025.0,
            Vec3::ZERO,
            Vec3::new(1e9, 1e9, 1e9),
            Vec3::new(1e9, 1e9, 1e9),
            0.01,
        );
        assert_eq!(f, Vec3::ZERO);
        assert!(fin.coefficient(1.2) < fin.lift_slope * 1.2_f64.sin());
        // Helmbold: 2π·A/(2 + √(A² + 4)); tends to 2π for long wings.
        assert!(
            (lift_slope(1.0) - 1.4833).abs() < 1e-3,
            "{}",
            lift_slope(1.0)
        );
        assert!(std::f64::consts::TAU - lift_slope(1e6) < 1e-3);
    }
}
