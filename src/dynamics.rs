//! Bounded engineering approximations: diagonal added mass, sampled immersion,
//! lagged/saturated force actuators, depth/heading PD and box contact proxies.
use crate::{math::Vec3, ocean::Ocean, physics::Vehicle};
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BodyModel {
    #[serde(default)]
    pub thrusters: Vec<crate::actuators::Thruster>,
    /// Fins, keels, rudders and hydroplanes (lifting surfaces).
    #[serde(default)]
    pub fins: Vec<crate::fins::Fin>,
    pub half: Vec3,
    pub added_mass: Vec3,
    /// Added (fluid) moments of inertia about body X/Y/Z, kg·m².
    #[serde(default)]
    pub added_inertia: Vec3,
    pub center_buoyancy: Vec3,
    pub partial_buoyancy: bool,
    pub profile_density: bool,
    pub contacts: bool,
    pub thrust_limit: f64,
    pub response_seconds: f64,
    pub hold_depth: Option<f64>,
    pub hold_heading: Option<f64>,
    /// Principal moments of inertia about body X/Y/Z, kg·m². `None` uses a
    /// solid box of the body's size and mass.
    pub inertia: Option<Vec3>,
    /// Quadratic translational drag per body axis, N/(m/s)². `None` scales a
    /// reference ROV's drag by mass.
    pub drag: Option<Vec3>,
    /// Quadratic rotational drag per body axis, N·m/(rad/s)². `None` scales a
    /// reference ROV's value by mass.
    pub angular_drag: Option<Vec3>,
}
impl Default for BodyModel {
    fn default() -> Self {
        Self {
            thrusters: vec![],
            fins: vec![],
            half: Vec3::new(0.65, 0.42, 0.4),
            added_mass: Vec3::ZERO,
            added_inertia: Vec3::ZERO,
            center_buoyancy: Vec3::ZERO,
            partial_buoyancy: false,
            profile_density: false,
            contacts: false,
            thrust_limit: 1e6,
            response_seconds: 0.0,
            hold_depth: None,
            hold_heading: None,
            inertia: None,
            drag: None,
            angular_drag: None,
        }
    }
}
impl BodyModel {
    pub fn rov() -> Self {
        Self {
            thrusters: crate::actuators::rov(),
            added_mass: Vec3::new(2.0, 5.0, 6.0),
            center_buoyancy: Vec3::new(0.0, 0.0, 0.05),
            partial_buoyancy: true,
            profile_density: true,
            contacts: true,
            thrust_limit: 40.0,
            response_seconds: 0.15,
            ..Default::default()
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.thrusters.len() > 32 {
            return Err("body supports at most 32 thrusters");
        }
        for t in &self.thrusters {
            t.validate()?;
        }
        if self.fins.len() > 16 {
            return Err("body supports at most 16 fins");
        }
        for f in &self.fins {
            f.validate()?;
        }
        if !self.half.is_finite()
            || !self.added_mass.is_finite()
            || !self.center_buoyancy.is_finite()
            || [self.half.x, self.half.y, self.half.z]
                .iter()
                .any(|v| *v < 0.02 || *v > 20.0)
            || [self.added_mass.x, self.added_mass.y, self.added_mass.z]
                .iter()
                .any(|v| *v < 0.0 || *v > 1e5)
            || !self.added_inertia.is_finite()
            || [
                self.added_inertia.x,
                self.added_inertia.y,
                self.added_inertia.z,
            ]
            .iter()
            .any(|v| *v < 0.0 || *v > 1e10)
            || !self.thrust_limit.is_finite()
            || !(0.0..=1e6).contains(&self.thrust_limit)
            || !self.response_seconds.is_finite()
            || !(0.0..=60.0).contains(&self.response_seconds)
            || self.center_buoyancy.length() > 10.0
            || self
                .hold_depth
                .is_some_and(|v| !v.is_finite() || !(0.0..=11000.0).contains(&v))
            || self.hold_heading.is_some_and(|v| !v.is_finite())
        {
            return Err("invalid body/actuator/controller model");
        }
        let within = |v: Option<Vec3>, lo: f64, hi: f64| {
            v.is_none_or(|v| v.is_finite() && [v.x, v.y, v.z].iter().all(|c| (lo..=hi).contains(c)))
        };
        if !within(self.inertia, 1e-4, 1e10)
            || !within(self.drag, 0.0, 1e8)
            || !within(self.angular_drag, 0.0, 1e8)
        {
            return Err("invalid inertia or drag coefficients");
        }
        Ok(())
    }
}
pub fn world_half(v: &Vehicle) -> Vec3 {
    let q = v.orientation;
    let h = v.model.half;
    q.rotate(Vec3::new(h.x, 0.0, 0.0)).abs()
        + q.rotate(Vec3::new(0.0, h.y, 0.0)).abs()
        + q.rotate(Vec3::new(0.0, 0.0, h.z)).abs()
}
pub fn immersion(v: &Vehicle, o: &Ocean, t: f64) -> f64 {
    if !v.model.partial_buoyancy {
        return 1.0;
    }
    let half = world_half(v);
    let mut fraction = 0.0;
    for x in [-0.8, 0.0, 0.8] {
        for y in [-0.8, 0.0, 0.8] {
            let surface = o
                .surface(v.position.x + x * half.x, v.position.y + y * half.y, t)
                .height;
            fraction += ((surface - v.position.z + half.z) / (2.0 * half.z)).clamp(0.0, 1.0);
        }
    }
    fraction / 9.0
}
/// Hydrostatics of a body crossing the free surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waterline {
    /// Submerged fraction of the displaced volume (0..1).
    pub fraction: f64,
    /// Buoyant force, world +Z, newtons.
    pub force: f64,
    /// Buoyancy moment about the centre of mass, body frame, N·m.
    pub torque: Vec3,
    /// Waterplane stiffness in heave (N/m) and roll/pitch (N·m/rad).
    pub heave_stiffness: f64,
    pub roll_stiffness: f64,
    pub pitch_stiffness: f64,
}
const COLUMNS_X: usize = 4;
const COLUMNS_Y: usize = 3;
/// Integrates buoyancy over a 4×3 grid of vertical body columns against the
/// local wave surface, so a vessel heaves, pitches and rolls with the waves
/// and has a waterplane righting moment. Returns `None` unless the body
/// crosses the surface (fully submerged or emerged bodies keep the lumped
/// model, unchanged). A box-hull strip approximation: no hull shape
/// coefficients, diffraction or radiation forces beyond linear damping.
pub fn waterline(v: &Vehicle, o: &Ocean, t: f64, density: f64) -> Option<Waterline> {
    if !v.model.partial_buoyancy {
        return None;
    }
    let h = v.model.half;
    let q = v.orientation;
    let share = 1.0 / (COLUMNS_X * COLUMNS_Y) as f64;
    // Effective waterplane of one column: buoyancy grows by ρ·g·V/(2·half.z)
    // per metre of immersion, so stiffness (and the damping derived from it)
    // uses V/(2·half.z), not the box footprint.
    let area = v.volume() / (2.0 * h.z) * share;
    let lift = density * v.volume() * o.gravity * share;
    let offset = q.rotate(v.model.center_buoyancy);
    let (mut partial, mut wet, mut dry) = (false, false, false);
    let mut w = Waterline {
        fraction: 0.0,
        force: 0.0,
        torque: Vec3::ZERO,
        heave_stiffness: 0.0,
        roll_stiffness: 0.0,
        pitch_stiffness: 0.0,
    };
    let mut moment = Vec3::ZERO;
    for i in 0..COLUMNS_X {
        for j in 0..COLUMNS_Y {
            let x = ((i as f64 + 0.5) / COLUMNS_X as f64 * 2.0 - 1.0) * h.x;
            let y = ((j as f64 + 0.5) / COLUMNS_Y as f64 * 2.0 - 1.0) * h.y;
            let a = v.position + q.rotate(Vec3::new(x, y, -h.z));
            let b = v.position + q.rotate(Vec3::new(x, y, h.z));
            let (low, high) = if a.z <= b.z { (a, b) } else { (b, a) };
            let mid = (low + high) * 0.5;
            let surface = o.surface(mid.x, mid.y, t).height;
            let f = ((surface - low.z) / (high.z - low.z).max(1e-9)).clamp(0.0, 1.0);
            if f > 0.0 && f < 1.0 {
                partial = true;
                w.heave_stiffness += area;
                w.roll_stiffness += area * y * y;
                w.pitch_stiffness += area * x * x;
            }
            wet |= f >= 1.0;
            dry |= f <= 0.0;
            w.fraction += f * share;
            let force = lift * f;
            w.force += force;
            // Centroid of the submerged part of the column.
            let point = low + (high - low) * (0.5 * f) + offset;
            moment = moment + (point - v.position).cross(Vec3::new(0.0, 0.0, force));
        }
    }
    if !partial && !(wet && dry) {
        return None;
    }
    let k = density * o.gravity;
    w.heave_stiffness *= k;
    w.roll_stiffness *= k;
    w.pitch_stiffness *= k;
    w.torque = q.conjugate().rotate(moment);
    Some(w)
}
/// Fraction of critical damping applied in heave, roll and pitch while a
/// body is in the waterline (stands in for wave-radiation damping).
pub const WATERLINE_DAMPING: f64 = 0.12;
fn respond(v: &mut Vehicle, n: Vec3, penetration: f64) {
    v.position = v.position + n * penetration;
    let speed = v.velocity.dot(n);
    if speed < 0.0 {
        v.velocity = v.velocity - n * (1.05 * speed);
    }
    let tangent = v.velocity - n * v.velocity.dot(n);
    v.velocity = v.velocity - tangent * 0.08;
    v.angular_velocity = v.angular_velocity * 0.92;
}
pub(crate) fn contact(v: &mut Vehicle, o: &Ocean) {
    if !v.model.contacts {
        return;
    }
    let h = world_half(v);
    let mut deepest = 0.0;
    let mut normal = Vec3::new(0.0, 0.0, 1.0);
    for x in [-1.0, 0.0, 1.0] {
        for y in [-1.0, 0.0, 1.0] {
            let (x, y) = (v.position.x + x * h.x, v.position.y + y * h.y);
            let penetration = o.bottom.elevation(x, y) - (v.position.z - h.z);
            if penetration > deepest {
                deepest = penetration;
                let e = 0.3;
                normal = Vec3::new(
                    o.bottom.elevation(x - e, y) - o.bottom.elevation(x + e, y),
                    o.bottom.elevation(x, y - e) - o.bottom.elevation(x, y + e),
                    2.0 * e,
                )
                .normalized();
            }
        }
    }
    if deepest > 0.0 {
        respond(v, normal, deepest / normal.z.max(0.2));
    }
    for b in &o.structures {
        let d = v.position - b.center;
        let e = h + b.half;
        let overlap = e - d.abs();
        if overlap.x > 0.0 && overlap.y > 0.0 && overlap.z > 0.0 {
            let (n, p) = if overlap.x <= overlap.y && overlap.x <= overlap.z {
                (
                    Vec3::new(if d.x >= 0.0 { 1.0 } else { -1.0 }, 0.0, 0.0),
                    overlap.x,
                )
            } else if overlap.y <= overlap.z {
                (
                    Vec3::new(0.0, if d.y >= 0.0 { 1.0 } else { -1.0 }, 0.0),
                    overlap.y,
                )
            } else {
                (
                    Vec3::new(0.0, 0.0, if d.z >= 0.0 { 1.0 } else { -1.0 }),
                    overlap.z,
                )
            };
            respond(v, n, p + 1e-6);
        }
    }
}
pub(crate) fn pairs(vehicles: &mut [Vehicle]) {
    // Bounded fleet (desktop ≤16). Sweep-and-prune rejects separated X intervals.
    let mut order: Vec<usize> = (0..vehicles.len())
        .filter(|&i| vehicles[i].model.contacts)
        .collect();
    order.sort_by(|&a, &b| {
        (vehicles[a].position.x - world_half(&vehicles[a]).x)
            .total_cmp(&(vehicles[b].position.x - world_half(&vehicles[b]).x))
    });
    for i in 0..order.len() {
        for j in i + 1..order.len() {
            let (ia, ib) = (order[i], order[j]);
            let ha = world_half(&vehicles[ia]);
            let hb = world_half(&vehicles[ib]);
            if vehicles[ib].position.x - hb.x > vehicles[ia].position.x + ha.x {
                break;
            }
            let d = vehicles[ia].position - vehicles[ib].position;
            let overlap = ha + hb - d.abs();
            if overlap.x <= 0.0 || overlap.y <= 0.0 || overlap.z <= 0.0 {
                continue;
            }
            let (n, p) = if overlap.x <= overlap.y && overlap.x <= overlap.z {
                (Vec3::new(d.x.signum(), 0.0, 0.0), overlap.x)
            } else if overlap.y <= overlap.z {
                (Vec3::new(0.0, d.y.signum(), 0.0), overlap.y)
            } else {
                (
                    Vec3::new(0.0, 0.0, if d.z == 0.0 { 1.0 } else { d.z.signum() }),
                    overlap.z,
                )
            };
            let n = if n.length() < 0.5 {
                Vec3::new(1.0, 0.0, 0.0)
            } else {
                n
            };
            vehicles[ia].position = vehicles[ia].position + n * (p * 0.5 + 1e-6);
            vehicles[ib].position = vehicles[ib].position - n * (p * 0.5 + 1e-6);
            let relative = (vehicles[ia].velocity - vehicles[ib].velocity).dot(n);
            if relative < 0.0 {
                let impulse =
                    -(1.05 * relative) / (1.0 / vehicles[ia].mass() + 1.0 / vehicles[ib].mass());
                let ma = vehicles[ia].mass();
                let mb = vehicles[ib].mass();
                vehicles[ia].velocity = vehicles[ia].velocity + n * (impulse / ma);
                vehicles[ib].velocity = vehicles[ib].velocity - n * (impulse / mb);
            }
        }
    }
}
