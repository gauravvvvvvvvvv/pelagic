//! Ship handling and the collision regulations (COLREGs 1972, Part B):
//! encounter geometry, the role of each vessel, a reference
//! collision-avoidance autopilot, scenario sessions with the vehicle's own
//! physics, and a rule-by-rule debrief.
//!
//! - **Geometry.** Closest point of approach (CPA) and time to it (TCPA)
//!   from positions and velocities; relative bearings from the bow.
//! - **Roles** (Rules 13–15, 18), fixed when risk of collision is first
//!   found (Rule 13(d): a later change of bearing does not change the
//!   overtaking vessel's duty): overtaking if coming up from more than
//!   22.5° abaft the other's beam; head-on when each sees the other within
//!   a few degrees of ahead; otherwise crossing, giving way to a vessel on
//!   the starboard side; a power-driven vessel keeps out of the way of
//!   sailing, fishing, restricted and not-under-command vessels.
//! - **Autopilot.** A velocity-obstacle search over course and speed with
//!   COLREG penalties (starboard alterations, not crossing ahead, stand-on
//!   keeps course and speed until Rule 17 allows or requires action,
//!   substantial rather than small alterations), then back to the route.
//! - **Debrief.** Per target: passing distance, when action was taken and
//!   how large, its direction, passing astern, and stand-on conduct.
//!
//! The numeric thresholds (safe passing distance, "early", "substantial",
//! head-on sector) are configurable interpretations used in training and
//! autonomy research; the Regulations themselves give no numbers.
use crate::{
    entities::Entity,
    helm::Helm,
    math::{Det, Quat, Vec3},
    sar::{bearing, towards},
    scene::Scene,
    trials::{self, Bench, Rig},
    waves::SeaState,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const FORMAT: &str = "pelagic-colregs";
const DT: f64 = 1.0 / 60.0;

/// Vessel categories for Rule 18 (responsibilities between vessels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VesselKind {
    PowerDriven,
    Sailing,
    Fishing,
    RestrictedManoeuvrability,
    NotUnderCommand,
}
impl VesselKind {
    pub fn name(self) -> &'static str {
        match self {
            VesselKind::PowerDriven => "power-driven",
            VesselKind::Sailing => "sailing",
            VesselKind::Fishing => "engaged in fishing",
            VesselKind::RestrictedManoeuvrability => "restricted in ability to manoeuvre",
            VesselKind::NotUnderCommand => "not under command",
        }
    }
    /// Rule 18 precedence: others keep out of the way of higher ranks.
    fn rank(self) -> u8 {
        match self {
            VesselKind::PowerDriven => 0,
            VesselKind::Sailing => 1,
            VesselKind::Fishing => 2,
            VesselKind::RestrictedManoeuvrability | VesselKind::NotUnderCommand => 3,
        }
    }
}

/// Own ship's situation towards one target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Situation {
    HeadOn,
    CrossingGiveWay,
    CrossingStandOn,
    Overtaking,
    BeingOvertaken,
    GiveWayRule18,
    StandOnRule18,
}
impl Situation {
    /// Own ship must keep out of the way.
    pub fn give_way(self) -> bool {
        matches!(
            self,
            Situation::HeadOn
                | Situation::CrossingGiveWay
                | Situation::Overtaking
                | Situation::GiveWayRule18
        )
    }
    pub fn label(self) -> &'static str {
        match self {
            Situation::HeadOn => "Head-on: both alter to starboard (Rule 14)",
            Situation::CrossingGiveWay => "Crossing: give way (Rule 15)",
            Situation::CrossingStandOn => "Crossing: stand on (Rules 15, 17)",
            Situation::Overtaking => "Overtaking: keep clear (Rule 13)",
            Situation::BeingOvertaken => "Being overtaken: stand on (Rules 13, 17)",
            Situation::GiveWayRule18 => "Keep out of the way (Rule 18)",
            Situation::StandOnRule18 => "Other vessel keeps out of the way (Rule 18)",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Situation::HeadOn => "Head-on · R14",
            Situation::CrossingGiveWay => "Crossing · give way · R15",
            Situation::CrossingStandOn => "Crossing · stand on · R17",
            Situation::Overtaking => "Overtaking · give way · R13",
            Situation::BeingOvertaken => "Overtaken · stand on · R13",
            Situation::GiveWayRule18 => "Give way · R18",
            Situation::StandOnRule18 => "Stand on · R18",
        }
    }
}

/// Position (m east/north), ground velocity (m/s) and compass heading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    pub position: [f64; 2],
    pub velocity: [f64; 2],
    pub heading_deg: f64,
}
impl Motion {
    pub fn speed(&self) -> f64 {
        self.velocity[0].hypot(self.velocity[1])
    }
    /// Compass course over the ground (the heading when stopped).
    pub fn course_deg(&self) -> f64 {
        if self.speed() < 0.05 {
            self.heading_deg
        } else {
            bearing(self.velocity)
        }
    }
}

/// Closest point of approach: distance (m) and time to it (s; negative
/// when it has passed).
pub fn cpa(a: &Motion, b: &Motion) -> (f64, f64) {
    let dp = [b.position[0] - a.position[0], b.position[1] - a.position[1]];
    let dv = [b.velocity[0] - a.velocity[0], b.velocity[1] - a.velocity[1]];
    let v2 = dv[0] * dv[0] + dv[1] * dv[1];
    let t = if v2 < 1e-12 {
        0.0
    } else {
        -(dp[0] * dv[0] + dp[1] * dv[1]) / v2
    };
    ((dp[0] + dv[0] * t).hypot(dp[1] + dv[1] * t), t)
}

/// Relative bearing (0–360°, clockwise from the bow) of `to` from `from`.
pub fn relative_bearing(from: &Motion, to: [f64; 2]) -> f64 {
    let d = [to[0] - from.position[0], to[1] - from.position[1]];
    (bearing(d) - from.heading_deg).rem_euclid(360.0)
}

