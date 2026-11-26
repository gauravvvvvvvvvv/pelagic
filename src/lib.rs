//! Native ocean simulation primitives. SI units; right-handed world, Z up.
//! Models are interactive approximations, not validated engineering predictions.
pub mod actuators;
pub mod assets;
pub mod bridge;
pub mod campaign;
pub mod colregs;
pub mod data;
pub mod dataset;
pub mod definition;
pub mod dynamics;
pub mod entities;
pub mod experiment;
pub mod fauna;
pub mod ffi;
pub mod fins;
pub mod geometry;
pub mod habitat;
pub(crate) mod helm;
pub mod inspection;
pub mod instruments;
pub mod landscape;
pub mod life;
pub mod math;
pub mod metocean;
pub mod ocean;
pub mod physics;
pub mod render;
pub mod render_scene;
pub mod sar;
pub mod scene;
pub mod seakeeping;
pub mod sensor;
pub mod spill;
pub mod survey;
pub mod terrain;
pub mod thumbnail;
pub mod trials;
pub mod waves;

use math::Vec3;
use ocean::Ocean;
use physics::Vehicle;
pub const BUILD_ID: &str = env!("PELAGIC_BUILD_ID");

/// Fixed-step world. Rendering and application code never own simulation time.
pub struct World {
    pub ocean: Ocean,
    vehicles: Vec<Vehicle>,
    tick: u64,
    dt: f64,
    seed: u64,
    pub schools: Vec<life::School>,
    pub particles: Vec<life::Particle>,
}

impl World {
    pub fn new(ocean: Ocean, dt: f64, seed: u64) -> Result<Self, &'static str> {
        if !dt.is_finite() || dt <= 0.0 || dt > 1.0 {
            return Err("time step must be finite and positive");
        }
        Ok(Self {
            ocean,
            vehicles: Vec::new(),
            tick: 0,
            dt,
            seed,
            schools: vec![],
            particles: vec![],
        })
    }
    /// Stable index for this world's lifetime. Removal is deliberately not exposed yet.
    pub fn add_vehicle(&mut self, vehicle: Vehicle) -> usize {
        let id = self.vehicles.len();
        self.vehicles.push(vehicle);
        id
    }
    pub fn vehicle_mut(&mut self, id: usize) -> Option<&mut Vehicle> {
        self.vehicles.get_mut(id)
    }
    pub fn vehicles(&self) -> &[Vehicle] {
        &self.vehicles
    }
    pub fn time(&self) -> f64 {
        self.tick as f64 * self.dt
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn seed(&self) -> u64 {
        self.seed
    }
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            tick: self.tick,
            dt: self.dt,
            seed: self.seed,
            vehicles: self.vehicles.clone(),
            schools: self.schools.clone(),
            particles: self.particles.clone(),
        }
    }
    pub fn restore(&mut self, state: &Checkpoint) -> Result<(), &'static str> {
        state.validate()?;
        self.tick = state.tick;
        self.dt = state.dt;
        self.seed = state.seed;
        self.vehicles = state.vehicles.clone();
        self.schools = state.schools.clone();
        self.particles = state.particles.clone();
        Ok(())
    }
    pub fn step(&mut self) {
        let time = self.time();
        let steps = if self.vehicles.iter().any(|v| v.model.contacts) {
            (self.dt * 120.0).ceil().clamp(1.0, 256.0) as usize
        } else {
            1
        };
        for sub in 0..steps {
            for vehicle in &mut self.vehicles {
                vehicle.step(
                    &self.ocean,
                    time + sub as f64 * self.dt / steps as f64,
                    self.dt / steps as f64,
                );
            }
            dynamics::pairs(&mut self.vehicles);
        }
        let positions: Vec<_> = self.vehicles.iter().map(|v| v.position).collect();
        for school in &mut self.schools {
            school.step(&self.ocean, &positions, time, self.dt);
        }
        life::advect(&mut self.particles, &self.ocean, time, self.dt);
        self.tick += 1;
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick,
            time: self.time(),
            seed: self.seed,
            vehicles: self
                .vehicles
                .iter()
                .map(|v| Pose {
                    position: v.position,
                    orientation: v.orientation,
                    half: v.model.half,
                    kind: v.kind,
                    asset: v.asset,
                    id: v.id,
                    velocity: v.velocity,
                    angular_velocity: v.angular_velocity,
                })
                .collect(),
            fish: self
                .schools
                .iter()
                .flat_map(|s| s.fish.iter().cloned())
                .collect(),
            particles: self.particles.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Pose {
    #[serde(default)]
    pub angular_velocity: Vec3,
    #[serde(default)]
    pub velocity: Vec3,
    pub position: Vec3,
    pub orientation: math::Quat,
    pub half: Vec3,
    pub kind: u8,
    pub asset: Option<u64>,
    pub id: u64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub time: f64,
    pub seed: u64,
    pub vehicles: Vec<Pose>,
    pub fish: Vec<life::Fish>,
    pub particles: Vec<life::Particle>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub tick: u64,
    pub dt: f64,
    pub seed: u64,
    pub vehicles: Vec<Vehicle>,
    pub schools: Vec<life::School>,
    pub particles: Vec<life::Particle>,
}
impl Checkpoint {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.dt.is_finite()
            || self.dt <= 0.0
            || self.dt > 1.0
            || self.vehicles.len() > 32
            || self.schools.len() > 8
            || self.schools.iter().map(|s| s.fish.len()).sum::<usize>() > 256
            || self.particles.len() > 512
        {
            return Err("checkpoint exceeds model budget");
        }
        for v in &self.vehicles {
            v.validate()?;
        }
        for s in &self.schools {
            s.config.validate()?;
            if s.fish.len() != s.config.count
                || s.fish.iter().any(|f| {
                    !f.position.is_finite()
                        || !f.velocity.is_finite()
                        || !f.phase.is_finite()
                        || !f.size.is_finite()
                })
            {
                return Err("invalid checkpoint school");
            }
        }
        if self
            .particles
            .iter()
            .any(|p| !p.position.is_finite() || !p.age.is_finite())
        {
            return Err("invalid checkpoint tracers");
        }
        Ok(())
    }
}
pub mod fields;
