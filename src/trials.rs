//! Standard trials for a vehicle under test: free decay (natural periods and
//! damping), speed against thrust, turning circles, zig-zag manoeuvres, crash
//! stop and response in regular waves. Each trial runs headless in a
//! controlled environment (deep calm water by default, optionally with a
//! steady current and an irregular sea during manoeuvres), measures what the
//! standard procedures measure and judges the results against optional
//! acceptance criteria. Trials are deterministic and run in parallel.
//!
//! Conventions: world Z up; the vehicle starts heading along +X. Heading
//! increases to port (anticlockwise seen from above). Roll is positive
//! starboard side down and pitch positive bow down (right-handed about body
//! X forward and Y to port).
//!
//! Steering: a vehicle with yaw-channel fins (rudders) is steered by rudder
//! angle, as in ship trials (turning circle at the trial rudder angle,
//! zig-zag rudder = switching angle). Otherwise the yaw command is a yaw
//! moment through the thrusters (or applied directly to a vehicle without
//! thrusters), as a fraction of its yaw authority. IMO manoeuvring criteria
//! are defined for ships steered by rudders at full scale; they are offered
//! as reference limits only.
use crate::{
    World, actuators,
    definition::VehicleDefinition,
    entities::Entity,
    fins::{self, Fin},
    math::{Det, Quat, Vec3},
    ocean::{FlatBottom, Ocean, UniformCurrent},
    physics::Vehicle,
    waves::{SeaState, wave_number},
};
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, TAU};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

pub const FORMAT: &str = "pelagic-trials";
const DT: f64 = 1.0 / 60.0;
/// Natural frequency (rad/s) and damping ratio of the trial heading autopilot.
const AUTOPILOT: (f64, f64) = (0.4, 0.9);

/// Where the trials run. Free decay and wave-response trials always use
/// still water; the current and the irregular sea apply to manoeuvres.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    /// Water density, kg/m³.
    pub density: f64,
    /// Water depth over a flat seabed, m.
    pub water_depth: f64,
    /// Steady uniform current, world frame, m/s.
    pub current: [f64; 3],
    /// Irregular sea during manoeuvres: significant height (m, 0 = calm),
    /// peak period (s), propagation direction (degrees from +X) and spread.
    pub wave_height: f64,
    pub wave_period: f64,
    pub wave_direction: f64,
    pub wave_spread: f64,
    pub seed: u64,
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            density: 1025.0,
            water_depth: 200.0,
            current: [0.0; 3],
            wave_height: 0.0,
            wave_period: 6.0,
            wave_direction: 180.0,
            wave_spread: 25.0,
            seed: 1,
        }
    }
}
impl Environment {
    /// The conditions of a scene: density, depth, surface current and sea.
    pub fn from_scene(scene: &crate::scene::Scene) -> Self {
        Self {
            density: scene.density,
            water_depth: scene.bottom_depth.clamp(5.0, 11000.0),
            current: scene.current,
            wave_height: scene.wave_height,
            wave_period: scene.wave_period,
            wave_direction: scene.wave_direction,
            wave_spread: scene.wave_spread,
            seed: scene.seed,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let current = Vec3::new(self.current[0], self.current[1], self.current[2]);
        if !(900.0..=1100.0).contains(&self.density)
            || !(5.0..=11000.0).contains(&self.water_depth)
            || !current.is_finite()
            || current.length() > 5.0
            || !(0.0..=15.0).contains(&self.wave_height)
            || !(1.0..=30.0).contains(&self.wave_period)
            || !self.wave_direction.is_finite()
            || !(0.0..=180.0).contains(&self.wave_spread)
        {
            return Err("Trial environment: density 900–1100 kg/m³, depth 5–11000 m, current up to 5 m/s, Hs 0–15 m, period 1–30 s.".into());
        }
        Ok(())
    }
    fn current(&self) -> Vec3 {
        Vec3::new(self.current[0], self.current[1], self.current[2])
    }
}

/// Which trials to run and their parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    pub free_decay: bool,
    pub speed: bool,
    pub turning: bool,
    pub zigzag: bool,
    pub stopping: bool,
    pub waves: bool,
    /// Forward thrust levels for the speed trial, fractions of capability.
    pub speed_levels: Vec<f64>,
    /// Approach thrust for turning, zig-zag and stopping, fraction of capability.
    pub approach_thrust: f64,
    /// Thruster-steered vehicles: yaw command for turning and zig-zag,
    /// fraction of yaw authority.
    pub yaw_command: f64,
    /// Rudder-steered vehicles: rudder angle for the turning circle, degrees
    /// (limited to the rudders' largest deflection).
    pub rudder_deg: f64,
    /// Zig-zag switching angles (heading change = command angle), degrees.
    pub zigzag_angles: Vec<f64>,
    /// Regular-wave height, crest to trough, m.
    pub wave_height: f64,
    /// Regular-wave periods, s.
    pub wave_periods: Vec<f64>,
    /// Relative wave headings, degrees (180 head seas, 90 beam seas from
    /// starboard, 0 following seas).
    pub wave_headings: Vec<f64>,
    /// Depth at which submerged vehicles are tested, m.
    pub operating_depth: f64,
}
impl Default for Setup {
    fn default() -> Self {
        Self {
            free_decay: true,
            speed: true,
            turning: true,
            zigzag: true,
            stopping: true,
            waves: true,
            speed_levels: vec![0.25, 0.5, 0.75, 1.0],
            approach_thrust: 1.0,
            yaw_command: 0.5,
            rudder_deg: 35.0,
            zigzag_angles: vec![10.0, 20.0],
            wave_height: 0.5,
            wave_periods: vec![2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0],
            wave_headings: vec![180.0, 90.0],
            operating_depth: 20.0,
        }
    }
}
impl Setup {
    /// A short set for previews and automated checks.
    pub fn quick() -> Self {
        Self {
            speed_levels: vec![0.5, 1.0],
            zigzag_angles: vec![10.0],
            wave_periods: vec![3.0, 6.0, 10.0],
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let fraction = |v: &f64| v.is_finite() && *v > 0.0 && *v <= 1.0;
        if self.speed_levels.len() > 10 || !self.speed_levels.iter().all(fraction) {
            return Err("Speed levels: up to 10 thrust fractions in (0, 1].".into());
        }
        if !fraction(&self.approach_thrust) || !fraction(&self.yaw_command) {
            return Err("Approach thrust and yaw command are fractions in (0, 1].".into());
        }
        if !(1.0..=60.0).contains(&self.rudder_deg) {
            return Err("Turning-circle rudder angle must be 1–60°.".into());
        }
        if self.zigzag_angles.len() > 4
            || !self.zigzag_angles.iter().all(|a| (1.0..=45.0).contains(a))
        {
            return Err("Zig-zag: up to 4 angles of 1–45°.".into());
        }
        if !(0.02..=5.0).contains(&self.wave_height)
            || self.wave_periods.len() > 24
            || !self.wave_periods.iter().all(|t| (1.0..=30.0).contains(t))
            || self.wave_headings.len() > 8
            || !self.wave_headings.iter().all(|h| h.is_finite())
        {
            return Err(
                "Waves: height 0.02–5 m, up to 24 periods of 1–30 s, up to 8 headings.".into(),
            );
        }
        if !(0.5..=11000.0).contains(&self.operating_depth) {
            return Err("Operating depth must be 0.5–11000 m.".into());
        }
        Ok(())
    }
}

/// Optional pass/fail limits. `None` disables a check.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Criteria {
    /// Smallest speed at the highest thrust level, m/s.
    pub min_speed_mps: Option<f64>,
    /// Largest advance (port or starboard), vehicle lengths.
    pub max_advance_lengths: Option<f64>,
    /// Largest tactical diameter, vehicle lengths.
    pub max_tactical_diameter_lengths: Option<f64>,
    /// Largest distance travelled before the 10° zig-zag's first command
    /// change, vehicle lengths (initial turning ability).
    pub max_initial_turning_lengths: Option<f64>,
    /// 10°/10° zig-zag overshoot limits, degrees.
    pub max_first_overshoot_deg: Option<f64>,
    pub max_second_overshoot_deg: Option<f64>,
    /// 20°/20° zig-zag first overshoot limit, degrees.
    pub max_first_overshoot_20_deg: Option<f64>,
    /// Largest stopping track reach, vehicle lengths.
    pub max_stopping_track_lengths: Option<f64>,
    /// Smallest roll damping ratio from free decay.
    pub min_roll_damping: Option<f64>,
    /// Largest roll response in regular waves, degrees per degree of wave slope.
    pub max_roll_rao: Option<f64>,
}
impl Criteria {
    /// IMO MSC.137(76) reference values for a ship of `length` (m) at
    /// `speed` (m/s): advance ≤ 4.5 L, tactical diameter ≤ 5 L, initial
    /// turning ≤ 2.5 L, 10°/10° overshoots by L/V, 20°/20° first overshoot
    /// ≤ 25°, stopping track reach ≤ 15 L.
    pub fn imo(length: f64, speed: f64) -> Self {
        let lv = length / speed.max(1e-6);
        let (first, second) = if lv < 10.0 {
            (10.0, 25.0)
        } else if lv >= 30.0 {
            (20.0, 40.0)
        } else {
            (5.0 + 0.5 * lv, 17.5 + 0.75 * lv)
        };
        Self {
            max_advance_lengths: Some(4.5),
            max_tactical_diameter_lengths: Some(5.0),
            max_initial_turning_lengths: Some(2.5),
            max_first_overshoot_deg: Some(first),
            max_second_overshoot_deg: Some(second),
            max_first_overshoot_20_deg: Some(25.0),
            max_stopping_track_lengths: Some(15.0),
            ..Self::default()
        }
    }
}