/// Signed compass difference b − a in (−180, 180] (positive clockwise).
pub fn turn(a: f64, b: f64) -> f64 {
    let d = (b - a).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// Configurable interpretations of the Regulations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rules {
    /// Passing distance to keep (m).
    pub safe_cpa_m: f64,
    /// Head-on when each sees the other within this angle of ahead (°).
    pub head_on_deg: f64,
    /// A give-way vessel acts before the range falls below this (m).
    pub early_range_m: f64,
    /// A stand-on vessel may act from this range when the give-way vessel
    /// is not acting (Rule 17(a)(ii)), and must at `extremis_range_m`.
    pub may_act_range_m: f64,
    pub extremis_range_m: f64,
    /// A substantial alteration of course (°) (Rule 8(b)).
    pub substantial_deg: f64,
    /// Course (°) or speed (fraction) change that counts as action.
    pub action_deg: f64,
    pub action_speed: f64,
}
impl Default for Rules {
    fn default() -> Self {
        Self::for_length(7.0)
    }
}
impl Rules {
    /// Distances scaled to own ship's length (15 lengths, 50 m – 1 NM).
    pub fn for_length(length: f64) -> Self {
        let safe = (15.0 * length).clamp(50.0, 1852.0);
        Self {
            safe_cpa_m: safe,
            head_on_deg: 10.0,
            early_range_m: 6.0 * safe,
            may_act_range_m: 4.0 * safe,
            extremis_range_m: 2.0 * safe,
            substantial_deg: 30.0,
            action_deg: 10.0,
            action_speed: 0.3,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let ok = |v: f64, lo: f64, hi: f64| v.is_finite() && v >= lo && v <= hi;
        if !ok(self.safe_cpa_m, 5.0, 20_000.0)
            || !ok(self.head_on_deg, 1.0, 45.0)
            || !ok(self.early_range_m, self.safe_cpa_m, 200_000.0)
            || !ok(
                self.may_act_range_m,
                self.extremis_range_m,
                self.early_range_m,
            )
            || !ok(self.extremis_range_m, self.safe_cpa_m * 0.5, 200_000.0)
            || !ok(self.substantial_deg, 5.0, 120.0)
            || !ok(self.action_deg, 1.0, 60.0)
            || !ok(self.action_speed, 0.05, 0.9)
        {
            return Err("Check the rule settings: early ≥ may act ≥ extremis range ≥ half the safe distance.".into());
        }
        Ok(())
    }
}

/// Own ship's situation towards a target (Rules 13, 14, 15 and 18).
pub fn classify(
    own: &Motion,
    own_kind: VesselKind,
    target: &Motion,
    target_kind: VesselKind,
    rules: &Rules,
) -> Situation {
    let seen = relative_bearing(own, target.position);
    let seen_by = relative_bearing(target, own.position);
    let abaft = |b: f64| (112.5..=247.5).contains(&b);
    // Rule 13 applies notwithstanding Rule 18.
    if abaft(seen_by) && own.speed() > target.speed() {
        return Situation::Overtaking;
    }
    if abaft(seen) && target.speed() > own.speed() {
        return Situation::BeingOvertaken;
    }
    match own_kind.rank().cmp(&target_kind.rank()) {
        std::cmp::Ordering::Less => return Situation::GiveWayRule18,
        std::cmp::Ordering::Greater => return Situation::StandOnRule18,
        std::cmp::Ordering::Equal => {}
    }
    let ahead = |b: f64| b <= rules.head_on_deg || b >= 360.0 - rules.head_on_deg;
    if ahead(seen) && ahead(seen_by) {
        return Situation::HeadOn;
    }
    if seen < 180.0 {
        Situation::CrossingGiveWay
    } else {
        Situation::CrossingStandOn
    }
}

/// An order to a target: from `at_s`, steer `course_deg` at `speed_mps`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Order {
    pub at_s: f64,
    pub course_deg: f64,
    pub speed_mps: f64,
}

/// A target vessel (moves kinematically, turning at a rate set by size).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetSpec {
    pub name: String,
    pub kind: VesselKind,
    pub length_m: f64,
    pub position: [f64; 2],
    pub course_deg: f64,
    pub speed_mps: f64,
    #[serde(default)]
    pub orders: Vec<Order>,
}

/// An encounter scenario: own ship starts at the origin on its route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub description: String,
    pub own_kind: VesselKind,
    /// Own ship's planned course (compass °) and speed (m/s).
    pub route_course_deg: f64,
    pub speed_mps: f64,
    pub targets: Vec<TargetSpec>,
    pub duration_s: f64,
}

/// Places a target that reaches own ship's track point at time `t` with
/// course `course` and speed `speed`, missing by `miss` (m, + passes
/// ahead of own ship).
fn meet(own_speed: f64, t: f64, course: f64, speed: f64, miss: f64) -> [f64; 2] {
    let c = [0.0, own_speed * t];
    let v = towards(speed, course);
    let rel = [v[0], v[1] - own_speed];
    let len = rel[0].hypot(rel[1]).max(1e-9);
    let side = [-rel[1] / len, rel[0] / len];
    [
        c[0] - v[0] * t + side[0] * miss,
        c[1] - v[1] * t + side[1] * miss,
    ]
}

/// Time for a target on a collision course to close to `range`.
fn time_at_range(own_speed: f64, course: f64, speed: f64, t_meet: f64, range: f64) -> f64 {
    let v = towards(speed, course);
    let closing = v[0].hypot(v[1] - own_speed).max(1e-6);
    (t_meet - range / closing).max(0.0)
}

/// Time for the meeting so the start range is `start_range`.
fn meeting_time(own_speed: f64, course: f64, speed: f64, start_range: f64) -> f64 {
    let v = towards(speed, course);
    start_range / v[0].hypot(v[1] - own_speed).max(1e-6)
}

fn target(
    name: &str,
    kind: VesselKind,
    length: f64,
    at: [f64; 2],
    course: f64,
    speed: f64,
) -> TargetSpec {
    TargetSpec {
        name: name.into(),
        kind,
        length_m: length,
        position: at,
        course_deg: course,
        speed_mps: speed,
        orders: vec![],
    }
}

