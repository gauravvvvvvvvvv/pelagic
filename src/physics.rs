use crate::math::Det;
use crate::{
    math::{Quat, Vec3},
    ocean::Ocean,
};

/// Simplified submerged rigid body: diagonal inertia, quadratic drag and buoyancy.
/// Diagonal added mass and bounded contact proxies; not a calibrated full 6×6
/// hydrodynamic model or a continuous collision solver.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vehicle {
    pub model: crate::dynamics::BodyModel,
    pub kind: u8,
    pub asset: Option<u64>,
    pub id: u64,
    actuator_force: Vec3,
    #[serde(default)]
    thruster_states: Vec<f64>,
    /// Control-surface command (roll, pitch, yaw), each −1..1: the fraction
    /// of each surface's largest deflection, signed so that a positive
    /// command turns the body positively about that axis.
    #[serde(default)]
    pub fin_command: Vec3,
    pub position: Vec3,
    pub velocity: Vec3,
    pub orientation: Quat,
    /// Body-frame angular velocity in radians/second.
    pub angular_velocity: Vec3,
    pub force: Vec3,
    pub torque: Vec3,
    mass: f64,
    volume: f64,
    inertia: Vec3,
    drag: Vec3,
    angular_drag: Vec3,
}
impl Vehicle {
    pub fn new(
        mass: f64,
        volume: f64,
        inertia: Vec3,
        drag: Vec3,
        angular_drag: Vec3,
    ) -> Result<Self, &'static str> {
        if !mass.is_finite()
            || mass <= 0.0
            || !volume.is_finite()
            || volume < 0.0
            || !inertia.is_finite()
            || inertia.x <= 0.0
            || inertia.y <= 0.0
            || inertia.z <= 0.0
            || !drag.is_finite()
            || drag.x < 0.0
            || drag.y < 0.0
            || drag.z < 0.0
            || !angular_drag.is_finite()
            || angular_drag.x < 0.0
            || angular_drag.y < 0.0
            || angular_drag.z < 0.0
        {
            return Err("invalid rigid-body parameters");
        }
        Ok(Self {
            model: Default::default(),
            kind: 0,
            asset: None,
            id: 0,
            actuator_force: Vec3::ZERO,
            thruster_states: vec![],
            fin_command: Vec3::ZERO,
            position: Vec3::new(0.0, 0.0, -10.0),
            velocity: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            angular_velocity: Vec3::ZERO,
            force: Vec3::ZERO,
            torque: Vec3::ZERO,
            mass,
            volume,
            inertia,
            drag,
            angular_drag,
        })
    }
    /// Force and torque are body-frame commands held until changed.
    pub fn mass(&self) -> f64 {
        self.mass
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        Self::new(
            self.mass,
            self.volume,
            self.inertia,
            self.drag,
            self.angular_drag,
        )?;
        self.model.validate()?;
        let q = self.orientation;
        let norm = q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z;
        if self.thruster_states.len() > 32
            || self.thruster_states.iter().any(|v| !v.is_finite())
            || !self.position.is_finite()
            || !self.velocity.is_finite()
            || !self.angular_velocity.is_finite()
            || !self.force.is_finite()
            || !self.torque.is_finite()
            || !self.actuator_force.is_finite()
            || !self.fin_command.is_finite()
            || !norm.is_finite()
            || (norm - 1.0).abs() > 1e-5
        {
            return Err("invalid body state");
        }
        Ok(())
    }
    pub fn volume(&self) -> f64 {
        self.volume
    }
    pub(crate) fn step(&mut self, ocean: &Ocean, time: f64, dt: f64) {
        let mut command = self.force;
        if let Some(depth) = self.model.hold_depth {
            let error = -depth - self.position.z;
            command = command
                + self.orientation.conjugate().rotate(Vec3::new(
                    0.0,
                    0.0,
                    self.mass * (1.8 * error - 2.6 * self.velocity.z),
                ));
        }
        let mut torque = self.torque;
        if let Some(heading) = self.model.hold_heading {
            let forward = self.orientation.rotate(Vec3::new(1.0, 0.0, 0.0));
            let error = (heading - forward.y.datan2(forward.x) + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            torque.z += 2.0 * error - 1.4 * self.angular_velocity.z;
        }
        let magnitude = command.length();
        if magnitude > self.model.thrust_limit {
            command = command * (self.model.thrust_limit / magnitude);
        }
        let blend = if self.model.response_seconds > 0.0 {
            1.0 - (-dt / self.model.response_seconds).dexp()
        } else {
            1.0
        };
        self.actuator_force = self.actuator_force + (command - self.actuator_force) * blend;
        if !self.model.thrusters.is_empty() {
            let targets = crate::actuators::allocate(&self.model.thrusters, command, torque);
            self.thruster_states.resize(targets.len(), 0.0);
            for (actual, target) in self.thruster_states.iter_mut().zip(targets) {
                *actual += (target - *actual) * blend;
            }
            let wrench = crate::actuators::wrench(&self.model.thrusters, &self.thruster_states);
            self.actuator_force = wrench.0;
            torque = wrench.1;
        }
        let relative = self
            .orientation
            .conjugate()
            .rotate(self.velocity - ocean.velocity(self.position, time));
        let density = if self.model.profile_density {
            ocean.water(self.position).density
        } else {
            ocean.density
        };
        // Bodies crossing the surface use distributed column hydrostatics
        // (wave-following heave/roll/pitch); others keep the lumped model.
        let waterline = crate::dynamics::waterline(self, ocean, time, density);
        let fraction = match &waterline {
            Some(w) => w.fraction,
            None => crate::dynamics::immersion(self, ocean, time),
        };
        let buoyancy = match &waterline {
            None => Vec3::new(
                0.0,
                0.0,
                (density * self.volume * fraction - self.mass) * ocean.gravity,
            ),
            Some(w) => {
                let heave = self.velocity.z - ocean.velocity(self.position, time).z;
                let mass = self.mass + self.model.added_mass.z;
                let damping =
                    2.0 * crate::dynamics::WATERLINE_DAMPING * (w.heave_stiffness * mass).sqrt();
                Vec3::new(
                    0.0,
                    0.0,
                    w.force - self.mass * ocean.gravity - damping * heave,
                )
            }
        };
        let effective = self.model.added_mass + Vec3::new(self.mass, self.mass, self.mass);
        let (fin_force, fin_moment) = if self.model.fins.is_empty() {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            crate::fins::wrench(
                &self.model.fins,
                relative,
                self.angular_velocity,
                density,
                self.fin_command,
                effective,
                self.inertia + self.model.added_inertia,
                dt,
            )
        };
        // Added-mass Coriolis force. Velocity is integrated in the world
        // frame with per-axis effective mass; this restores the body-frame
        // equation (M+A)·v̇ + ω×((M+A)·v) = F, so the Munk moment below
        // exchanges energy instead of creating it (X = (A_y−A_x)·v·r,
        // Y = (A_y−A_x)·u·r for a manoeuvring vessel). Surface-piercing
        // hulls keep only the yaw-rate terms, like the Munk moment.
        // A hull partly or wholly out of the water (outside the waterline
        // model) carries added mass only in proportion to its immersion; in
        // air it has none.
        let surface = waterline.is_some() || fraction < 1.0;
        let wet = if waterline.is_some() { 1.0 } else { fraction };
        let a = self.model.added_mass;
        let spin = if surface {
            Vec3::new(0.0, 0.0, self.angular_velocity.z)
        } else {
            self.angular_velocity
        };
        let coriolis =
            (a.component_mul(spin.cross(relative)) - spin.cross(a.component_mul(relative))) * wet;
        let force = self.actuator_force
            + fin_force
            + coriolis
            + self.orientation.conjugate().rotate(buoyancy);
        let free = relative
            + Vec3::new(
                force.x / effective.x,
                force.y / effective.y,
                force.z / effective.z,
            ) * dt;
        let drag = self.drag.component_mul(free.abs()) * dt;
        let damped = Vec3::new(
            free.x / (1.0 + drag.x / effective.x),
            free.y / (1.0 + drag.y / effective.y),
            free.z / (1.0 + drag.z / effective.z),
        );
        self.velocity = ocean.velocity(self.position, time) + self.orientation.rotate(damped);
        self.position = self.position + self.velocity * dt;
        let w = self.angular_velocity;
        // Rigid-body plus added (fluid) moments of inertia.
        let rot = self.inertia + self.model.added_inertia;
        let restoring = match &waterline {
            None => self
                .model
                .center_buoyancy
                .cross(self.orientation.conjugate().rotate(Vec3::new(
                    0.0,
                    0.0,
                    density * self.volume * fraction * ocean.gravity,
                ))),
            Some(line) => {
                let zeta = 2.0 * crate::dynamics::WATERLINE_DAMPING;
                line.torque
                    - Vec3::new(
                        zeta * (line.roll_stiffness * rot.x).sqrt() * w.x,
                        zeta * (line.pitch_stiffness * rot.y).sqrt() * w.y,
                        0.0,
                    )
            }
        };
        // Munk moment of the added mass. A surface-piercing hull's heave and
        // pitch added masses are free-surface (frequency-dependent) values;
        // the infinite-fluid moment would pitch a vessel over at speed, so a
        // body in the waterline keeps only the yaw (manoeuvring) component.
        let munk = relative.cross(self.model.added_mass.component_mul(relative)) * wet;
        let munk = if surface {
            Vec3::new(0.0, 0.0, munk.z)
        } else {
            munk
        };
        let moment = torque + restoring + fin_moment
            - munk
            - self.angular_drag.component_mul(w.component_mul(w.abs()))
            - w.cross(rot.component_mul(w));
        self.angular_velocity =
            w + Vec3::new(moment.x / rot.x, moment.y / rot.y, moment.z / rot.z) * dt;
        self.orientation = (self.orientation
            * Quat::from_rotation_vector(self.angular_velocity * dt))
        .normalized();
        crate::dynamics::contact(self, ocean);
    }
}