/// Free-decay result for one degree of freedom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decay {
    /// "heave", "roll" or "pitch".
    pub axis: String,
    /// "m" or "deg".
    pub unit: String,
    /// Initial displacement from equilibrium.
    pub initial: f64,
    /// Value the motion settles to.
    pub equilibrium: f64,
    /// Damped natural period, s; `None` when the motion does not oscillate.
    pub period_s: Option<f64>,
    /// Damping ratio from the logarithmic decrement (1 when overdamped).
    pub damping_ratio: Option<f64>,
    /// Closed-form linear estimate of the undamped period, where one applies.
    pub predicted_period_s: Option<f64>,
    /// Roll and pitch: metacentric height implied by the measured period,
    /// (I + added inertia)·ωn²/(m·g), m (BG for a submerged body).
    #[serde(default)]
    pub metacentric_height_m: Option<f64>,
    /// (time s, value) at 10 Hz.
    pub series: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpeedPoint {
    pub fraction: f64,
    pub thrust_n: f64,
    /// Steady speed through the water, m/s.
    pub speed_mps: f64,
    pub steady: bool,
    /// Time to reach 90 % of the steady speed from rest, s.
    pub time_to_90_s: Option<f64>,
    /// Thrust / speed², N/(m/s)² (the effective resistance coefficient).
    pub resistance_coefficient: Option<f64>,
    /// Largest tilt from upright during the trial, degrees.
    #[serde(default)]
    pub max_tilt_deg: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Turning {
    /// "port" or "starboard".
    pub side: String,
    pub approach_speed_mps: f64,
    /// Thruster-steered: yaw moment command, N·m (0 when rudder-steered).
    pub yaw_moment_nm: f64,
    /// Rudder-steered: rudder angle, degrees.
    #[serde(default)]
    pub rudder_deg: Option<f64>,
    /// Largest heel or tilt during the turn, degrees.
    #[serde(default)]
    pub max_tilt_deg: f64,
    /// Distance along the original course when the heading has changed 90°.
    pub advance_m: Option<f64>,
    /// Distance across the original course at 90°.
    pub transfer_m: Option<f64>,
    /// Distance across the original course at 180°.
    pub tactical_diameter_m: Option<f64>,
    /// Diameter of the steady turning circle.
    pub steady_diameter_m: Option<f64>,
    pub time_to_90_s: Option<f64>,
    pub time_to_180_s: Option<f64>,
    pub steady_speed_mps: Option<f64>,
    pub steady_yaw_rate_dps: Option<f64>,
    pub drift_angle_deg: Option<f64>,
    /// (time s, along m, to port m) relative to the execute point, 2 Hz.
    pub track: Vec<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Zigzag {
    pub angle_deg: f64,
    pub approach_speed_mps: f64,
    /// Rudder-steered: rudder angle, degrees.
    #[serde(default)]
    pub rudder_deg: Option<f64>,
    #[serde(default)]
    pub max_tilt_deg: f64,
    /// Time from execute to the first command change, s.
    pub initial_turning_time_s: Option<f64>,
    /// Distance travelled by the first command change, vehicle lengths.
    pub initial_turning_lengths: Option<f64>,
    pub first_overshoot_deg: Option<f64>,
    pub second_overshoot_deg: Option<f64>,
    /// Time between the first and third command changes, s.
    pub period_s: Option<f64>,
    /// (time s, heading deg, command −1/+1), 5 Hz.
    pub series: Vec<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stopping {
    pub approach_speed_mps: f64,
    /// Astern thrust used; 0 means the vehicle coasted to a stop.
    pub astern_thrust_n: f64,
    pub time_to_stop_s: Option<f64>,
    #[serde(default)]
    pub max_tilt_deg: f64,
    /// Distance travelled along the track.
    pub track_reach_m: Option<f64>,
    /// Distance along the original course.
    pub head_reach_m: Option<f64>,
    pub lateral_deviation_m: f64,
    /// (time s, along m, to port m), 2 Hz.
    pub track: Vec<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaveResponse {
    /// Relative heading, degrees (180 head, 90 beam from starboard).
    pub heading_deg: f64,
    pub period_s: f64,
    pub height_m: f64,
    pub wavelength_m: f64,
    /// Motion amplitudes (first harmonic at the wave frequency).
    pub heave_m: f64,
    pub roll_deg: f64,
    pub pitch_deg: f64,
    /// Heave amplitude / wave amplitude.
    pub heave_rao: f64,
    /// Angle amplitude / wave-slope amplitude (k·a).
    pub roll_rao: f64,
    pub pitch_rao: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub name: String,
    pub value: Option<f64>,
    pub limit: f64,
    pub unit: String,
    /// "max" or "min".
    pub bound: String,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub format: String,
    pub engine: String,
    pub build_id: String,
    /// The exact vehicle tested.
    pub vehicle: VehicleDefinition,
    /// "surface" or "submerged".
    pub mode: String,
    pub length_m: f64,
    pub forward_thrust_n: f64,
    pub astern_thrust_n: f64,
    pub yaw_authority_nm: f64,
    /// "rudder" or "thrusters".
    #[serde(default)]
    pub steering: String,
    pub environment: Environment,
    pub setup: Setup,
    pub criteria: Criteria,
    pub decay: Vec<Decay>,
    pub speed: Vec<SpeedPoint>,
    pub turning: Vec<Turning>,
    pub zigzag: Vec<Zigzag>,
    pub stopping: Option<Stopping>,
    pub waves: Vec<WaveResponse>,
    pub checks: Vec<CheckResult>,
    /// Trials that could not run, with the reason.
    pub skipped: Vec<String>,
    /// Results that need attention (instability, untuned coefficients).
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Assumptions behind the numbers.
    pub notes: Vec<String>,
}

fn wrap(a: f64) -> f64 {
    (a + PI).rem_euclid(TAU) - PI
}

/// Roll, pitch and yaw (Z-Y-X order), radians.
pub fn attitude(q: Quat) -> (f64, f64, f64) {
    let roll = (2.0 * (q.w * q.x + q.y * q.z)).datan2(1.0 - 2.0 * (q.x * q.x + q.y * q.y));
    let pitch = (2.0 * (q.w * q.y - q.z * q.x)).clamp(-1.0, 1.0).dasin();
    let yaw = (2.0 * (q.w * q.z + q.x * q.y)).datan2(1.0 - 2.0 * (q.y * q.y + q.z * q.z));
    (roll, pitch, yaw)
}

/// Force and moment the vehicle can actually produce for a command (after
/// the thrust limit and thruster allocation).
fn produce(e: &Entity, force: Vec3, torque: Vec3) -> (Vec3, Vec3) {
    let limit = e.model.thrust_limit;
    let force = if force.length() > limit {
        force * (limit / force.length())
    } else {
        force
    };
    if e.model.thrusters.is_empty() {
        return (force, torque);
    }
    let values = actuators::allocate(&e.model.thrusters, force, torque);
    actuators::wrench(&e.model.thrusters, &values)
}

/// Damped period and damping ratio from a free-decay record (time, value),
/// relative to the value it settles to.
pub fn analyse_decay(record: &[(f64, f64)]) -> (f64, Option<f64>, Option<f64>) {
    let n = record.len();
    if n < 8 {
        return (f64::NAN, None, None);
    }
    let tail = &record[n * 3 / 4..];
    let equilibrium = tail.iter().map(|s| s.1).sum::<f64>() / tail.len() as f64;
    let x0 = record[0].1 - equilibrium;
    if x0 == 0.0 {
        return (equilibrium, None, None);
    }
    let floor = 0.03 * x0.abs();
    // Alternating extrema, starting with the release point.
    let mut extrema: Vec<(f64, f64)> = vec![(record[0].0, x0)];
    for w in record.windows(3) {
        let (a, b, c) = (w[0].1, w[1].1, w[2].1);
        let x = b - equilibrium;
        let peak = (b >= a && b > c) || (b <= a && b < c);
        if !peak || x.abs() < floor {
            continue;
        }
        let count = extrema.len();
        let last = extrema.last_mut().expect("seeded");
        if last.1.signum() == x.signum() {
            if x.abs() > last.1.abs() && count > 1 {
                *last = (w[1].0, x);
            }
        } else {
            extrema.push((w[1].0, x));
        }
        if extrema.len() >= 9 {
            break;
        }
    }
    if extrema.len() < 2 {
        return (equilibrium, None, Some(1.0));
    }
    let used = &extrema[..extrema.len().min(7)];
    let half_periods = (used.last().expect("non-empty").0 - used[0].0) / (used.len() - 1) as f64;
    let decrement = used
        .windows(2)
        .map(|p| (p[0].1.abs() / p[1].1.abs()).dln())
        .sum::<f64>()
        / (used.len() - 1) as f64
        * 2.0;
    let zeta = decrement / (4.0 * PI * PI + decrement * decrement).sqrt();
    (equilibrium, Some(2.0 * half_periods), Some(zeta.max(0.0)))
}

/// One simulated test: a single-vehicle world plus heading bookkeeping.
pub(crate) struct Rig {
    pub(crate) world: World,
    current: Vec3,
    yaw: f64,
    /// Unwrapped heading, radians.
    pub(crate) heading: f64,
    /// Autopilot proportional gain, derivative gain and thruster moment limit.
    gains: (f64, f64, f64),
    /// Rudders (yaw-channel fins) and water density for the autopilot.
    rudders: Vec<Fin>,
    density: f64,
    /// Largest tilt from upright so far, degrees.
    max_tilt: f64,
}
impl Rig {
    pub(crate) fn vehicle(&self) -> &Vehicle {
        &self.world.vehicles()[0]
    }
    pub(crate) fn time(&self) -> f64 {
        self.world.time()
    }
    fn position(&self) -> Vec3 {
        self.vehicle().position
    }
    /// One step with a surge force (N), a yaw moment (N·m) and a rudder
    /// command (−1..1).
    pub(crate) fn step(&mut self, surge: f64, (yaw_moment, rudder): (f64, f64)) {
        if let Some(v) = self.world.vehicle_mut(0) {
            v.force = Vec3::new(surge, 0.0, 0.0);
            v.torque = Vec3::new(0.0, 0.0, yaw_moment);
            v.fin_command = Vec3::new(0.0, 0.0, rudder);
        }
        self.world.step();
        let v = self.vehicle();
        let (_, _, yaw) = attitude(v.orientation);
        let up = v.orientation.rotate(Vec3::new(0.0, 0.0, 1.0)).z;
        self.max_tilt = self.max_tilt.max(up.clamp(-1.0, 1.0).dacos().to_degrees());
        self.heading += wrap(yaw - self.yaw);
        self.yaw = yaw;
    }
    /// Steering (yaw moment, rudder) that holds `target` (unwrapped
    /// radians): rudders first, thrusters for what the rudders cannot give
    /// (all of it at low speed).
    pub(crate) fn hold(&self, target: f64) -> (f64, f64) {
        let (kp, kd, limit) = self.gains;
        let wanted = kp * wrap(target - self.heading) - kd * self.vehicle().angular_velocity.z;
        let mut rudder = 0.0;
        let mut rest = wanted;
        if !self.rudders.is_empty() {
            let authority = fins::yaw_authority(&self.rudders, self.speed().max(0.2), self.density);
            if authority > 0.0 {
                rudder = (wanted / authority).clamp(-1.0, 1.0);
                rest = wanted - rudder * authority;
            }
        }
        (rest.clamp(-limit, limit), rudder)
    }
    /// Velocity through the water in the body frame.
    pub(crate) fn body_velocity(&self) -> Vec3 {
        let v = self.vehicle();
        v.orientation.conjugate().rotate(v.velocity - self.current)
    }
    /// Horizontal speed through the water.
    pub(crate) fn speed(&self) -> f64 {
        let r = self.vehicle().velocity - self.current;
        (r.x * r.x + r.y * r.y).sqrt()
    }
}

/// The prepared vehicle under test and its capabilities.
pub(crate) struct Bench {
    pub(crate) entity: Entity,
    env: Environment,
    setup: Setup,
    pub(crate) surface: bool,
    pub(crate) start_z: f64,
    pub(crate) length: f64,
    pub(crate) forward: f64,
    /// Largest astern thrust (N).
    pub(crate) astern: f64,
    /// Largest yaw moment from the thrusters (N·m).
    pub(crate) yaw: f64,
    yaw_inertia: f64,
    /// Largest rudder deflection, degrees; 0 when steered by thrusters.
    rudder: f64,
}

#[derive(Clone, Copy)]
enum Axis {
    Heave,
    Roll,
    Pitch,
}

#[derive(Clone, Copy)]
enum Task {
    Decay(Axis),
    Speed(usize),
    Turn(f64),
    Zigzag(usize),
    Stop,
    Wave(usize, usize),
}

enum Piece {
    Decay(Decay),
    Speed(SpeedPoint),
    Turn(Turning),
    Zigzag(Zigzag),
    Stop(Stopping),
    Wave(WaveResponse),
}

impl Bench {
    pub(crate) fn new(vehicle: &Entity, env: &Environment, setup: &Setup) -> Result<Self, String> {
        vehicle.validate()?;
        env.validate()?;
        setup.validate()?;
        let mut e = vehicle.clone();
        e.id = e.id.max(1);
        e.heading = 0.0;
        e.force = Vec3::ZERO;
        e.position = Vec3::ZERO;
        e.model.hold_heading = None;
        e.model.hold_depth = None;
        let h = e.model.half;
        let probe = Ocean {
            density: env.density,
            ..Ocean::default()
        };
        let density_at = |z: f64| {
            if e.model.profile_density {
                probe.water(Vec3::new(0.0, 0.0, z)).density
            } else {
                env.density
            }
        };
        let floats = e.model.partial_buoyancy && e.mass < 0.98 * density_at(0.0) * e.volume;
        let surface = e.kind != 0 && floats;
        if e.kind == 1 && !surface {
            return Err(format!(
                "{} does not float: it needs displaced volume above {:.3} m³ (mass / density) and surface crossing enabled.",
                e.name,
                e.mass / density_at(0.0)
            ));
        }
        let start_z = if surface {
            // Upright box: submerged fraction of the hull depth = m / (ρV).
            let mut z = 0.0;
            for _ in 0..3 {
                z = h.z - 2.0 * h.z * e.mass / (density_at(z) * e.volume);
            }
            z
        } else {
            if setup.operating_depth + h.length() + 1.0 > env.water_depth {
                return Err(format!(
                    "Operating depth {:.1} m does not fit in {:.1} m of water.",
                    setup.operating_depth, env.water_depth
                ));
            }
            -setup.operating_depth
        };
        let limit = e.model.thrust_limit;
        let forward = produce(&e, Vec3::new(limit, 0.0, 0.0), Vec3::ZERO)
            .0
            .x
            .max(0.0);
        let astern = (-produce(&e, Vec3::new(-limit, 0.0, 0.0), Vec3::ZERO).0.x).max(0.0);
        let yaw = if e.model.thrusters.is_empty() {
            limit * h.x
        } else {
            let reach: f64 = e
                .model
                .thrusters
                .iter()
                .map(|t| t.limit * t.position.length())
                .sum();
            produce(&e, Vec3::ZERO, Vec3::new(0.0, 0.0, 10.0 * reach + 1.0))
                .1
                .z
                .abs()
        };
        if !surface {
            if forward > 0.0 {
                e.model.hold_depth = Some(setup.operating_depth);
            } else {
                // A passive submerged body is ballasted neutral for the test.
                e.volume = e.mass / density_at(start_z);
            }
        }
        let rudder = e
            .model
            .fins
            .iter()
            .filter(|f| f.channel == fins::YAW)
            .map(|f| f.max_deflection_deg)
            .fold(0.0, f64::max);
        Ok(Self {
            rudder,
            yaw_inertia: e.effective_inertia().z,
            length: 2.0 * h.x,
            entity: e,
            env: env.clone(),
            setup: setup.clone(),
            surface,
            start_z,
            forward,
            astern,
            yaw,
        })
    }

    pub(crate) fn rig(
        &self,
        entity: &Entity,
        waves: SeaState,
        current: bool,
        position: Vec3,
        orientation: Quat,
    ) -> Result<Rig, String> {
        let current = if current {
            self.env.current()
        } else {
            Vec3::ZERO
        };
        let ocean = Ocean {
            current: Box::new(UniformCurrent(current)),
            bottom: Box::new(FlatBottom(-self.env.water_depth)),
            density: self.env.density,
            waves,
            ..Ocean::default()
        };
        let mut world = World::new(ocean, DT, self.env.seed).map_err(str::to_string)?;
        let mut v = entity.spawn();
        v.position = position;
        v.orientation = orientation;
        v.velocity = current;
        world.add_vehicle(v);
        Ok(self.rig_in(world, current))
    }

    /// A rig around an existing world whose vehicle 0 is the vehicle under
    /// test (for example a full scene with its seabed and sea).
    pub(crate) fn rig_in(&self, world: World, current: Vec3) -> Rig {
        let (w, z) = AUTOPILOT;
        let (_, _, yaw) = attitude(world.vehicles()[0].orientation);
        Rig {
            world,
            current,
            yaw,
            heading: yaw,
            gains: (
                self.yaw_inertia * w * w,
                2.0 * z * w * self.yaw_inertia,
                self.yaw,
            ),
            rudders: self
                .entity
                .model
                .fins
                .iter()
                .filter(|f| f.channel == fins::YAW)
                .cloned()
                .collect(),
            density: self.env.density,
            max_tilt: 0.0,
        }
    }

    /// Steering for a fraction of full command, signed (+ to port):
    /// (yaw moment, rudder command).
    fn steer(&self, fraction: f64) -> (f64, f64) {
        if self.rudder > 0.0 {
            (0.0, fraction.clamp(-1.0, 1.0))
        } else {
            (fraction * self.yaw, 0.0)
        }
    }

    fn manoeuvre_rig(&self) -> Result<Rig, String> {
        let sea = if self.env.wave_height > 0.0 {
            SeaState::spectrum(
                self.env.wave_height,
                self.env.wave_period,
                self.env.wave_direction.to_radians(),
                self.env.wave_spread.to_radians(),
                self.env.water_depth,
                self.env.seed,
            )
            .map_err(str::to_string)?
        } else {
            SeaState::default()
        };
        self.rig(
            &self.entity,
            sea,
            true,
            Vec3::new(0.0, 0.0, self.start_z),
            Quat::IDENTITY,
        )
    }

    /// Accelerates on a steady course until the speed settles (or 300 s).
    /// Returns the steady speed, whether it settled and a 2 Hz speed record.
    pub(crate) fn approach(&self, rig: &mut Rig, thrust: f64) -> (f64, bool, Vec<[f64; 2]>) {
        let mut seconds: Vec<f64> = vec![];
        let mut record = vec![];
        let mut steady = false;
        let start = rig.time();
        let mut tick = 0u64;
        while rig.time() - start < 300.0 {
            let steer = rig.hold(0.0);
            rig.step(thrust, steer);
            tick += 1;
            if tick.is_multiple_of(30) {
                record.push([rig.time() - start, rig.speed()]);
            }
            if tick.is_multiple_of(60) {
                seconds.push(rig.speed());
                let n = seconds.len();
                if n >= 20 {
                    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
                    let (now, before) = (mean(&seconds[n - 5..]), mean(&seconds[n - 10..n - 5]));
                    if (now - before).abs() <= 0.002 * now.max(0.05) {
                        steady = true;
                        break;
                    }
                }
            }
        }
        let n = seconds.len();
        let speed = seconds[n.saturating_sub(10)..].iter().sum::<f64>() / n.clamp(1, 10) as f64;
        (speed, steady, record)
    }

    fn decay(&self, axis: Axis) -> Result<Decay, String> {
        let h = self.entity.model.half;
        let mut entity = self.entity.clone();
        entity.model.hold_depth = None;
        let probe = Ocean {
            density: self.env.density,
            ..Ocean::default()
        };
        let density = if entity.model.profile_density {
            probe.water(Vec3::new(0.0, 0.0, self.start_z)).density
        } else {
            self.env.density
        };
        if !self.surface {
            // Ballasted neutral so the body stays at depth during the test.
            entity.volume = entity.mass / density;
        }
        let draft = h.z - self.start_z;
        let freeboard = h.z + self.start_z;
        let margin = draft.min(freeboard).max(0.0);
        let g = probe.gravity;
        let mass_g = entity.mass * g;
        // Rigid-body plus added moments of inertia.
        let inertia = entity.effective_inertia() + entity.model.added_inertia;
        let cb = entity.model.center_buoyancy.z;
        let angular = |i: f64| (cb > 0.0).then(|| TAU * (i / (mass_g * cb)).sqrt());
        let (offset, rotation, initial, unit, predicted) = match axis {
            Axis::Heave => {
                let dz = 0.3 * margin;
                // Effective waterplane of the column model: V / hull depth.
                let area = entity.volume / (2.0 * h.z);
                let predicted =
                    TAU * ((entity.mass + entity.model.added_mass.z) / (density * g * area)).sqrt();
                (-dz, Vec3::ZERO, -dz, "m", Some(predicted))
            }
            Axis::Roll => {
                let angle = if self.surface {
                    5.0_f64.min(0.5 * (margin / h.y).datan2(1.0).to_degrees())
                } else {
                    5.0
                };
                let predicted = (!self.surface).then(|| angular(inertia.x)).flatten();
                (
                    0.0,
                    Vec3::new(angle.to_radians(), 0.0, 0.0),
                    angle,
                    "deg",
                    predicted,
                )
            }
            Axis::Pitch => {
                let angle = if self.surface {
                    2.0_f64.min(0.5 * (margin / h.x).datan2(1.0).to_degrees())
                } else {
                    5.0
                };
                let predicted = (!self.surface).then(|| angular(inertia.y)).flatten();
                (
                    0.0,
                    Vec3::new(0.0, angle.to_radians(), 0.0),
                    angle,
                    "deg",
                    predicted,
                )
            }
        };
        let mut rig = self.rig(
            &entity,
            SeaState::default(),
            false,
            Vec3::new(0.0, 0.0, self.start_z + offset),
            Quat::from_rotation_vector(rotation),
        )?;
        let seconds = predicted.map_or(60.0, |t| (8.0 * t).clamp(30.0, 180.0));
        let value = |rig: &Rig| {
            let v = rig.vehicle();
            let (roll, pitch, _) = attitude(v.orientation);
            match axis {
                Axis::Heave => v.position.z,
                Axis::Roll => roll.to_degrees(),
                Axis::Pitch => pitch.to_degrees(),
            }
        };
        let mut record = vec![(0.0, value(&rig))];
        while rig.time() < seconds {
            rig.step(0.0, (0.0, 0.0));
            record.push((rig.time(), value(&rig)));
            if !record.last().expect("pushed").1.is_finite() {
                return Err("The vehicle became unstable during free decay.".into());
            }
        }
        let (equilibrium, period, zeta) = analyse_decay(&record);
        let initial = match axis {
            Axis::Heave => initial,
            _ => initial - equilibrium,
        };
        let metacentric_height_m = match (axis, period, zeta) {
            (Axis::Heave, ..) => None,
            (_, Some(t), Some(z)) if z < 1.0 => {
                let wn = TAU / t / (1.0 - z * z).sqrt();
                let i = match axis {
                    Axis::Roll => inertia.x,
                    _ => inertia.y,
                };
                Some(i * wn * wn / mass_g)
            }
            _ => None,
        };
        Ok(Decay {
            axis: match axis {
                Axis::Heave => "heave",
                Axis::Roll => "roll",
                Axis::Pitch => "pitch",
            }
            .into(),
            unit: unit.into(),
            initial,
            equilibrium,
            period_s: period,
            damping_ratio: zeta,
            predicted_period_s: predicted,
            metacentric_height_m,
            series: record.iter().step_by(6).map(|(t, v)| [*t, *v]).collect(),
        })
    }

    fn speed(&self, level: usize) -> Result<SpeedPoint, String> {
        let fraction = self.setup.speed_levels[level];
        let thrust = fraction * self.forward;
        let mut rig = self.manoeuvre_rig()?;
        let (speed, steady, record) = self.approach(&mut rig, thrust);
        let time_to_90_s = record.iter().find(|s| s[1] >= 0.9 * speed).map(|s| s[0]);
        Ok(SpeedPoint {
            fraction,
            thrust_n: thrust,
            speed_mps: speed,
            steady,
            time_to_90_s,
            resistance_coefficient: (speed > 1e-3).then(|| thrust / (speed * speed)),
            max_tilt_deg: rig.max_tilt,
        })
    }

    fn turn(&self, side: f64) -> Result<Turning, String> {
        let thrust = self.setup.approach_thrust * self.forward;
        let mut rig = self.manoeuvre_rig()?;
        let (approach_speed, _, _) = self.approach(&mut rig, thrust);
        let (p0, psi0, t0) = (rig.position(), rig.heading, rig.time());
        let (s0, c0) = psi0.dsin_cos();
        let frame = |p: Vec3| {
            let d = p - p0;
            (c0 * d.x + s0 * d.y, -s0 * d.x + c0 * d.y)
        };
        rig.max_tilt = 0.0;
        let fraction = if self.rudder > 0.0 {
            (self.setup.rudder_deg / self.rudder).min(1.0)
        } else {
            self.setup.yaw_command
        };
        let steer = self.steer(side * fraction);
        let mut out = Turning {
            side: if side > 0.0 { "port" } else { "starboard" }.into(),
            approach_speed_mps: approach_speed,
            yaw_moment_nm: steer.0,
            rudder_deg: (self.rudder > 0.0).then_some(fraction * self.rudder),
            max_tilt_deg: 0.0,
            advance_m: None,
            transfer_m: None,
            tactical_diameter_m: None,
            steady_diameter_m: None,
            time_to_90_s: None,
            time_to_180_s: None,
            steady_speed_mps: None,
            steady_yaw_rate_dps: None,
            drift_angle_deg: None,
            track: vec![[0.0, 0.0, 0.0]],
        };
        let mut steady: Vec<(f64, f64, f64, f64, f64)> = vec![];
        let mut tick = 0u64;
        let mut turned = 0.0;
        while rig.time() - t0 < 900.0 && turned < 540.0 {
            rig.step(thrust, steer);
            tick += 1;
            turned = (rig.heading - psi0).to_degrees() * side;
            let (along, across) = frame(rig.position());
            let t = rig.time() - t0;
            if tick.is_multiple_of(30) {
                out.track.push([t, along, across]);
            }
            if out.advance_m.is_none() && turned >= 90.0 {
                out.advance_m = Some(along);
                out.transfer_m = Some(across * side);
                out.time_to_90_s = Some(t);
            }
            if out.tactical_diameter_m.is_none() && turned >= 180.0 {
                out.tactical_diameter_m = Some(across * side);
                out.time_to_180_s = Some(t);
            }
            if turned >= 180.0 {
                let b = rig.body_velocity();
                steady.push((
                    rig.position().x,
                    rig.position().y,
                    rig.speed(),
                    rig.vehicle().angular_velocity.z.to_degrees().abs(),
                    b.y.datan2(b.x).to_degrees().abs(),
                ));
            }
            if !rig.position().is_finite() {
                return Err("The vehicle became unstable while turning.".into());
            }
        }
        out.max_tilt_deg = rig.max_tilt;
        if !steady.is_empty() {
            let n = steady.len() as f64;
            let mean =
                |f: fn(&(f64, f64, f64, f64, f64)) -> f64| steady.iter().map(f).sum::<f64>() / n;
            out.steady_speed_mps = Some(mean(|s| s.2));
            out.steady_yaw_rate_dps = Some(mean(|s| s.3));
            out.drift_angle_deg = Some(mean(|s| s.4));
            if turned >= 540.0 {
                let (cx, cy) = (mean(|s| s.0), mean(|s| s.1));
                let radius = steady
                    .iter()
                    .map(|s| ((s.0 - cx).powi(2) + (s.1 - cy).powi(2)).sqrt())
                    .sum::<f64>()
                    / n;
                out.steady_diameter_m = Some(2.0 * radius);
            }
        }
        Ok(out)
    }

    fn zigzag(&self, index: usize) -> Result<Zigzag, String> {
        let angle = self.setup.zigzag_angles[index];
        let thrust = self.setup.approach_thrust * self.forward;
        let mut rig = self.manoeuvre_rig()?;
        let (approach_speed, _, _) = self.approach(&mut rig, thrust);
        let (psi0, t0) = (rig.heading, rig.time());
        rig.max_tilt = 0.0;
        // Rudder-steered: rudder angle = switching angle (e.g. 10°/10°).
        let fraction = if self.rudder > 0.0 {
            (angle / self.rudder).min(1.0)
        } else {
            self.setup.yaw_command
        };
        // Starboard first, by convention.
        let mut sign = -1.0;
        let mut switches: Vec<f64> = vec![];
        let mut distance = 0.0;
        let mut last = rig.position();
        let mut first_lengths = None;
        let mut extreme = [0.0_f64; 2];
        let mut series = vec![[0.0, 0.0, sign]];
        let mut tick = 0u64;
        while rig.time() - t0 < 600.0 && switches.len() < 5 {
            rig.step(thrust, self.steer(sign * fraction));
            tick += 1;
            let p = rig.position();
            distance += ((p.x - last.x).powi(2) + (p.y - last.y).powi(2)).sqrt();
            last = p;
            let heading = (rig.heading - psi0).to_degrees();
            let t = rig.time() - t0;
            match switches.len() {
                1 => extreme[0] = extreme[0].min(heading),
                2 => extreme[1] = extreme[1].max(heading),
                _ => {}
            }
            if (sign < 0.0 && heading <= -angle) || (sign > 0.0 && heading >= angle) {
                if switches.is_empty() {
                    first_lengths = Some(distance / self.length);
                }
                switches.push(t);
                sign = -sign;
            }
            if tick.is_multiple_of(12) {
                series.push([t, heading, sign]);
            }
            if !p.is_finite() {
                return Err("The vehicle became unstable during the zig-zag.".into());
            }
        }
        Ok(Zigzag {
            angle_deg: angle,
            approach_speed_mps: approach_speed,
            rudder_deg: (self.rudder > 0.0).then_some(fraction * self.rudder),
            max_tilt_deg: rig.max_tilt,
            initial_turning_time_s: switches.first().copied(),
            initial_turning_lengths: first_lengths,
            first_overshoot_deg: (switches.len() >= 2).then(|| -extreme[0] - angle),
            second_overshoot_deg: (switches.len() >= 3).then(|| extreme[1] - angle),
            period_s: (switches.len() >= 3).then(|| switches[2] - switches[0]),
            series,
        })
    }

    fn stop(&self) -> Result<Stopping, String> {
        let thrust = self.setup.approach_thrust * self.forward;
        let mut rig = self.manoeuvre_rig()?;
        let (approach_speed, _, _) = self.approach(&mut rig, thrust);
        let (p0, psi0, t0) = (rig.position(), rig.heading, rig.time());
        let (s0, c0) = psi0.dsin_cos();
        let threshold = (0.02 * approach_speed).max(0.01);
        rig.max_tilt = 0.0;
        let mut out = Stopping {
            approach_speed_mps: approach_speed,
            astern_thrust_n: self.astern,
            time_to_stop_s: None,
            max_tilt_deg: 0.0,
            track_reach_m: None,
            head_reach_m: None,
            lateral_deviation_m: 0.0,
            track: vec![[0.0, 0.0, 0.0]],
        };
        let mut distance = 0.0;
        let mut last = p0;
        let mut tick = 0u64;
        while rig.time() - t0 < 900.0 {
            let steer = rig.hold(psi0);
            rig.step(-self.astern, steer);
            out.max_tilt_deg = rig.max_tilt;
            tick += 1;
            let p = rig.position();
            distance += ((p.x - last.x).powi(2) + (p.y - last.y).powi(2)).sqrt();
            last = p;
            let d = p - p0;
            let (along, across) = (c0 * d.x + s0 * d.y, -s0 * d.x + c0 * d.y);
            out.lateral_deviation_m = out.lateral_deviation_m.max(across.abs());
            let t = rig.time() - t0;
            if tick.is_multiple_of(30) {
                out.track.push([t, along, across]);
            }
            if rig.body_velocity().x <= threshold {
                out.time_to_stop_s = Some(t);
                out.track_reach_m = Some(distance);
                out.head_reach_m = Some(along);
                out.track.push([t, along, across]);
                break;
            }
            if !p.is_finite() {
                return Err("The vehicle became unstable while stopping.".into());
            }
        }
        Ok(out)
    }

    fn wave(&self, heading: usize, period: usize) -> Result<WaveResponse, String> {
        let relative = self.setup.wave_headings[heading];
        let period = self.setup.wave_periods[period];
        let height = self.setup.wave_height;
        let sea = SeaState::regular(height, period, relative.to_radians(), self.env.water_depth)
            .map_err(str::to_string)?;
        let omega = TAU / period;
        let k = wave_number(omega, self.env.water_depth);
        let mut rig = self.rig(
            &self.entity,
            sea,
            false,
            Vec3::new(0.0, 0.0, self.start_z),
            Quat::IDENTITY,
        )?;
        let ramp = (8.0 * period).max(40.0);
        let end = ramp + 6.0 * period;
        // Σv, Σv·cos, Σv·sin for heave, roll, pitch; Σcos, Σsin; count.
        let mut sums = [[0.0; 3]; 3];
        let (mut sc, mut ss, mut n) = (0.0, 0.0, 0.0);
        while rig.time() < end {
            let steer = rig.hold(0.0);
            rig.step(0.0, steer);
            let t = rig.time();
            if t <= ramp {
                continue;
            }
            let v = rig.vehicle();
            if !v.position.is_finite() {
                return Err("The vehicle became unstable in waves.".into());
            }
            let (roll, pitch, _) = attitude(v.orientation);
            let (s, c) = (omega * t).dsin_cos();
            for (acc, value) in sums.iter_mut().zip([v.position.z, roll, pitch]) {
                acc[0] += value;
                acc[1] += value * c;
                acc[2] += value * s;
            }
            sc += c;
            ss += s;
            n += 1.0;
        }
        let amplitude = |acc: [f64; 3]| {
            let mean = acc[0] / n;
            let (a, b) = (acc[1] - mean * sc, acc[2] - mean * ss);
            2.0 / n * (a * a + b * b).sqrt()
        };
        let (heave, roll, pitch) = (amplitude(sums[0]), amplitude(sums[1]), amplitude(sums[2]));
        let a = height * 0.5;
        Ok(WaveResponse {
            heading_deg: relative,
            period_s: period,
            height_m: height,
            wavelength_m: TAU / k,
            heave_m: heave,
            roll_deg: roll.to_degrees(),
            pitch_deg: pitch.to_degrees(),
            heave_rao: heave / a,
            roll_rao: roll / (k * a),
            pitch_rao: pitch / (k * a),
        })
    }

    fn run(&self, task: Task) -> Result<Piece, String> {
        Ok(match task {
            Task::Decay(axis) => Piece::Decay(self.decay(axis)?),
            Task::Speed(i) => Piece::Speed(self.speed(i)?),
            Task::Turn(side) => Piece::Turn(self.turn(side)?),
            Task::Zigzag(i) => Piece::Zigzag(self.zigzag(i)?),
            Task::Stop => Piece::Stop(self.stop()?),
            Task::Wave(h, p) => Piece::Wave(self.wave(h, p)?),
        })
    }

    fn tasks(&self, skipped: &mut Vec<String>) -> Vec<Task> {
        let s = &self.setup;
        let mut tasks = vec![];
        if s.free_decay {
            if self.surface {
                tasks.push(Task::Decay(Axis::Heave));
            }
            if self.surface || self.entity.model.center_buoyancy.z > 0.0 {
                tasks.push(Task::Decay(Axis::Roll));
                tasks.push(Task::Decay(Axis::Pitch));
            } else {
                skipped.push(
                    "Free decay in roll and pitch: the centre of buoyancy is not above the centre of mass, so there is no restoring moment.".into(),
                );
            }
        }
        let propelled = self.forward > 0.0;
        let manoeuvres = s.speed || s.turning || s.zigzag || s.stopping;
        if manoeuvres && !propelled {
            skipped.push(
                "Speed, turning, zig-zag and stopping trials: the vehicle has no forward thrust."
                    .into(),
            );
        }
        if propelled {
            if s.speed {
                tasks.extend((0..s.speed_levels.len()).map(Task::Speed));
            }
            if (s.turning || s.zigzag) && self.yaw <= 0.0 {
                skipped.push(
                    "Turning and zig-zag trials: the thrusters cannot produce a yaw moment.".into(),
                );
            } else if (s.turning || s.zigzag) && !self.surface && self.rudder <= 0.0 {
                // A thruster-steered underwater vehicle turns on the spot:
                // turning circles and zig-zag overshoots (ship tests for
                // rudder-steered hulls) would be meaningless numbers.
                skipped.push(
                    "Turning circle and zig-zag: these ship manoeuvring tests (IMO MSC.137(76)) are for rudder-steered hulls. This underwater vehicle steers with thrusters and turns on the spot, so they do not apply; speed, stopping, decay and wave trials still run.".into(),
                );
            } else {
                if s.turning {
                    tasks.push(Task::Turn(1.0));
                    tasks.push(Task::Turn(-1.0));
                }
                if s.zigzag {
                    tasks.extend((0..s.zigzag_angles.len()).map(Task::Zigzag));
                }
            }
            if s.stopping {
                tasks.push(Task::Stop);
            }
        }
        if s.waves {
            for h in 0..s.wave_headings.len() {
                for p in 0..s.wave_periods.len() {
                    tasks.push(Task::Wave(h, p));
                }
            }
        }
        tasks
    }

    fn notes(&self) -> Vec<String> {
        let mut notes = vec![
            format!(
                "Tested {} in {:.0} m of water (density {:.1} kg/m³).",
                if self.surface {
                    "at the surface".to_string()
                } else {
                    format!("submerged at {:.1} m", self.setup.operating_depth)
                },
                self.env.water_depth,
                self.env.density
            ),
            if self.rudder > 0.0 {
                format!(
                    "Steered by rudder (largest deflection {:.0}°): turning circle at {:.0}°, zig-zag rudder angle = switching angle.",
                    self.rudder,
                    self.setup.rudder_deg.min(self.rudder)
                )
            } else {
                format!(
                    "Steered by a yaw moment through the thrusters (no rudders defined): {:.0} % of the {:.0} N·m yaw authority.",
                    self.setup.yaw_command * 100.0,
                    self.yaw
                )
            },
            format!(
                "Heading is held during approaches by the trial autopilot (natural frequency {} rad/s, damping {}), not by the vehicle's own heading controller.",
                AUTOPILOT.0, AUTOPILOT.1
            ),
            "Free decay and wave response use still water; the current and irregular sea apply to speed, turning, zig-zag and stopping.".into(),
            "Wave response is the first harmonic at the wave frequency after at least 8 periods; roll and pitch are normalised by the wave slope k·a.".into(),
        ];
        if !self.surface {
            notes.push(
                "Submerged vehicles are ballasted to neutral buoyancy for free decay; during manoeuvres and wave tests the depth is held by the vehicle's depth controller.".into(),
            );
        }
        if self.entity.model.thrusters.is_empty() {
            notes.push(format!(
                "No thrusters defined: forces are applied directly, limited to {:.0} N; yaw authority is taken as that force at half the hull length ({:.0} N·m).",
                self.entity.model.thrust_limit, self.yaw
            ));
        }
        if self.env.wave_height > 0.0 || self.env.current().length() > 0.0 {
            notes.push(format!(
                "Manoeuvres ran in a current of {:.2} m/s and an irregular sea of Hs {:.1} m, Tp {:.1} s; turning-circle diameters drift with the current.",
                self.env.current().length(),
                self.env.wave_height,
                self.env.wave_period
            ));
        }
        notes
    }
}

fn judge(report: &Report) -> Vec<CheckResult> {
    let c = &report.criteria;
    let l = report.length_m;
    let mut checks = vec![];
    // A check applies only when its trial ran (`ran`); a trial that ran but
    // produced no value fails.
    let mut add =
        |name: &str, ran: bool, value: Option<f64>, limit: Option<f64>, unit: &str, max: bool| {
            if let (true, Some(limit)) = (ran, limit) {
                let passed = value.is_some_and(|v| if max { v <= limit } else { v >= limit });
                checks.push(CheckResult {
                    name: name.into(),
                    value,
                    limit,
                    unit: unit.into(),
                    bound: if max { "max" } else { "min" }.into(),
                    passed,
                });
            }
        };
    let worst = |values: Vec<Option<f64>>| {
        if values.is_empty() || values.iter().any(Option::is_none) {
            None
        } else {
            values.into_iter().flatten().reduce(f64::max)
        }
    };
    add(
        "Speed at full thrust",
        !report.speed.is_empty(),
        report
            .speed
            .iter()
            .max_by(|a, b| a.fraction.total_cmp(&b.fraction))
            .map(|s| s.speed_mps),
        c.min_speed_mps,
        "m/s",
        false,
    );
    add(
        "Advance",
        !report.turning.is_empty(),
        worst(
            report
                .turning
                .iter()
                .map(|t| t.advance_m.map(|v| v / l))
                .collect(),
        ),
        c.max_advance_lengths,
        "L",
        true,
    );
    add(
        "Tactical diameter",
        !report.turning.is_empty(),
        worst(
            report
                .turning
                .iter()
                .map(|t| t.tactical_diameter_m.map(|v| v / l))
                .collect(),
        ),
        c.max_tactical_diameter_lengths,
        "L",
        true,
    );
    let zig = |angle: f64| {
        report
            .zigzag
            .iter()
            .find(|z| (z.angle_deg - angle).abs() < 0.5)
    };
    add(
        "Initial turning (10° zig-zag)",
        zig(10.0).is_some(),
        zig(10.0).and_then(|z| z.initial_turning_lengths),
        c.max_initial_turning_lengths,
        "L",
        true,
    );
    add(
        "10°/10° first overshoot",
        zig(10.0).is_some(),
        zig(10.0).and_then(|z| z.first_overshoot_deg),
        c.max_first_overshoot_deg,
        "°",
        true,
    );
    add(
        "10°/10° second overshoot",
        zig(10.0).is_some(),
        zig(10.0).and_then(|z| z.second_overshoot_deg),
        c.max_second_overshoot_deg,
        "°",
        true,
    );
    add(
        "20°/20° first overshoot",
        zig(20.0).is_some(),
        zig(20.0).and_then(|z| z.first_overshoot_deg),
        c.max_first_overshoot_20_deg,
        "°",
        true,
    );
    add(
        "Stopping track reach",
        report.stopping.is_some(),
        report
            .stopping
            .as_ref()
            .and_then(|s| s.track_reach_m.map(|v| v / l)),
        c.max_stopping_track_lengths,
        "L",
        true,
    );
    add(
        "Roll damping ratio",
        report.decay.iter().any(|d| d.axis == "roll"),
        report
            .decay
            .iter()
            .find(|d| d.axis == "roll")
            .and_then(|d| d.damping_ratio),
        c.min_roll_damping,
        "",
        false,
    );
    add(
        "Roll response in waves",
        !report.waves.is_empty(),
        report.waves.iter().map(|w| w.roll_rao).reduce(f64::max),
        c.max_roll_rao,
        "°/°",
        true,
    );
    checks
}

impl Report {
    /// Approach speed of the manoeuvres (the trial speed), if any ran.
    pub fn test_speed(&self) -> Option<f64> {
        self.turning
            .first()
            .map(|t| t.approach_speed_mps)
            .or_else(|| self.zigzag.first().map(|z| z.approach_speed_mps))
            .or_else(|| self.stopping.as_ref().map(|s| s.approach_speed_mps))
            .or_else(|| self.speed.last().map(|s| s.speed_mps))
    }
    /// IMO MSC.137(76) reference limits for this vehicle at its trial speed.
    pub fn imo_criteria(&self) -> Criteria {
        Criteria::imo(self.length_m, self.test_speed().unwrap_or(1.0))
    }
    /// Replaces the acceptance criteria and judges the results again (the
    /// trials themselves are not re-run).
    pub fn judge_with(&mut self, criteria: Criteria) {
        self.criteria = criteria;
        self.checks = judge(self);
    }
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }
    pub fn failed(&self) -> usize {
        self.checks.iter().filter(|c| !c.passed).count()
    }

    /// Plain-text summary for logs, CI and the summary file.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "Trials: {} ({}, L {:.2} m, forward thrust {:.0} N, astern {:.0} N, yaw authority {:.0} N·m)\n",
            self.vehicle.name,
            self.mode,
            self.length_m,
            self.forward_thrust_n,
            self.astern_thrust_n,
            self.yaw_authority_nm
        );
        let opt = |v: Option<f64>, digits: usize| {
            v.map_or_else(|| "—".to_string(), |v| format!("{v:.digits$}"))
        };
        for d in &self.decay {
            s += &format!(
                "Free decay {:<5}  period {} s  damping {}  (linear estimate {} s){}\n",
                d.axis,
                opt(d.period_s, 2),
                opt(d.damping_ratio, 3),
                opt(d.predicted_period_s, 2),
                d.metacentric_height_m
                    .map(|g| format!("  GM {g:.3} m"))
                    .unwrap_or_default()
            );
        }
        for p in &self.speed {
            s += &format!(
                "Speed  {:>3.0}% thrust {:>8.1} N  →  {:.2} m/s ({:.1} kn){}\n",
                p.fraction * 100.0,
                p.thrust_n,
                p.speed_mps,
                p.speed_mps * 1.943_844,
                if p.steady { "" } else { "  (not settled)" }
            );
        }
        for t in &self.turning {
            s += &format!(
                "Turn {:<9} advance {} m  transfer {} m  tactical {} m  steady {} m  yaw rate {} °/s\n",
                t.side,
                opt(t.advance_m, 1),
                opt(t.transfer_m, 1),
                opt(t.tactical_diameter_m, 1),
                opt(t.steady_diameter_m, 1),
                opt(t.steady_yaw_rate_dps, 2)
            );
        }
        for z in &self.zigzag {
            s += &format!(
                "Zig-zag {:.0}°/{:.0}°  overshoots {}° / {}°  initial turning {} s  period {} s\n",
                z.angle_deg,
                z.angle_deg,
                opt(z.first_overshoot_deg, 1),
                opt(z.second_overshoot_deg, 1),
                opt(z.initial_turning_time_s, 1),
                opt(z.period_s, 1)
            );
        }
        if let Some(st) = &self.stopping {
            s += &format!(
                "Stopping from {:.2} m/s: track reach {} m, head reach {} m, {} s{}\n",
                st.approach_speed_mps,
                opt(st.track_reach_m, 1),
                opt(st.head_reach_m, 1),
                opt(st.time_to_stop_s, 1),
                if st.astern_thrust_n > 0.0 {
                    ""
                } else {
                    " (coasting, no astern thrust)"
                }
            );
        }
        if !self.waves.is_empty() {
            let peak = |f: fn(&WaveResponse) -> f64| {
                self.waves
                    .iter()
                    .max_by(|a, b| f(a).total_cmp(&f(b)))
                    .map(|w| (f(w), w.period_s, w.heading_deg))
                    .unwrap_or((f64::NAN, 0.0, 0.0))
            };
            let (h, ht, hh) = peak(|w| w.heave_rao);
            let (r, rt, rh) = peak(|w| w.roll_rao);
            let (p, pt, ph) = peak(|w| w.pitch_rao);
            s += &format!(
                "Waves: peak heave RAO {h:.2} ({ht:.1} s, {hh:.0}°), roll {r:.2} ({rt:.1} s, {rh:.0}°), pitch {p:.2} ({pt:.1} s, {ph:.0}°)\n"
            );
        }
        for c in &self.checks {
            s += &format!(
                "{} {}: {} {} {} {}{}\n",
                if c.passed { "PASS" } else { "FAIL" },
                c.name,
                opt(c.value, 2),
                if c.bound == "max" { "≤" } else { "≥" },
                c.limit,
                c.unit,
                if c.value.is_none() { " (no value)" } else { "" }
            );
        }
        for k in &self.skipped {
            s += &format!("Skipped: {k}\n");
        }
        for w in &self.warnings {
            s += &format!("Warning: {w}\n");
        }
        s
    }

    /// Writes trials.json, summary.txt and CSV tables into `folder`.
    pub fn write(&self, folder: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let put = |name: &str, bytes: &[u8]| {
            let path = folder.join(name);
            let temp = path.with_extension("partial");
            std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&temp, &path).map_err(|e| e.to_string())
        };
        put(
            "trials.json",
            &serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )?;
        put("summary.txt", self.summary().as_bytes())?;
        let opt = |v: Option<f64>| v.map_or(String::new(), |v| format!("{v:.5}"));
        let mut speed = String::from(
            "fraction,thrust_n,speed_mps,steady,time_to_90_s,resistance_coefficient\n",
        );
        for p in &self.speed {
            speed += &format!(
                "{},{:.3},{:.5},{},{},{}\n",
                p.fraction,
                p.thrust_n,
                p.speed_mps,
                p.steady,
                opt(p.time_to_90_s),
                opt(p.resistance_coefficient)
            );
        }
        put("speed.csv", speed.as_bytes())?;
        let mut decay = String::from("axis,time_s,value\n");
        for d in &self.decay {
            for [t, v] in &d.series {
                decay += &format!("{},{t:.3},{v:.6}\n", d.axis);
            }
        }
        put("decay.csv", decay.as_bytes())?;
        let mut tracks = String::from("trial,time_s,along_m,port_m\n");
        for t in &self.turning {
            for [time, a, c] in &t.track {
                tracks += &format!("turn-{},{time:.2},{a:.4},{c:.4}\n", t.side);
            }
        }
        if let Some(st) = &self.stopping {
            for [time, a, c] in &st.track {
                tracks += &format!("stopping,{time:.2},{a:.4},{c:.4}\n");
            }
        }
        put("tracks.csv", tracks.as_bytes())?;
        let mut zig = String::from("angle_deg,time_s,heading_deg,command\n");
        for z in &self.zigzag {
            for [t, h, c] in &z.series {
                zig += &format!("{},{t:.2},{h:.4},{c}\n", z.angle_deg);
            }
        }
        put("zigzag.csv", zig.as_bytes())?;
        let mut waves = String::from(
            "heading_deg,period_s,height_m,wavelength_m,heave_m,roll_deg,pitch_deg,heave_rao,roll_rao,pitch_rao\n",
        );
        for w in &self.waves {
            waves += &format!(
                "{},{},{},{:.3},{:.5},{:.4},{:.4},{:.4},{:.4},{:.4}\n",
                w.heading_deg,
                w.period_s,
                w.height_m,
                w.wavelength_m,
                w.heave_m,
                w.roll_deg,
                w.pitch_deg,
                w.heave_rao,
                w.roll_rao,
                w.pitch_rao
            );
        }
        put("waves.csv", waves.as_bytes())
    }
}