/// The standard encounter set for own ship's speed and length.
pub fn standard(speed: f64, length: f64, rules: &Rules) -> Vec<Scenario> {
    let v = speed.max(0.5);
    let r0 = rules.early_range_m * 10.0 / 6.0;
    let len = length * 1.5;
    let power = VesselKind::PowerDriven;
    let mut out = vec![];
    let scenario = |name: &str, description: &str, targets: Vec<TargetSpec>, t: f64| Scenario {
        name: name.into(),
        description: description.into(),
        own_kind: power,
        route_course_deg: 0.0,
        speed_mps: v,
        duration_s: (2.0 * t).max(120.0) + 60.0,
        targets,
    };
    // Head-on: the other vessel also alters to starboard.
    let t = meeting_time(v, 180.0, v, r0);
    let mut a = target("Ship A", power, len, meet(v, t, 180.0, v, 0.0), 180.0, v);
    let act = time_at_range(v, 180.0, v, t, rules.early_range_m * 0.9);
    a.orders = vec![
        Order {
            at_s: act,
            course_deg: 220.0,
            speed_mps: v,
        },
        Order {
            at_s: t + 60.0,
            course_deg: 180.0,
            speed_mps: v,
        },
    ];
    out.push(scenario(
        "Head-on",
        "A power-driven vessel on a reciprocal course. Both alter to starboard and pass port to port (Rule 14).",
        vec![a],
        t,
    ));
    // Crossing from starboard: own ship gives way.
    let t = meeting_time(v, 270.0, v, r0);
    out.push(scenario(
        "Crossing, give way",
        "A vessel crossing from your starboard side keeps her course and speed. Keep out of her way and avoid crossing ahead (Rules 15, 16).",
        vec![target("Ship B", power, len, meet(v, t, 270.0, v, 0.0), 270.0, v)],
        t,
    ));
    // Crossing from port, the other vessel gives way.
    let t = meeting_time(v, 90.0, v, r0);
    let mut c = target("Ship C", power, len, meet(v, t, 90.0, v, 0.0), 90.0, v);
    let act = time_at_range(v, 90.0, v, t, rules.early_range_m * 0.9);
    c.orders = vec![
        Order {
            at_s: act,
            course_deg: 150.0,
            speed_mps: v,
        },
        Order {
            at_s: t + 90.0,
            course_deg: 90.0,
            speed_mps: v,
        },
    ];
    out.push(scenario(
        "Crossing, stand on",
        "A vessel crossing from your port side gives way. Keep your course and speed (Rule 17(a)(i)).",
        vec![c],
        t,
    ));
    // Crossing from port, the other vessel does not act.
    out.push(scenario(
        "Crossing, stand on: other vessel does not act",
        "The give-way vessel on your port side holds on. Act when it is apparent she is not keeping out of the way (Rule 17(a)(ii)); do not alter to port (17(c)); act in time to avoid collision (17(b)).",
        vec![target("Ship D", power, len, meet(v, t, 90.0, v, 0.0), 90.0, v)],
        t,
    ));
    // Overtaking a slower vessel.
    let t = meeting_time(v, 0.0, 0.4 * v, r0);
    out.push(scenario(
        "Overtaking",
        "You come up on a slower vessel from astern. Keep out of her way until finally past and clear (Rule 13).",
        vec![target("Ship E", power, len, meet(v, t, 0.0, 0.4 * v, 0.0), 0.0, 0.4 * v)],
        t,
    ));
    // Being overtaken by a faster vessel, which keeps clear.
    let t = meeting_time(v, 0.0, 1.8 * v, r0);
    let mut f = target(
        "Ship F",
        power,
        len,
        meet(v, t, 0.0, 1.8 * v, 0.0),
        0.0,
        1.8 * v,
    );
    let act = time_at_range(v, 0.0, 1.8 * v, t, rules.early_range_m * 0.6);
    f.orders = vec![
        Order {
            at_s: act,
            course_deg: 25.0,
            speed_mps: 1.8 * v,
        },
        Order {
            at_s: t + 60.0,
            course_deg: 0.0,
            speed_mps: 1.8 * v,
        },
    ];
    out.push(scenario(
        "Being overtaken",
        "A faster vessel comes up from astern and keeps clear. Keep your course and speed (Rules 13, 17).",
        vec![f],
        t,
    ));
    // A fishing vessel on the port side: Rule 18 makes own ship give way.
    let t = meeting_time(v, 90.0, 0.6 * v, r0);
    out.push(scenario(
        "Fishing vessel crossing from port",
        "A vessel engaged in fishing crosses from your port side. Although she is on your port side, a power-driven vessel keeps out of her way (Rule 18).",
        vec![target("Trawler G", VesselKind::Fishing, len, meet(v, t, 90.0, 0.6 * v, 0.0), 90.0, 0.6 * v)],
        t,
    ));
    // Two vessels at once.
    let t = meeting_time(v, 180.0, v, r0);
    let mut h = target("Ship H", power, len, meet(v, t, 180.0, v, 0.0), 180.0, v);
    let act = time_at_range(v, 180.0, v, t, rules.early_range_m * 0.9);
    h.orders = vec![
        Order {
            at_s: act,
            course_deg: 215.0,
            speed_mps: v,
        },
        Order {
            at_s: t + 60.0,
            course_deg: 180.0,
            speed_mps: v,
        },
    ];
    let t2 = t * 1.15;
    let j = target(
        "Ship J",
        power,
        len,
        meet(v, t2, 250.0, 0.8 * v, 0.0),
        250.0,
        0.8 * v,
    );
    out.push(scenario(
        "Head-on and crossing together",
        "A head-on vessel and a vessel crossing from starboard arrive together. One alteration must resolve both.",
        vec![h, j],
        t2,
    ));
    out
}

/// Random encounters (seeded): 1–4 targets on or near collision courses.
pub fn random(speed: f64, length: f64, rules: &Rules, seed: u64) -> Scenario {
    let mut state = seed ^ 0xc01_2e65;
    let mut rnd = || crate::life::random(&mut state);
    let v = speed.max(0.5);
    let n = 1 + (rnd() * 4.0) as usize;
    let mut targets = vec![];
    let mut longest: f64 = 0.0;
    for i in 0..n {
        let course = (rnd() * 360.0).round();
        let sp = v * (0.4 + rnd() * 1.1);
        let range = rules.early_range_m * (1.4 + rnd() * 0.8);
        let t = meeting_time(v, course, sp, range);
        longest = longest.max(t);
        let miss = (rnd() - 0.5) * rules.safe_cpa_m;
        let kind = match (rnd() * 10.0) as u32 {
            0 => VesselKind::Fishing,
            1 => VesselKind::Sailing,
            _ => VesselKind::PowerDriven,
        };
        targets.push(target(
            &format!("Target {}", i + 1),
            kind,
            length * (1.0 + rnd() * 3.0),
            meet(v, t, course, sp, miss),
            course,
            sp,
        ));
    }
    Scenario {
        name: format!("Random encounter {seed}"),
        description: format!(
            "{n} vessel{} on or near collision courses; they keep their course and speed.",
            if n == 1 { "" } else { "s" }
        ),
        own_kind: VesselKind::PowerDriven,
        route_course_deg: 0.0,
        speed_mps: v,
        targets,
        duration_s: 2.0 * longest + 60.0,
    }
}

