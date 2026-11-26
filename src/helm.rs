//! A helm for guided runs (survey lines, encounter scenarios): holds an
//! ordered heading and speed through the water with the vehicle's own
//! rudders and thrusters.
//!
//! - Heading: a PD loop sized from what the vehicle can deliver — full
//!   rudder plus thruster yaw moment at [`FULL_ERROR`] of heading error,
//!   damping from the rigid plus added yaw inertia. Rudder authority grows
//!   with speed squared, so the loop stiffens with speed as a real helm
//!   does. (A loop tuned from rigid inertia alone turned a finned AUV
//!   wide and could not hold a directionally unstable boat.)
//! - Speed: PI on speed through the water, limited to 80 % of the forward
//!   thrust so depth keeping and steering keep some authority (thrusters
//!   share one limit); astern thrust only when allowed.
use crate::{
    fins::{self, Fin},
    trials::{Bench, Rig},
};

/// Heading error (rad) at which the helm uses full yaw authority.
pub(crate) const FULL_ERROR: f64 = 0.35;
const DT: f64 = 1.0 / 60.0;

pub(crate) struct Helm {
    rudders: Vec<Fin>,
    yaw_inertia: f64,
    thruster_yaw: f64,
    ahead: f64,
    astern: f64,
    kp: f64,
    ki: f64,
    integral: f64,
    density: f64,
}

impl Helm {
    pub(crate) fn new(bench: &Bench, density: f64, allow_astern: bool) -> Self {
        let e = &bench.entity;
        let mass = e.mass;
        Self {
            rudders: e
                .model
                .fins
                .iter()
                .filter(|f| f.channel == fins::YAW)
                .cloned()
                .collect(),
            yaw_inertia: e.effective_inertia().z + e.model.added_inertia.z.max(0.0),
            thruster_yaw: bench.yaw,
            ahead: 0.8 * bench.forward,
            astern: if allow_astern {
                0.8 * bench.astern
            } else {
                0.0
            },
            kp: mass / 4.0,
            ki: mass / 60.0,
            integral: 0.0,
            density,
        }
    }

    /// Starts the speed loop at `thrust` (N), e.g. the thrust that holds
    /// the speed a run starts at.
    pub(crate) fn prime(&mut self, thrust: f64) {
        self.integral = thrust.clamp(-self.astern, self.ahead) / self.ki;
    }

    /// (surge thrust, (yaw moment, rudder)) steering to `heading` (scene
    /// angle, radians) at `speed` (m/s through the water).
    pub(crate) fn command(&mut self, rig: &Rig, heading: f64, speed: f64) -> (f64, (f64, f64)) {
        let wrap = |a: f64| {
            (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
        };
        let error = wrap(heading - rig.heading);
        let fin = if self.rudders.is_empty() {
            0.0
        } else {
            fins::yaw_authority(&self.rudders, rig.speed().max(0.2), self.density)
        };
        let k = (fin + self.thruster_yaw) / FULL_ERROR;
        let d = 2.0 * 0.9 * (k * self.yaw_inertia).sqrt();
        let wanted = k * error - d * rig.vehicle().angular_velocity.z;
        let rudder = if fin > 0.0 {
            (wanted / fin).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let moment = (wanted - rudder * fin).clamp(-self.thruster_yaw, self.thruster_yaw);
        // Signed speed through the water along the body axis.
        let e = speed - rig.body_velocity().x;
        let (lo, hi) = (-self.astern, self.ahead);
        self.integral = (self.integral + e * DT).clamp(lo / self.ki, hi / self.ki);
        let thrust = (self.kp * e + self.ki * self.integral).clamp(lo, hi);
        (thrust, (moment, rudder))
    }
}