/// Runs the selected trials for `vehicle` on up to `threads` threads.
/// `progress` is called on the calling thread with (completed, total).
pub fn run(
    vehicle: &Entity,
    environment: &Environment,
    setup: &Setup,
    criteria: &Criteria,
    threads: usize,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Report, String> {
    let bench = Bench::new(vehicle, environment, setup)?;
    let mut skipped = vec![];
    let tasks = bench.tasks(&mut skipped);
    let total = tasks.len();
    let next = Arc::new(AtomicUsize::new(0));
    let (send, receive) = mpsc::channel();
    let mut pieces: Vec<Option<Result<Piece, String>>> = (0..total).map(|_| None).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, 16).min(total.max(1)) {
            let send = send.clone();
            let next = next.clone();
            let (bench, tasks) = (&bench, &tasks);
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= total || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    if send.send((index, bench.run(tasks[index]))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(send);
        let mut done = 0;
        for (index, piece) in receive {
            pieces[index] = Some(piece);
            done += 1;
            progress(done, total);
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err("Trials cancelled.".into());
    }
    let mut report = Report {
        format: FORMAT.into(),
        engine: format!("pelagic {}", env!("CARGO_PKG_VERSION")),
        build_id: crate::BUILD_ID.into(),
        vehicle: VehicleDefinition::from_entity(vehicle),
        mode: if bench.surface {
            "surface"
        } else {
            "submerged"
        }
        .into(),
        length_m: bench.length,
        forward_thrust_n: bench.forward,
        astern_thrust_n: bench.astern,
        yaw_authority_nm: bench.yaw,
        steering: if bench.rudder > 0.0 {
            "rudder"
        } else {
            "thrusters"
        }
        .into(),
        environment: environment.clone(),
        setup: setup.clone(),
        criteria: criteria.clone(),
        decay: vec![],
        speed: vec![],
        turning: vec![],
        zigzag: vec![],
        stopping: None,
        waves: vec![],
        checks: vec![],
        skipped,
        warnings: vec![],
        notes: bench.notes(),
    };
    for (task, piece) in tasks.iter().zip(pieces) {
        match piece {
            Some(Ok(Piece::Decay(d))) => report.decay.push(d),
            Some(Ok(Piece::Speed(p))) => report.speed.push(p),
            Some(Ok(Piece::Turn(t))) => report.turning.push(t),
            Some(Ok(Piece::Zigzag(z))) => report.zigzag.push(z),
            Some(Ok(Piece::Stop(s))) => report.stopping = Some(s),
            Some(Ok(Piece::Wave(w))) => report.waves.push(w),
            Some(Err(e)) => report.skipped.push(format!("{}: {e}", name(*task))),
            None => report.skipped.push(format!("{}: did not run", name(*task))),
        }
    }
    report.checks = judge(&report);
    report.warnings = warnings(&report, vehicle);
    Ok(report)
}

/// Results that need attention before they are relied on.
fn warnings(report: &Report, vehicle: &Entity) -> Vec<String> {
    let mut out = vec![];
    let m = &vehicle.model;
    if m.drag.is_none() || m.angular_drag.is_none() || m.inertia.is_none() {
        out.push(
            "Inertia or drag uses the legacy mass-scaled defaults, which give far too little rotational damping for most hulls. Use Estimate in Details (or measured coefficients) before relying on manoeuvring results.".into(),
        );
    }
    for d in &report.decay {
        if d.axis != "heave" && d.equilibrium.abs() > 10.0 {
            out.push(format!(
                "The vehicle settles at {:.0}° of {}: it is not stable upright (centre of mass too high for its hull).",
                d.equilibrium, d.axis
            ));
        }
    }
    let mut tilted = vec![];
    for p in &report.speed {
        if p.max_tilt_deg > 30.0 {
            tilted.push(format!(
                "speed {:.0} % ({:.0}°)",
                p.fraction * 100.0,
                p.max_tilt_deg
            ));
        }
    }
    for t in &report.turning {
        if t.max_tilt_deg > 30.0 {
            tilted.push(format!("{} turn ({:.0}°)", t.side, t.max_tilt_deg));
        }
    }
    for z in &report.zigzag {
        if z.max_tilt_deg > 30.0 {
            tilted.push(format!(
                "{:.0}° zig-zag ({:.0}°)",
                z.angle_deg, z.max_tilt_deg
            ));
        }
    }
    if let Some(s) = report.stopping.as_ref().filter(|s| s.max_tilt_deg > 30.0) {
        tilted.push(format!("stopping ({:.0}°)", s.max_tilt_deg));
    }
    if !tilted.is_empty() {
        out.push(format!(
            "Tilted beyond 30° in: {}. Check stability: a streamlined hull needs fins against its Munk moment, a vessel needs enough metacentric height for its turning forces, and every hull needs realistic rotational damping.",
            tilted.join(", ")
        ));
    }
    out
}

fn name(task: Task) -> &'static str {
    match task {
        Task::Decay(_) => "Free decay",
        Task::Speed(_) => "Speed trial",
        Task::Turn(_) => "Turning circle",
        Task::Zigzag(_) => "Zig-zag",
        Task::Stop => "Stopping",
        Task::Wave(..) => "Wave response",
    }
}