/// A target during a session.
#[derive(Clone, Debug, PartialEq)]
struct Target {
    spec: TargetSpec,
    motion: Motion,
    ordered: (f64, f64),
    turn_rate: f64,
    next_order: usize,
}
impl Target {
    fn new(spec: TargetSpec) -> Self {
        let motion = Motion {
            position: spec.position,
            velocity: towards(spec.speed_mps, spec.course_deg),
            heading_deg: spec.course_deg,
        };
        Self {
            ordered: (spec.course_deg, spec.speed_mps),
            turn_rate: (180.0 / spec.length_m.max(1.0)).clamp(0.3, 5.0),
            next_order: 0,
            motion,
            spec,
        }
    }
    fn step(&mut self, time: f64, dt: f64) {
        while let Some(o) = self.spec.orders.get(self.next_order) {
            if o.at_s > time {
                break;
            }
            self.ordered = (o.course_deg, o.speed_mps);
            self.next_order += 1;
        }
        let m = &mut self.motion;
        let d = turn(m.heading_deg, self.ordered.0);
        let step = self.turn_rate * dt;
        m.heading_deg = (m.heading_deg + d.clamp(-step, step)).rem_euclid(360.0);
        let speed = m.speed();
        let s = speed + (self.ordered.1 - speed).clamp(-0.05 * dt, 0.05 * dt);
        m.velocity = towards(s, m.heading_deg);
        m.position[0] += m.velocity[0] * dt;
        m.position[1] += m.velocity[1] * dt;
    }
}

/// A sampled moment of a session (every second).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub t: f64,
    pub own: Motion,
    pub targets: Vec<Motion>,
    /// Ordered course (°) and speed (m/s), and whether the autopilot had
    /// the helm.
    pub order: (f64, f64),
    pub autopilot: bool,
}

/// What the display shows for each target.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetView {
    pub name: String,
    pub kind: VesselKind,
    pub length_m: f64,
    pub motion: Motion,
    pub range_m: f64,
    pub bearing_deg: f64,
    pub cpa_m: f64,
    pub tcpa_s: f64,
    /// Fixed when risk of collision was first found.
    pub situation: Option<Situation>,
    pub collided: bool,
}

#[derive(Clone, Debug, Default)]
struct Planner {
    next: f64,
    command: Option<(f64, f64)>,
    avoiding: bool,
    /// Heading when the avoiding action began: alterations are measured
    /// from it, so finishing a turn is not a new "small" alteration.
    base: Option<f64>,
}

/// A running encounter: own ship with its physics, targets, helm orders or
/// the autopilot, and a log for the debrief.
pub struct Session {
    rig: Rig,
    helm: Helm,
    scenario: Scenario,
    rules: Rules,
    own_length: f64,
    targets: Vec<Target>,
    roles: Vec<Option<(Situation, f64)>>,
    collided: Vec<bool>,
    time: f64,
    order: (f64, f64),
    autopilot: bool,
    planner: Planner,
    log: Vec<Sample>,
    vehicle: String,
}

/// Own ship's service speed (m/s, at 65 % thrust in calm water) and length.
pub fn service_speed(scene: &Scene, vehicle: &Entity) -> Result<(f64, f64), String> {
    let bench = bench(scene, vehicle)?;
    let mut rig = bench.rig(
        &bench.entity,
        SeaState::default(),
        false,
        Vec3::new(0.0, 0.0, bench.start_z),
        Quat::IDENTITY,
    )?;
    let (speed, _, _) = bench.approach(&mut rig, 0.65 * bench.forward);
    Ok((speed, bench.length))
}

fn bench(scene: &Scene, vehicle: &Entity) -> Result<Bench, String> {
    let env = trials::Environment {
        water_depth: scene.bottom_depth.clamp(20.0, 11_000.0),
        ..trials::Environment::from_scene(scene)
    };
    let bench = Bench::new(vehicle, &env, &trials::Setup::quick())?;
    if !bench.surface {
        return Err(format!(
            "{} does not float: encounter training needs a surface vessel.",
            vehicle.name
        ));
    }
    if bench.forward <= 0.0 {
        return Err(format!("{} has no forward thrust.", vehicle.name));
    }
    Ok(bench)
}

