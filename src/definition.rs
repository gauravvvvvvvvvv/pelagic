//! Vehicle definitions: a shareable JSON description of a vehicle prototype
//! (`*.pelagic-vehicle.json`) that can be imported into any scene, edited,
//! tested and exported again.
//!
//! A definition carries identity, mass properties, hull size, hydrodynamic
//! coefficients (drag, added mass, inertia, centre of buoyancy), thrusters,
//! actuator limits and controller settings, plus an optional hull mesh (an
//! OBJ file next to the definition). Units are SI; the body frame is X
//! forward, Y left, Z up, with the origin at the centre of mass.
use crate::{dynamics::BodyModel, entities::Entity, math::Vec3};
use serde::{Deserialize, Serialize};

pub const FORMAT: &str = "pelagic-vehicle";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VehicleDefinition {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// 0 underwater vehicle, 1 surface vessel, 2 drifter / passive body.
    pub kind: u8,
    /// Total mass, kg.
    pub mass: f64,
    /// Displaced volume when fully submerged, m³.
    pub volume: f64,
    /// Hull size, hydrodynamics, thrusters and controllers.
    pub model: BodyModel,
    /// Hull mesh (Wavefront OBJ) relative to the definition file.
    #[serde(default)]
    pub mesh_obj: Option<String>,
    /// Free text: origin, revision, data sources, assumptions.
    #[serde(default)]
    pub notes: String,
}

impl VehicleDefinition {
    /// Describes an existing entity. Inertia and drag are written out
    /// explicitly (the values the simulation actually uses), so the file is
    /// a complete, editable record.
    pub fn from_entity(e: &Entity) -> Self {
        let mut model = e.model.clone();
        model.inertia = Some(e.effective_inertia());
        model.drag = Some(e.effective_drag());
        model.angular_drag = Some(e.effective_angular_drag());
        Self {
            format: FORMAT.into(),
            version: VERSION,
            name: e.name.clone(),
            kind: e.kind,
            mass: e.mass,
            volume: e.volume,
            model,
            mesh_obj: None,
            notes: String::new(),
        }
    }

    /// An entity with this definition's physics at `position`, facing
    /// `heading` degrees. Library mesh/material handles are left empty.
    pub fn to_entity(&self, id: u64, position: Vec3, heading: f64) -> Entity {
        Entity {
            id,
            name: self.name.clone(),
            kind: self.kind,
            position,
            heading,
            mass: self.mass,
            volume: self.volume,
            model: self.model.clone(),
            force: Vec3::ZERO,
            asset: None,
            material: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.format != FORMAT {
            return Err(format!(
                "Not a vehicle definition (format '{}').",
                self.format
            ));
        }
        if self.version == 0 || self.version > VERSION {
            return Err(format!(
                "Vehicle definition version {} is newer than this build supports ({VERSION}).",
                self.version
            ));
        }
        if self.notes.len() > 20_000 {
            return Err("Notes are limited to 20,000 characters.".into());
        }
        if self
            .mesh_obj
            .as_deref()
            .is_some_and(|p| p.is_empty() || p.len() > 1024 || p.contains('\0'))
        {
            return Err("Invalid hull mesh path.".into());
        }
        self.to_entity(1, Vec3::new(0.0, 0.0, -5.0), 0.0).validate()
    }

    /// Parses and validates a definition from JSON text.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > 4 * 1024 * 1024 {
            return Err("Vehicle definition is larger than 4 MB.".into());
        }
        let definition: Self =
            serde_json::from_str(text).map_err(|e| format!("Vehicle definition: {e}"))?;
        definition.validate()?;
        Ok(definition)
    }

    /// Pretty JSON for sharing and version control.
    pub fn to_json(&self) -> Result<String, String> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Weight minus buoyancy when fully submerged in `density` water, N
    /// (positive sinks).
    pub fn net_weight(&self, density: f64, gravity: f64) -> f64 {
        (self.mass - density * self.volume) * gravity
    }
}

/// First-order hydrodynamic coefficients for a box-shaped hull with half
/// extents `half` (m), displaced volume `volume` (m³), mass `mass` (kg), in
/// water of `density` (kg/m³). Standard engineering estimates:
/// - inertia: solid box;
/// - quadratic drag per axis: 0.5·ρ·Cd·A over the projected area, with Cd
///   1.0 for blunt faces and 0.3 along X when `streamlined`;
/// - rotational drag: strip integration of the same cross-flow drag,
///   K_z = 0.5·ρ·Cd·c·(a⁴ + b⁴) and cyclic for X and Y;
/// - added mass: 2-D strip theory, A_y = ρ·π·c²·2a, A_z = ρ·π·b²·2a, and
///   10 % of the displaced mass along X (20 % when blunt);
/// - added inertia: the same strips about the centre, N_ṙ = A_y·a²/3,
///   M_q̇ = A_z·a²/3, and K_ṗ = 2a·ρ·π/8·(b² − c²)² (zero for a square
///   section).
///
/// These give physically scaled starting values; measured or CFD
/// coefficients should replace them when available.
pub fn estimate_hydrodynamics(
    half: Vec3,
    mass: f64,
    volume: f64,
    density: f64,
    streamlined: bool,
) -> Hydrodynamics {
    let (a, b, c) = (half.x, half.y, half.z);
    let q = 0.5 * density;
    let axial = if streamlined { 0.3 } else { 1.0 };
    Hydrodynamics {
        inertia: Vec3::new(
            (b * b + c * c) * mass / 3.0,
            (a * a + c * c) * mass / 3.0,
            (a * a + b * b) * mass / 3.0,
        ),
        drag: Vec3::new(
            q * axial * (2.0 * b) * (2.0 * c),
            q * (2.0 * a) * (2.0 * c),
            q * (2.0 * a) * (2.0 * b),
        ),
        angular_drag: Vec3::new(
            q * a * (b.powi(4) + c.powi(4)),
            q * b * (a.powi(4) + c.powi(4)),
            q * c * (a.powi(4) + b.powi(4)),
        ),
        added_mass: Vec3::new(
            density * volume * if streamlined { 0.1 } else { 0.2 },
            density * std::f64::consts::PI * c * c * 2.0 * a,
            density * std::f64::consts::PI * b * b * 2.0 * a,
        ),
        added_inertia: Vec3::new(
            2.0 * a * density * std::f64::consts::PI / 8.0 * (b * b - c * c).powi(2),
            density * std::f64::consts::PI * b * b * 2.0 * a * a * a / 3.0,
            density * std::f64::consts::PI * c * c * 2.0 * a * a * a / 3.0,
        ),
    }
}

/// Output of [`estimate_hydrodynamics`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hydrodynamics {
    pub inertia: Vec3,
    pub drag: Vec3,
    pub angular_drag: Vec3,
    pub added_mass: Vec3,
    pub added_inertia: Vec3,
}
impl Hydrodynamics {
    /// Writes the estimate into a body model as explicit overrides.
    pub fn apply(&self, model: &mut BodyModel) {
        model.inertia = Some(self.inertia);
        model.drag = Some(self.drag);
        model.angular_drag = Some(self.angular_drag);
        model.added_mass = self.added_mass;
        model.added_inertia = self.added_inertia;
    }
}
