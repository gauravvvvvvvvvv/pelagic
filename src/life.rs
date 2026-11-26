//! Deterministic illustrative schooling and passive advection, not population ecology.
use crate::math::Det;
use crate::{math::Vec3, ocean::Ocean};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SchoolConfig {
    pub count: usize,
    pub center: Vec3,
    pub radius: f64,
    pub speed: f64,
    pub separation: f64,
    pub avoidance: f64,
    pub size: f64,
}
impl Default for SchoolConfig {
    fn default() -> Self {
        Self {
            count: 48,
            center: Vec3::new(5.0, 2.0, -18.0),
            radius: 12.0,
            speed: 1.2,
            separation: 1.2,
            avoidance: 5.0,
            size: 0.45,
        }
    }
}
impl SchoolConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.count > 256
            || !self.center.is_finite()
            || self.center.length() > 20000.0
            || !self.radius.is_finite()
            || !(1.0..=100.0).contains(&self.radius)
            || !self.speed.is_finite()
            || !(0.1..=5.0).contains(&self.speed)
            || !self.separation.is_finite()
            || !(0.2..=10.0).contains(&self.separation)
            || !self.avoidance.is_finite()
            || !(0.0..=20.0).contains(&self.avoidance)
            || !self.size.is_finite()
            || !(0.1..=2.0).contains(&self.size)
        {
            return Err("invalid school count, bounds or behaviour parameters");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Fish {
    pub position: Vec3,
    pub velocity: Vec3,
    pub phase: f64,
    pub size: f64,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Particle {
    pub position: Vec3,
    pub age: f64,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct School {
    pub config: SchoolConfig,
    pub fish: Vec<Fish>,
}
pub fn random(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((z ^ (z >> 31)) >> 11) as f64 / ((1u64 << 53) as f64)
}
fn limit(v: Vec3, max: f64) -> Vec3 {
    let length = v.length();
    if length > max { v * (max / length) } else { v }
}
impl School {
    pub fn new(config: SchoolConfig, seed: u64) -> Result<Self, &'static str> {
        config.validate()?;
        let mut state = seed;
        let mut fish = vec![];
        for _ in 0..config.count {
            let position = config.center
                + Vec3::new(
                    random(&mut state) - 0.5,
                    random(&mut state) - 0.5,
                    (random(&mut state) - 0.5) * 0.35,
                ) * config.radius;
            let heading = (random(&mut state) - 0.5) * 0.8;
            fish.push(Fish {
                position,
                velocity: Vec3::new(heading.dcos(), heading.dsin(), 0.0) * config.speed,
                phase: random(&mut state) * std::f64::consts::TAU,
                size: config.size * (0.8 + random(&mut state) * 0.4),
            });
        }
        Ok(Self { config, fish })
    }
    pub fn step(&mut self, ocean: &Ocean, vehicles: &[Vec3], t: f64, dt: f64) {
        let cell = self.config.separation.max(4.0);
        let key = |p: Vec3| {
            (
                (p.x / cell).floor() as i32,
                (p.y / cell).floor() as i32,
                (p.z / cell).floor() as i32,
            )
        };
        let mut bins: BTreeMap<(i32, i32, i32), Vec<usize>> = BTreeMap::new();
        for (i, f) in self.fish.iter().enumerate() {
            bins.entry(key(f.position)).or_default().push(i);
        }
        let old = self.fish.clone();
        for (i, f) in self.fish.iter_mut().enumerate() {
            let p = old[i].position;
            let (x, y, z) = key(p);
            let mut separate = Vec3::ZERO;
            let mut alignment = Vec3::ZERO;
            let mut cohesion = Vec3::ZERO;
            let mut count = 0.0;
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        if let Some(neighbors) = bins.get(&(x + dx, y + dy, z + dz)) {
                            for &j in neighbors {
                                if i == j {
                                    continue;
                                }
                                let delta = p - old[j].position;
                                let d = delta.length();
                                if d < cell && d > 1e-8 {
                                    alignment = alignment + old[j].velocity;
                                    cohesion = cohesion + old[j].position;
                                    count += 1.0;
                                    if d < self.config.separation {
                                        separate = separate
                                            + delta * ((self.config.separation - d) / (d * d));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let mut acceleration = separate * 2.5;
            if count > 0.0 {
                acceleration = acceleration
                    + (alignment / count - old[i].velocity) * 0.7
                    + (cohesion / count - p) * 0.18;
            }
            let home = self.config.center - p;
            let d = home.length();
            if d > self.config.radius * 0.6 {
                acceleration =
                    acceleration + home.normalized() * ((d / self.config.radius).min(3.0));
            }
            let desired_z = self
                .config
                .center
                .z
                .max(ocean.bottom.elevation(p.x, p.y) + 2.0)
                .min(ocean.surface(p.x, p.y, t).height - 1.0);
            acceleration.z += (desired_z - p.z) * 0.8 - old[i].velocity.z * 0.5;
            for &v in vehicles {
                let delta = p - v;
                let d = delta.length();
                if d < self.config.avoidance && d > 1e-6 {
                    acceleration =
                        acceleration + delta.normalized() * ((self.config.avoidance - d) * 2.0);
                }
            }
            for b in &ocean.structures {
                let closest = Vec3::new(
                    p.x.clamp(b.center.x - b.half.x, b.center.x + b.half.x),
                    p.y.clamp(b.center.y - b.half.y, b.center.y + b.half.y),
                    p.z.clamp(b.center.z - b.half.z, b.center.z + b.half.z),
                );
                let delta = p - closest;
                let d = delta.length();
                if d < 2.0 {
                    acceleration = acceleration
                        + if d > 1e-6 {
                            delta.normalized() * (2.0 - d) * 3.0
                        } else {
                            Vec3::new(0.0, 0.0, 6.0)
                        };
                }
            }
            let swimming = limit(
                old[i].velocity + limit(acceleration, 3.0) * dt,
                self.config.speed * 1.5,
            );
            let swimming = if swimming.length() < self.config.speed * 0.5 {
                swimming.normalized() * self.config.speed * 0.5
            } else {
                swimming
            };
            f.velocity = swimming;
            f.position = p + (swimming + limit(ocean.velocity(p, t), 3.0)) * dt;
            // Steering is anticipatory, not a penetration constraint. Project
            // an overlapping fish centre to the nearest padded obstacle face.
            // This is deterministic and also handles spawning inside a box.
            for obstacle in &ocean.structures {
                let half = obstacle.half + Vec3::new(f.size * 0.3, f.size * 0.3, f.size * 0.3);
                let delta = f.position - obstacle.center;
                if delta.x.abs() < half.x && delta.y.abs() < half.y && delta.z.abs() < half.z {
                    let distances = [
                        half.x - delta.x.abs(),
                        half.y - delta.y.abs(),
                        half.z - delta.z.abs(),
                    ];
                    let axis = (0..3)
                        .min_by(|a, b| distances[*a].total_cmp(&distances[*b]))
                        .unwrap();
                    match axis {
                        0 => {
                            f.position.x =
                                obstacle.center.x + half.x * if delta.x < 0.0 { -1.0 } else { 1.0 };
                            f.velocity.x = 0.0;
                        }
                        1 => {
                            f.position.y =
                                obstacle.center.y + half.y * if delta.y < 0.0 { -1.0 } else { 1.0 };
                            f.velocity.y = 0.0;
                        }
                        _ => {
                            f.position.z =
                                obstacle.center.z + half.z * if delta.z < 0.0 { -1.0 } else { 1.0 };
                            f.velocity.z = 0.0;
                        }
                    }
                }
            }
            let bottom = ocean.bottom.elevation(f.position.x, f.position.y) + 0.6;
            let surface = ocean.surface(f.position.x, f.position.y, t).height - 0.6;
            if bottom < surface {
                f.position.z = f.position.z.clamp(bottom, surface);
            } else {
                f.position = self.config.center;
            }
            f.phase =
                (f.phase + dt * (7.0 + swimming.length() * 3.0)).rem_euclid(std::f64::consts::TAU);
        }
    }
}
pub fn advect(particles: &mut [Particle], o: &Ocean, t: f64, dt: f64) {
    for p in particles {
        let a = o.velocity(p.position, t);
        let b = o.velocity(p.position + a * (dt * 0.5), t + dt * 0.5);
        p.position = p.position + b * dt;
        p.age += dt;
        let low = o.bottom.elevation(p.position.x, p.position.y) + 0.1;
        let high = o.surface(p.position.x, p.position.y, t).height - 0.1;
        if low < high {
            p.position.z = p.position.z.clamp(low, high);
        }
    }
}