impl Session {
    /// Starts `scenario` with `vehicle` as own ship in the scene's sea and
    /// current (open water).
    pub fn new(
        scene: &Scene,
        vehicle: &Entity,
        scenario: Scenario,
        rules: Rules,
    ) -> Result<Self, String> {
        rules.validate()?;
        if scenario.targets.is_empty() || scenario.targets.len() > 32 {
            return Err("A scenario needs 1 to 32 target vessels.".into());
        }
        let bench = bench(scene, vehicle)?;
        let sea = crate::scene::sea_state(scene);
        let yaw = (90.0 - scenario.route_course_deg).to_radians();
        let mut rig = bench.rig(
            &bench.entity,
            sea,
            true,
            Vec3::new(0.0, 0.0, bench.start_z),
            Quat::from_rotation_vector(Vec3::new(0.0, 0.0, yaw)),
        )?;
        let current = Vec3::new(scene.current[0], scene.current[1], scene.current[2]);
        if let Some(v) = rig.world.vehicle_mut(0) {
            let (s, c) = yaw.dsin_cos();
            v.velocity = current + Vec3::new(c, s, 0.0) * scenario.speed_mps;
        }
        let mut helm = Helm::new(&bench, scene.density, true);
        // Start at speed: thrust for the scenario speed, from the calm-water
        // speed at 65 % thrust and a square-law resistance.
        let mut calm = bench.rig(
            &bench.entity,
            SeaState::default(),
            false,
            Vec3::new(0.0, 0.0, bench.start_z),
            Quat::IDENTITY,
        )?;
        let (s65, _, _) = bench.approach(&mut calm, 0.65 * bench.forward);
        if s65 > 0.05 {
            helm.prime(0.65 * bench.forward * (scenario.speed_mps / s65).powi(2));
        }
        let n = scenario.targets.len();
        Ok(Self {
            rig,
            helm,
            own_length: bench.length,
            targets: scenario.targets.iter().cloned().map(Target::new).collect(),
            roles: vec![None; n],
            collided: vec![false; n],
            time: 0.0,
            order: (scenario.route_course_deg, scenario.speed_mps),
            autopilot: false,
            planner: Planner::default(),
            log: vec![],
            vehicle: vehicle.name.clone(),
            rules,
            scenario,
        })
    }

    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }
    pub fn rules(&self) -> &Rules {
        &self.rules
    }
    pub fn time(&self) -> f64 {
        self.time
    }
    pub fn finished(&self) -> bool {
        self.time >= self.scenario.duration_s
    }
    pub fn autopilot(&self) -> bool {
        self.autopilot
    }
    pub fn order(&self) -> (f64, f64) {
        self.order
    }
    pub fn log(&self) -> &[Sample] {
        &self.log
    }
    pub fn own_length(&self) -> f64 {
        self.own_length
    }
    pub fn vehicle(&self) -> &str {
        &self.vehicle
    }
    /// Helm order: course (compass °) and speed through the water (m/s).
    /// Takes the helm from the autopilot.
    pub fn set_order(&mut self, course_deg: f64, speed_mps: f64) {
        self.autopilot = false;
        self.order = (course_deg.rem_euclid(360.0), speed_mps.clamp(0.0, 50.0));
    }
    pub fn set_autopilot(&mut self, on: bool) {
        self.autopilot = on;
        self.planner = Planner::default();
    }

    /// Own ship: position, ground velocity, heading.
    pub fn own(&self) -> Motion {
        let v = self.rig.vehicle();
        let (_, _, yaw) = trials::attitude(v.orientation);
        Motion {
            position: [v.position.x, v.position.y],
            velocity: [v.velocity.x, v.velocity.y],
            heading_deg: (90.0 - yaw.to_degrees()).rem_euclid(360.0),
        }
    }
    /// Speed through the water (m/s).
    pub fn speed_through_water(&self) -> f64 {
        self.rig.speed()
    }
    /// Roll and pitch (degrees).
    pub fn attitude_deg(&self) -> (f64, f64) {
        let (r, p, _) = trials::attitude(self.rig.vehicle().orientation);
        (r.to_degrees(), p.to_degrees())
    }

    pub fn targets(&self) -> Vec<TargetView> {
        let own = self.own();
        self.targets
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let (cpa_m, tcpa_s) = cpa(&own, &t.motion);
                let d = [
                    t.motion.position[0] - own.position[0],
                    t.motion.position[1] - own.position[1],
                ];
                TargetView {
                    name: t.spec.name.clone(),
                    kind: t.spec.kind,
                    length_m: t.spec.length_m,
                    motion: t.motion,
                    range_m: d[0].hypot(d[1]),
                    bearing_deg: bearing(d),
                    cpa_m,
                    tcpa_s,
                    situation: self.roles[i].map(|r| r.0),
                    collided: self.collided[i],
                }
            })
            .collect()
    }

    /// Advances by `seconds` (in 1/60 s steps).
    pub fn step(&mut self, seconds: f64) {
        let steps = (seconds / DT).round().max(0.0) as usize;
        for _ in 0..steps {
            if self.finished() {
                return;
            }
            self.tick();
        }
    }

    fn horizon(&self) -> f64 {
        2.0 * self.rules.early_range_m / self.scenario.speed_mps.max(0.5)
    }

    fn tick(&mut self) {
        let own = self.own();
        // Roles fixed when risk of collision is first found.
        for (i, t) in self.targets.iter().enumerate() {
            let (d, tc) = cpa(&own, &t.motion);
            let range = (t.motion.position[0] - own.position[0])
                .hypot(t.motion.position[1] - own.position[1]);
            if self.roles[i].is_none()
                && d < self.rules.safe_cpa_m
                && tc > 0.0
                && (tc < self.horizon() || range < 1.7 * self.rules.early_range_m)
            {
                self.roles[i] = Some((
                    classify(
                        &own,
                        self.scenario.own_kind,
                        &t.motion,
                        t.spec.kind,
                        &self.rules,
                    ),
                    self.time,
                ));
            }
            if range < 0.5 * (self.own_length + t.spec.length_m) {
                self.collided[i] = true;
            }
        }
        if self.autopilot && self.time >= self.planner.next {
            self.planner.next = self.time + 2.0;
            self.order = self.plan(&own);
        }
        let heading = (90.0 - self.order.0).to_radians();
        let (thrust, steer) = self.helm.command(&self.rig, heading, self.order.1);
        self.rig.step(thrust, steer);
        for t in &mut self.targets {
            t.step(self.time, DT);
        }
        self.time += DT;
        if self.log.is_empty() || self.time - self.log.last().map_or(0.0, |s| s.t) >= 1.0 - 1e-9 {
            let own = self.own();
            self.log.push(Sample {
                t: self.time,
                own,
                targets: self.targets.iter().map(|t| t.motion).collect(),
                order: self.order,
                autopilot: self.autopilot,
            });
        }
    }

    /// Route-keeping course back to the planned track (line of sight).
    fn route_course(&self, own: &Motion) -> f64 {
        let c = self.scenario.route_course_deg;
        let u = towards(1.0, c);
        // Positive to the left of the route.
        let cross = -own.position[0] * u[1] + own.position[1] * u[0];
        let lookahead = 3.0 * self.rules.safe_cpa_m;
        (c + (cross / lookahead).datan2(1.0).to_degrees()).rem_euclid(360.0)
    }

    /// The reference collision-avoidance autopilot.
    fn plan(&mut self, own: &Motion) -> (f64, f64) {
        let r = self.rules.clone();
        let mut must_act = false;
        let mut active = vec![];
        let mut near = vec![];
        for (i, t) in self.targets.iter().enumerate() {
            let (d, tc) = cpa(own, &t.motion);
            let range = (t.motion.position[0] - own.position[0])
                .hypot(t.motion.position[1] - own.position[1]);
            // Vessels that matter when returning to the route.
            if range < 1.7 * r.early_range_m || (tc > 0.0 && tc < self.horizon()) {
                near.push(i);
            }
            let Some((role, _)) = self.roles[i] else {
                continue;
            };
            // Finally past and clear (Rule 8(d)): opening, abaft the beam
            // and well off, or far away.
            let abaft = (90.0..=270.0).contains(&relative_bearing(own, t.motion.position));
            if tc < 0.0 && ((abaft && range > 2.0 * r.safe_cpa_m) || range > r.early_range_m) {
                continue;
            }
            active.push(i);
            let risk = d < r.safe_cpa_m && tc > 0.0;
            if role.give_way() {
                if risk || self.planner.avoiding {
                    must_act = true;
                }
            } else if risk && range < r.may_act_range_m {
                // Stand-on: still a risk of collision this close means the
                // give-way vessel's action is not appropriate (Rule
                // 17(a)(ii)); closer still, 17(b) requires action.
                must_act = true;
            }
        }
        if active.is_empty() {
            // Back to the route, never onto a course that closes a vessel
            // nearby.
            self.planner.avoiding = false;
            self.planner.command = None;
            self.planner.base = None;
            let preferred = self.route_course(own);
            let (_, course, speed) = self.search(own, preferred, preferred, &near, false);
            return (course, speed);
        }
        if !must_act {
            // Stand on (or nothing to do yet): keep course and speed.
            return self.planner.command.unwrap_or((self.order.0, self.order.1));
        }
        let base = *self.planner.base.get_or_insert(own.heading_deg);
        let route = self.scenario.route_course_deg;
        let (_, course, speed) = self.search(own, route, base, &active, true);
        self.planner.avoiding = true;
        self.planner.command = Some((course, speed));
        (course, speed)
    }

    /// Velocity-obstacle search over courses (5° steps, ±90° of `base`)
    /// and speeds: the cheapest of deviation from `preferred`, risk of
    /// collision with `consider`, and (with `colregs`) the rules'
    /// penalties. Returns (cost, course, speed).
    fn search(
        &self,
        own: &Motion,
        preferred: f64,
        base: f64,
        consider: &[usize],
        colregs: bool,
    ) -> (f64, f64, f64) {
        let r = &self.rules;
        let cruise = self.scenario.speed_mps;
        let mut best = (f64::MAX, preferred, cruise);
        let mut candidates: Vec<f64> = (-18..=18)
            .map(|k| (base + 5.0 * k as f64).rem_euclid(360.0))
            .collect();
        candidates.push(preferred);
        for course in candidates {
            for factor in [1.0, 0.5, 0.25] {
                let speed = cruise * factor;
                let mut me = *own;
                me.velocity = towards(speed, course);
                me.heading_deg = course;
                let mut cost = 4.0 * turn(preferred, course).abs() + 800.0 * (1.0 - factor);
                let alteration = turn(base, course);
                let to_port = alteration < -1.0;
                for &i in consider {
                    let t = &self.targets[i];
                    let (d, tc) = cpa(&me, &t.motion);
                    if tc > 0.0 {
                        if d < r.safe_cpa_m {
                            cost += 2000.0 + 10_000.0 * (1.0 - d / r.safe_cpa_m).powi(2);
                        } else if d < 1.5 * r.safe_cpa_m {
                            cost += 200.0 * (1.5 - d / r.safe_cpa_m);
                        }
                    }
                    if !colregs {
                        continue;
                    }
                    match self.roles[i].map(|x| x.0) {
                        Some(Situation::HeadOn) | Some(Situation::GiveWayRule18) if to_port => {
                            cost += 3000.0;
                        }
                        Some(Situation::CrossingGiveWay) => {
                            if to_port {
                                cost += 3000.0;
                            }
                            // Rule 15: do not cross ahead.
                            let at = |m: &Motion, s: f64| {
                                [
                                    m.position[0] + m.velocity[0] * s,
                                    m.position[1] + m.velocity[1] * s,
                                ]
                            };
                            let (a, b) = (at(&me, tc.max(0.0)), at(&t.motion, tc.max(0.0)));
                            let dir = towards(1.0, t.motion.heading_deg);
                            if (a[0] - b[0]) * dir[0] + (a[1] - b[1]) * dir[1] > 0.0 {
                                cost += 3000.0;
                            }
                        }
                        Some(Situation::CrossingStandOn) if to_port => cost += 3000.0,
                        _ => {}
                    }
                }
                if colregs {
                    // Rule 8(b): avoid small alterations (a speed change
                    // alone may be substantial; a small turn adds nothing
                    // readily apparent).
                    if alteration.abs() > 1.0 && alteration.abs() < r.substantial_deg {
                        cost += 1500.0;
                    }
                    if let Some((c, s)) = self.planner.command {
                        cost += 2.0 * turn(c, course).abs() + 200.0 * (s - speed).abs() / cruise;
                    }
                }
                if cost < best.0 {
                    best = (cost, course, speed);
                }
            }
        }
        best
    }

    /// The debrief.
    pub fn assess(&self) -> Assessment {
        assess(self)
    }
}

