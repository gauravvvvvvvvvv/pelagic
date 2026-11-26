//! Straight-ray geometry sonar and seeded navigation observations. Not multipath acoustics.
use crate::math::Det;
use crate::{Snapshot, math::Vec3, ocean::Ocean};
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SensorConfig {
    pub entity: u64,
    pub rate_hz: f64,
    pub range: f64,
    pub beams: usize,
    pub fan_degrees: f64,
    pub range_noise: f64,
    pub position_noise: f64,
    pub velocity_sigma: f64,
    pub gyro_sigma: f64,
    pub acceleration_sigma: f64,
    pub mount: Vec3,
    pub tilt_degrees: f64,
    pub latency_seconds: f64,
    pub camera_fov: f64,
}
impl Default for SensorConfig {
    fn default() -> Self {
        Self {
            entity: 0,
            rate_hz: 10.0,
            range: 80.0,
            beams: 48,
            fan_degrees: 90.0,
            range_noise: 0.02,
            position_noise: 0.03,
            velocity_sigma: 0.01,
            gyro_sigma: 0.001,
            acceleration_sigma: 0.01,
            mount: Vec3::new(0.8, 0.0, -0.2),
            tilt_degrees: 60.0,
            latency_seconds: 0.02,
            camera_fov: 60.0,
        }
    }
}
impl SensorConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if ![
            self.rate_hz,
            self.range,
            self.fan_degrees,
            self.range_noise,
            self.position_noise,
            self.velocity_sigma,
            self.gyro_sigma,
            self.acceleration_sigma,
            self.tilt_degrees,
            self.latency_seconds,
            self.camera_fov,
        ]
        .iter()
        .all(|v| v.is_finite())
            || !(0.5..=30.0).contains(&self.rate_hz)
            || !(1.0..=500.0).contains(&self.range)
            || !(1..=128).contains(&self.beams)
            || !(1.0..=150.0).contains(&self.fan_degrees)
            || !(0.0..=5.0).contains(&self.range_noise)
            || !(0.0..=100.0).contains(&self.position_noise)
            || !(0.0..=10.0).contains(&self.velocity_sigma)
            || !(0.0..=1.0).contains(&self.gyro_sigma)
            || !(0.0..=10.0).contains(&self.acceleration_sigma)
            || !self.mount.is_finite()
            || self.mount.length() > 20.0
            || !(0.0..=90.0).contains(&self.tilt_degrees)
            || !(0.0..=10.0).contains(&self.latency_seconds)
            || !(20.0..=120.0).contains(&self.camera_fov)
        {
            return Err("invalid sensor rate, bounds, mount or noise");
        }
        Ok(())
    }
    pub fn ticks(&self, dt: f64) -> u64 {
        (1.0 / (self.rate_hz * dt)).round().max(1.0) as u64
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Beam {
    pub angle: f64,
    pub truth_range: Option<f64>,
    pub range: Option<f64>,
    pub travel_seconds: Option<f64>,
    pub intensity: f64,
    pub target: u64,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Observation {
    pub angular_velocity: Vec3,
    pub specific_force: Vec3,
    pub time: f64,
    pub available_time: f64,
    pub vehicle: u64,
    pub position: Vec3,
    pub depth: f64,
    pub velocity: Vec3,
    pub orientation: crate::math::Quat,
    pub beams: Vec<Beam>,
}
impl Observation {
    pub fn valid(&self) -> bool {
        let q = self.orientation;
        let norm = q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z;
        self.time.is_finite()
            && self.available_time.is_finite()
            && self.available_time >= self.time
            && self.position.is_finite()
            && self.velocity.is_finite()
            && self.angular_velocity.is_finite()
            && self.specific_force.is_finite()
            && self.depth.is_finite()
            && norm.is_finite()
            && (norm - 1.0).abs() < 1e-5
            && self.beams.len() <= 128
            && self.beams.iter().all(|b| {
                b.angle.is_finite()
                    && b.intensity.is_finite()
                    && [b.range, b.truth_range, b.travel_seconds]
                        .iter()
                        .all(|r| r.is_none_or(|v| v.is_finite() && v >= 0.0))
            })
    }
}
fn gaussian(seed: &mut u64) -> f64 {
    let u = crate::life::random(seed).max(1e-12);
    let v = crate::life::random(seed);
    (-2.0 * u.dln()).sqrt() * (std::f64::consts::TAU * v).dcos()
}
pub fn sample(
    o: &Ocean,
    s: &Snapshot,
    velocities: &[Vec3],
    own: usize,
    c: &SensorConfig,
) -> Option<Observation> {
    c.validate().ok()?;
    let v = s.vehicles.get(own)?;
    let origin = v.position + v.orientation.rotate(c.mount);
    let mut seed = s.seed ^ s.tick.wrapping_mul(0x517cc1b727220a95) ^ v.id;
    let noise = Vec3::new(
        gaussian(&mut seed),
        gaussian(&mut seed),
        gaussian(&mut seed),
    ) * c.position_noise;
    let mut beams = vec![];
    let tilt = c.tilt_degrees.to_radians();
    for i in 0..c.beams {
        let angle = if c.beams == 1 {
            0.0
        } else {
            (i as f64 / (c.beams - 1) as f64 - 0.5) * c.fan_degrees.to_radians()
        };
        let d = v
            .orientation
            .rotate(Vec3::new(
                tilt.dcos() * angle.dcos(),
                angle.dsin(),
                -tilt.dsin() * angle.dcos(),
            ))
            .normalized();
        let mut range = crate::sensor::bottom_echo(o, origin, d, c.range, 0.5).map(|e| e.range);
        let mut target = 0;
        for (j, b) in o.structures.iter().enumerate() {
            if let Some(r) = b
                .ray_distance(origin, d)
                .filter(|r| *r > 0.01 && *r <= c.range && range.is_none_or(|best| *r < best))
            {
                range = Some(r);
                target = 1_000_000 + j as u64;
            }
        }
        for (j, p) in s.vehicles.iter().enumerate() {
            if j == own {
                continue;
            }
            let ray = crate::geometry::StaticBox {
                center: Vec3::ZERO,
                half: p.half,
                color: [0.0; 3],
            };
            let local = p.orientation.conjugate();
            if let Some(r) = ray
                .ray_distance(local.rotate(origin - p.position), local.rotate(d))
                .filter(|r| *r > 0.01 && *r <= c.range && range.is_none_or(|best| *r < best))
            {
                range = Some(r);
                target = p.id + 1;
            }
        }
        let travel = range.map(|r| {
            let mut sum = 0.0;
            for j in 0..8 {
                sum += r
                    / 8.0
                    / o.water(origin + d * (r * (j as f64 + 0.5) / 8.0))
                        .sound_speed
                        .max(100.0);
            }
            2.0 * sum
        });
        let measured = range.map(|r| (r + gaussian(&mut seed) * c.range_noise).clamp(0.0, c.range));
        let reflectivity = if target >= 1_000_000 {
            0.8
        } else if target > 0 {
            0.9
        } else {
            0.35
        };
        let incidence = if target == 0 {
            range.map_or(1.0, |r| {
                let p = origin + d * r;
                let e = 0.5;
                Vec3::new(
                    o.bottom.elevation(p.x - e, p.y) - o.bottom.elevation(p.x + e, p.y),
                    o.bottom.elevation(p.x, p.y - e) - o.bottom.elevation(p.x, p.y + e),
                    2.0 * e,
                )
                .normalized()
                .dot(-d)
                .abs()
                .max(0.05)
            })
        } else {
            1.0
        };
        beams.push(Beam {
            angle,
            truth_range: range,
            range: measured,
            travel_seconds: travel,
            intensity: range.map_or(0.0, |r| {
                reflectivity * incidence * (-0.002 * r).dexp() / (1.0 + r * r)
            }),
            target,
        });
    }
    Some(Observation {
        angular_velocity: v.angular_velocity
            + Vec3::new(
                gaussian(&mut seed),
                gaussian(&mut seed),
                gaussian(&mut seed),
            ) * c.gyro_sigma,
        specific_force: v
            .orientation
            .conjugate()
            .rotate(Vec3::new(0.0, 0.0, o.gravity)),
        time: s.time,
        available_time: s.time + c.latency_seconds,
        vehicle: v.id,
        position: v.position + noise,
        depth: -v.position.z + gaussian(&mut seed) * c.position_noise,
        velocity: velocities.get(own).copied().unwrap_or(Vec3::ZERO)
            + Vec3::new(
                gaussian(&mut seed),
                gaussian(&mut seed),
                gaussian(&mut seed),
            ) * c.velocity_sigma,
        orientation: v.orientation,
        beams,
    })
}
/// Fixed-tick sampling and simulated delivery latency. Consumers see delivered
/// observations; timestamps distinguish physical truth from delayed measurements.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Suite {
    previous: Option<(f64, Vec3)>,
    pub config: SensorConfig,
    pub latest: Vec<Observation>,
    pending: std::collections::VecDeque<Observation>,
}
impl Suite {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        if self.latest.len() > 1
            || self.pending.len() > 302
            || self
                .previous
                .is_some_and(|(t, v)| !t.is_finite() || !v.is_finite())
            || self
                .pending
                .iter()
                .chain(self.latest.iter())
                .any(|o| !o.valid())
        {
            return Err("invalid sensor checkpoint");
        }
        Ok(())
    }
    pub fn new(config: SensorConfig) -> Self {
        Self {
            previous: None,
            config,
            latest: vec![],
            pending: Default::default(),
        }
    }
    pub fn update(&mut self, world: &crate::World, dt: f64) {
        if world.tick().is_multiple_of(self.config.ticks(dt)) {
            let snapshot = world.snapshot();
            let velocities = world
                .vehicles()
                .iter()
                .map(|v| v.velocity)
                .collect::<Vec<_>>();
            if let Some(index) = snapshot
                .vehicles
                .iter()
                .position(|p| p.id == self.config.entity)
                .or_else(|| (!snapshot.vehicles.is_empty()).then_some(0))
                && let Some(mut obs) =
                    sample(&world.ocean, &snapshot, &velocities, index, &self.config)
            {
                let velocity = velocities[index];
                let acceleration = self.previous.map_or(Vec3::ZERO, |(time, old)| {
                    if snapshot.time > time {
                        (velocity - old) / (snapshot.time - time)
                    } else {
                        Vec3::ZERO
                    }
                });
                self.previous = Some((snapshot.time, velocity));
                let mut seed = snapshot.seed ^ snapshot.tick ^ 0xabcdef;
                let noise = Vec3::new(
                    gaussian(&mut seed),
                    gaussian(&mut seed),
                    gaussian(&mut seed),
                ) * self.config.acceleration_sigma;
                obs.specific_force = snapshot.vehicles[index]
                    .orientation
                    .conjugate()
                    .rotate(acceleration + Vec3::new(0.0, 0.0, world.ocean.gravity))
                    + noise;
                self.pending.push_back(obs);
            }
        }
        while self
            .pending
            .front()
            .is_some_and(|o| o.available_time <= world.time() + 1e-12)
        {
            self.latest = self.pending.pop_front().into_iter().collect();
        }
    }
}
