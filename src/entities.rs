use crate::{dynamics::BodyModel, math::Vec3};
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entity {
    pub id: u64,
    pub name: String,
    pub kind: u8,
    pub position: Vec3,
    pub heading: f64,
    pub mass: f64,
    pub volume: f64,
    pub model: BodyModel,
    pub force: Vec3,
    /// Library mesh handle (`Library::meshes[].id`); `None` draws the procedural body.
    #[serde(default)]
    pub asset: Option<u64>,
    /// Library material handle (`Library::materials[].id`); `None` uses the default look.
    #[serde(default)]
    pub material: Option<u64>,
}
impl Entity {
    pub fn rov(id: u64, position: Vec3) -> Self {
        Self {
            id,
            name: format!("ROV {id}"),
            kind: 0,
            position,
            heading: 0.0,
            mass: 12.0,
            volume: 12.0 / 1025.0,
            model: BodyModel::rov(),
            force: Vec3::ZERO,
            asset: None,
            material: None,
        }
    }
    pub fn boat(id: u64) -> Self {
        let mut e = Self::rov(id, Vec3::new(10.0, 0.0, 0.0));
        e.name = format!("Surface vessel {id}");
        e.kind = 1;
        e.mass = 200.0;
        e.volume = 0.32;
        e.model.half = Vec3::new(1.8, 0.7, 0.45);
        e.model.center_buoyancy = Vec3::new(0.0, 0.0, 0.3);
        e.model.added_mass = Vec3::new(30.0, 100.0, 200.0);
        e.model.thrust_limit = 300.0;
        e.model.thrusters.clear();
        e
    }
    pub fn drifter(id: u64) -> Self {
        let mut e = Self::rov(id, Vec3::new(4.0, 0.0, 0.0));
        e.name = format!("Drifter {id}");
        e.kind = 2;
        e.mass = 2.0;
        e.volume = 0.003;
        e.model.half = Vec3::new(0.12, 0.12, 0.25);
        e.model.added_mass = Vec3::new(0.5, 0.5, 0.5);
        e.model.thrust_limit = 0.0;
        e.model.thrusters.clear();
        e
    }
    pub fn validate(&self) -> Result<(), String> {
        self.model.validate().map_err(str::to_string)?;
        if self.id == 0
            || self.name.is_empty()
            || self.name.len() > 80
            || self.kind > 2
            || !self.position.is_finite()
            || self.position.length() > 100000.0
            || !self.heading.is_finite()
            || !self.mass.is_finite()
            || !(0.01..=100000.0).contains(&self.mass)
            || !self.volume.is_finite()
            || !(0.0..=1000.0).contains(&self.volume)
            || !self.force.is_finite()
            || self.force.length() > 1e6
        {
            return Err("Invalid entity identity, transform or mass/volume/force.".into());
        }
        if self.asset == Some(0) || self.material == Some(0) {
            return Err("Asset and material handles must be positive.".into());
        }
        Ok(())
    }
    /// Moments of inertia used by the simulation: the model's override, else
    /// a solid box of this size and mass.
    pub fn effective_inertia(&self) -> Vec3 {
        self.model.inertia.unwrap_or_else(|| {
            let h = self.model.half;
            Vec3::new(
                (h.y * h.y + h.z * h.z) * self.mass / 3.0,
                (h.x * h.x + h.z * h.z) * self.mass / 3.0,
                (h.x * h.x + h.y * h.y) * self.mass / 3.0,
            )
        })
    }
    /// Quadratic drag used by the simulation (override or mass-scaled default).
    pub fn effective_drag(&self) -> Vec3 {
        self.model
            .drag
            .unwrap_or_else(|| Vec3::new(8.0, 12.0, 15.0) * (self.mass / 12.0))
    }
    /// Quadratic rotational drag used by the simulation.
    pub fn effective_angular_drag(&self) -> Vec3 {
        self.model
            .angular_drag
            .unwrap_or_else(|| Vec3::new(0.4, 0.4, 0.4) * (self.mass / 12.0))
    }
    pub fn spawn(&self) -> crate::physics::Vehicle {
        let mut v = crate::physics::Vehicle::new(
            self.mass,
            self.volume,
            self.effective_inertia(),
            self.effective_drag(),
            self.effective_angular_drag(),
        )
        .expect("validated entity");
        v.position = self.position;
        v.orientation =
            crate::math::Quat::from_rotation_vector(Vec3::new(0.0, 0.0, self.heading.to_radians()));
        v.model = self.model.clone();
        v.force = self.force;
        v.id = self.id;
        v.kind = self.kind;
        v.asset = self.asset.map(|_| self.id);
        v
    }
}