/// Own ship's action against one target.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// When it was first apparent, and the range and TCPA then.
    pub at_s: f64,
    pub range_m: f64,
    pub tcpa_s: f64,
    /// Largest alteration (°, + to starboard) and speed reduction (fraction).
    pub course_change_deg: f64,
    pub speed_reduction: f64,
    /// Largest cut in the ordered speed (fraction): an engine order acts
    /// before the vessel has slowed.
    #[serde(default)]
    pub ordered_cut: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub rule: String,
    pub text: String,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetResult {
    pub name: String,
    pub kind: VesselKind,
    pub situation: Option<Situation>,
    pub min_range_m: f64,
    pub at_s: f64,
    pub action: Option<Action>,
    pub passed_astern: Option<bool>,
    pub collided: bool,
    pub checks: Vec<Check>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assessment {
    pub scenario: String,
    pub vehicle: String,
    pub autopilot: bool,
    pub duration_s: f64,
    pub targets: Vec<TargetResult>,
    pub passed: bool,
    pub rules: Rules,
}

fn assess(s: &Session) -> Assessment {
    let r = &s.rules;
    let log = &s.log;
    // A vessel (other than `i`) own ship had to give way to at time `t`.
    // A vessel (other than `i`) towards which own ship had to act at time
    // `t`: a give-way duty, or a stand-on vessel's permitted or required
    // action (Rule 17(a)(ii), (b)) at close range.
    let duty_at = |i: usize, t: f64| -> Option<String> {
        let k = log.iter().find(|k| k.t >= t)?;
        s.roles.iter().enumerate().find_map(|(j, role)| {
            let (situation, since) = (*role)?;
            let m = &k.targets[j];
            let (d, tc) = cpa(&k.own, m);
            let range =
                (m.position[0] - k.own.position[0]).hypot(m.position[1] - k.own.position[1]);
            let duty =
                situation.give_way() || (d < r.safe_cpa_m && range <= r.may_act_range_m * 1.05);
            (j != i && duty && since <= t + 1.0 && tc > 0.0).then(|| s.targets[j].spec.name.clone())
        })
    };
    if log.is_empty() {
        return Assessment {
            scenario: s.scenario.name.clone(),
            vehicle: s.vehicle.clone(),
            autopilot: s.autopilot,
            duration_s: 0.0,
            targets: vec![],
            passed: false,
            rules: r.clone(),
        };
    }
    let mut results = vec![];
    for (i, t) in s.targets.iter().enumerate() {
        let role = s.roles[i];
        let range = |k: &Sample| {
            let p = k.targets[i].position;
            (p[0] - k.own.position[0]).hypot(p[1] - k.own.position[1])
        };
        let (closest, min_range) = log
            .iter()
            .enumerate()
            .map(|(k, x)| (k, range(x)))
            .fold((0, f64::MAX), |a, b| if b.1 < a.1 { b } else { a });
        let mut checks = vec![];
        let collided = s.collided[i];
        checks.push(Check {
            rule: if collided {
                "Collision".into()
            } else {
                "Rules 8(d), 16".into()
            },
            text: if collided {
                format!("Collision with {}.", t.spec.name)
            } else {
                format!(
                    "Passed at {:.0} m (safe distance {:.0} m).",
                    min_range, r.safe_cpa_m
                )
            },
            passed: !collided && min_range >= r.safe_cpa_m * 0.95,
        });
        // Own ship's action from the moment the role was fixed.
        let mut action = None;
        let mut passed_astern = None;
        if let Some((situation, since)) = role {
            let start = log.iter().position(|x| x.t >= since).unwrap_or(0);
            let base = &log[start];
            // A give-way vessel already off its route (manoeuvring for another
            // vessel) is judged from the planned route; a stand-on vessel from
            // its heading when the risk arose (only later alterations count).
            let route = s.scenario.route_course_deg;
            let base_heading = if turn(route, base.own.heading_deg).abs() >= r.action_deg
                && situation.give_way()
            {
                route
            } else {
                base.own.heading_deg
            };
            let base_speed = base.own.speed().max(0.1);
            // Action up to the closest point of approach (not the return to
            // the route afterwards).
            let end = log[start..]
                .iter()
                .position(|k| cpa(&k.own, &k.targets[i]).1 <= 0.0)
                .map_or(log.len() - 1, |p| start + p)
                .max(start);
            let mut first = None;
            let (mut most, mut slowest) = (0.0f64, 1.0f64);
            let base_order = base.order.1.max(0.1);
            let mut least_order = 1.0f64;
            for k in &log[start..=end] {
                least_order = least_order.min(k.order.1 / base_order);
                let change = turn(base_heading, k.own.heading_deg);
                let ratio = k.own.speed() / base_speed;
                if first.is_none()
                    && (change.abs() >= r.action_deg || ratio <= 1.0 - r.action_speed)
                {
                    first = Some(k);
                }
                if change.abs() > most.abs() {
                    most = change;
                }
                slowest = slowest.min(ratio);
            }
            if let Some(k) = first {
                action = Some(Action {
                    at_s: k.t,
                    range_m: range(k),
                    tcpa_s: cpa(&k.own, &k.targets[i]).1,
                    course_change_deg: most,
                    speed_reduction: 1.0 - slowest,
                    ordered_cut: (1.0 - least_order).max(0.0),
                });
            }
            // Which side of the target own ship passed.
            let k = &log[closest];
            let dir = towards(1.0, k.targets[i].heading_deg);
            let along = (k.own.position[0] - k.targets[i].position[0]) * dir[0]
                + (k.own.position[1] - k.targets[i].position[1]) * dir[1];
            passed_astern = Some(along < 0.0);
            let fmt_action = |a: &Action| {
                format!(
                    "at {:.0} m ({:.0} s before CPA): {:+.0}°{}",
                    a.range_m,
                    a.tcpa_s.max(0.0),
                    a.course_change_deg,
                    if a.speed_reduction > r.action_speed {
                        format!(", speed −{:.0} %", a.speed_reduction * 100.0)
                    } else {
                        String::new()
                    }
                )
            };
            if situation.give_way() {
                match &action {
                    None => {
                        let clear = min_range >= r.safe_cpa_m * 0.95;
                        checks.push(Check {
                            rule: "Rule 16".into(),
                            text: if clear {
                                "Kept clear without a separate alteration.".into()
                            } else {
                                "No action taken as the give-way vessel.".into()
                            },
                            passed: clear,
                        });
                    }
                    Some(a) => {
                        // Early: before the early range, or promptly once
                        // the risk arose (it may arise closer in).
                        let early = a.range_m >= r.early_range_m * 0.95;
                        let prompt = a.at_s - since <= 20.0;
                        checks.push(Check {
                            rule: "Rules 8(a), 16".into(),
                            text: format!(
                                "Action {} — {}.",
                                fmt_action(a),
                                if early {
                                    "early"
                                } else if prompt {
                                    "promptly once the risk arose"
                                } else {
                                    "late"
                                }
                            ),
                            passed: early || prompt,
                        });
                        let substantial = a.course_change_deg.abs() >= r.substantial_deg - 1.0
                            || a.speed_reduction >= 0.5
                            || a.ordered_cut >= 0.5;
                        checks.push(Check {
                            rule: "Rule 8(b)".into(),
                            text: if substantial {
                                "Alteration large enough to be readily apparent.".into()
                            } else {
                                format!(
                                    "Alteration of {:.0}° is small (≥ {:.0}° or an order to half speed or less is readily apparent).",
                                    a.course_change_deg.abs(),
                                    r.substantial_deg
                                )
                            },
                            passed: substantial,
                        });
                    }
                }
                match situation {
                    Situation::HeadOn => {
                        let ok = action.as_ref().is_some_and(|a| a.course_change_deg > 0.0);
                        checks.push(Check {
                            rule: "Rule 14".into(),
                            text: if ok {
                                "Altered to starboard.".into()
                            } else {
                                "Did not alter to starboard in a head-on situation.".into()
                            },
                            passed: ok,
                        });
                    }
                    Situation::CrossingGiveWay => {
                        let astern = passed_astern == Some(true);
                        checks.push(Check {
                            rule: "Rule 15".into(),
                            text: if astern {
                                "Passed astern; did not cross ahead.".into()
                            } else {
                                "Crossed ahead of the stand-on vessel.".into()
                            },
                            passed: astern,
                        });
                    }
                    _ => {}
                }
            } else {
                let other_acted = log.iter().any(|k| {
                    turn(t.spec.course_deg, k.targets[i].heading_deg).abs() >= r.action_deg
                });
                // In a multi-vessel situation own ship may have had to give
                // way to another vessel at the same time.
                let duty_to = action.as_ref().and_then(|a| duty_at(i, a.at_s));
                let early_stand_on = |a: &Action| a.range_m > r.may_act_range_m * 1.02;
                match &action {
                    Some(a) if early_stand_on(a) && duty_to.is_some() => {
                        checks.push(Check {
                            rule: "Rule 17(a)(i)".into(),
                            text: format!(
                                "Course changed early for {} (several vessels at once).",
                                duty_to.clone().unwrap_or_default()
                            ),
                            passed: true,
                        })
                    }
                    Some(a) if early_stand_on(a) => checks.push(Check {
                        rule: "Rule 17(a)(i)".into(),
                        text: format!(
                            "Stand-on vessel acted early, {} (keep course and speed until {:.0} m).",
                            fmt_action(a),
                            r.may_act_range_m
                        ),
                        passed: false,
                    }),
                    Some(a) => checks.push(Check {
                        rule: "Rule 17(a)(ii), (b)".into(),
                        text: format!("Acted {} when the other vessel {}.", fmt_action(a), if other_acted { "was acting" } else { "was not keeping out of the way" }),
                        passed: true,
                    }),
                    None => checks.push(Check {
                        rule: "Rule 17(a)(i)".into(),
                        text: "Kept course and speed.".into(),
                        passed: !(min_range < r.safe_cpa_m && !other_acted),
                    }),
                }
                if situation == Situation::CrossingStandOn
                    && let Some(a) = &action
                {
                    let ok = a.course_change_deg > -r.action_deg;
                    checks.push(Check {
                        rule: "Rule 17(c)".into(),
                        text: if ok {
                            "Did not alter to port for a vessel on the port side.".into()
                        } else {
                            "Altered to port for a vessel on the port side.".into()
                        },
                        passed: ok,
                    });
                }
            }
        } else {
            checks.push(Check {
                rule: "Rule 7".into(),
                text: "No risk of collision developed with this vessel.".into(),
                passed: true,
            });
        }
        results.push(TargetResult {
            name: t.spec.name.clone(),
            kind: t.spec.kind,
            situation: role.map(|x| x.0),
            min_range_m: min_range,
            at_s: log.get(closest).map_or(0.0, |x| x.t),
            action,
            passed_astern,
            collided,
            checks,
        });
    }
    let passed = results.iter().all(|t| t.checks.iter().all(|c| c.passed));
    Assessment {
        scenario: s.scenario.name.clone(),
        vehicle: s.vehicle.clone(),
        autopilot: s.log.iter().all(|x| x.autopilot),
        duration_s: s.time,
        targets: results,
        passed,
        rules: s.rules.clone(),
    }
}

impl Assessment {
    pub fn summary(&self) -> String {
        let mut t = format!(
            "{} — {} ({}): {}\n",
            self.scenario,
            self.vehicle,
            if self.autopilot {
                "autopilot"
            } else {
                "helm orders"
            },
            if self.passed { "PASS" } else { "FAIL" }
        );
        for r in &self.targets {
            t += &format!(
                "  {} ({}): {}; closest {:.0} m\n",
                r.name,
                r.kind.name(),
                r.situation.map_or("no risk of collision", |s| s.label()),
                r.min_range_m
            );
            for c in &r.checks {
                t += &format!(
                    "    {} {:<20} {}\n",
                    if c.passed { "✔" } else { "✖" },
                    c.rule,
                    c.text
                );
            }
        }
        t
    }
}

/// An encounter's debrief and its one-second log.
pub type Outcome = (Assessment, Vec<Sample>);

/// Runs every scenario with the autopilot (in parallel).
pub fn run_suite(
    scene: &Scene,
    vehicle: &Entity,
    scenarios: &[Scenario],
    rules: &Rules,
    threads: usize,
    cancel: &AtomicBool,
    progress: impl Fn(usize, usize) + Sync,
) -> Result<Vec<Outcome>, String> {
    // Fail fast on an unsuitable vehicle.
    bench(scene, vehicle)?;
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let total = scenarios.len();
    let mut out: Vec<(usize, Result<Outcome, String>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.clamp(1, 32).min(total.max(1)))
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = vec![];
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(sc) = scenarios.get(i) else { break };
                        let result =
                            Session::new(scene, vehicle, sc.clone(), rules.clone()).map(|mut s| {
                                s.set_autopilot(true);
                                while !s.finished() {
                                    s.step(1.0);
                                }
                                (s.assess(), s.log.clone())
                            });
                        mine.push((i, result));
                        progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                    }
                    mine
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_default())
            .collect()
    });
    if cancel.load(Ordering::Relaxed) {
        return Err("Cancelled.".into());
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

/// Writes summary.txt, colregs.json and one track CSV per scenario.
pub fn write_suite(folder: &std::path::Path, results: &[Outcome]) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let mut summary = String::from("Pelagic collision-regulations (COLREGs) scenarios\n\n");
    for (a, log) in results {
        summary += &a.summary();
        summary.push('\n');
        let mut csv = String::from(
            "t_s,own_x,own_y,own_heading_deg,own_speed_mps,order_course_deg,order_speed_mps",
        );
        if let Some(first) = log.first() {
            for k in 0..first.targets.len() {
                csv += &format!(",t{k}_x,t{k}_y,t{k}_heading_deg");
            }
        }
        csv.push('\n');
        for x in log {
            csv += &format!(
                "{:.1},{:.2},{:.2},{:.1},{:.3},{:.1},{:.2}",
                x.t,
                x.own.position[0],
                x.own.position[1],
                x.own.heading_deg,
                x.own.speed(),
                x.order.0,
                x.order.1
            );
            for m in &x.targets {
                csv += &format!(
                    ",{:.2},{:.2},{:.1}",
                    m.position[0], m.position[1], m.heading_deg
                );
            }
            csv.push('\n');
        }
        let name: String = a
            .scenario
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        std::fs::write(folder.join(format!("track-{name}.csv")), csv).map_err(|e| e.to_string())?;
    }
    let passed = results.iter().filter(|(a, _)| a.passed).count();
    summary += &format!(
        "{passed} of {} scenarios passed.\nThresholds are training interpretations of the Regulations, which give no numbers.\n",
        results.len()
    );
    std::fs::write(folder.join("summary.txt"), summary).map_err(|e| e.to_string())?;
    let json = serde_json::json!({
        "format": FORMAT,
        "version": 1,
        "results": results.iter().map(|(a, _)| a).collect::<Vec<_>>(),
    });
    std::fs::write(
        folder.join("colregs.json"),
        serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
