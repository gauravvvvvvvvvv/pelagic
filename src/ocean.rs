use crate::math::Det;
use crate::math::Vec3;

/// Pluggable current field, sampled in world metres and simulation seconds.
pub trait CurrentField: Send + Sync {
    fn velocity(&self, position: Vec3, time: f64) -> Vec3;
}
pub struct UniformCurrent(pub Vec3);
impl CurrentField for UniformCurrent {
    fn velocity(&self, _: Vec3, _: f64) -> Vec3 {
        self.0
    }
}

/// Seafloor elevation in metres, negative below the surface at Z=0.
pub trait Bathymetry: Send + Sync {
    fn elevation(&self, x: f64, y: f64) -> f64;
    fn grid(&self) -> Option<&crate::terrain::HeightGrid> {
        None
    }
    /// Dynamic implementations increment this when geometry changes.
    fn revision(&self) -> u64 {
        0
    }
}
pub struct FlatBottom(pub f64);
impl Bathymetry for FlatBottom {
    fn elevation(&self, _: f64, _: f64) -> f64 {
        self.0
    }
    fn revision(&self) -> u64 {
        self.0.to_bits()
    }
}

pub struct Ocean {
    pub current: Box<dyn CurrentField>,
    pub bottom: Box<dyn Bathymetry>,
    pub density: f64,
    pub gravity: f64,
    /// Optical attenuation coefficient in inverse metres.
    pub attenuation: f64,
    /// Constant sound speed for the reference sonar model, metres/second.
    pub sound_speed: f64,
    pub waves: crate::waves::SeaState,
    pub profile: WaterProfile,
    pub structures: Vec<crate::geometry::StaticBox>,
    /// Visual meshes keyed by runtime body ID (reference body is 0).
    pub assets: std::collections::BTreeMap<u64, std::sync::Arc<crate::assets::Mesh>>,
    /// Visual materials keyed by runtime body ID. Never affects physics.
    pub materials: std::collections::BTreeMap<u64, crate::assets::Material>,
}
impl Default for Ocean {
    fn default() -> Self {
        Self {
            current: Box::new(UniformCurrent(Vec3::ZERO)),
            bottom: Box::new(FlatBottom(-100.0)),
            density: 1025.0,
            gravity: 9.80665,
            attenuation: 0.035,
            sound_speed: 1500.0,
            waves: crate::waves::SeaState::default(),
            profile: WaterProfile::default(),
            structures: vec![],
            assets: Default::default(),
            materials: Default::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct WaterSample {
    pub temperature: f64,
    pub salinity: f64,
    pub density: f64,
    pub sound_speed: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct WaterProfile {
    pub surface_temperature: f64,
    pub deep_temperature: f64,
    pub salinity: f64,
    pub thermocline: f64,
}
impl Default for WaterProfile {
    fn default() -> Self {
        Self {
            surface_temperature: 22.0,
            deep_temperature: 6.0,
            salinity: 35.0,
            thermocline: 35.0,
        }
    }
}
impl WaterProfile {
    /// Illustrative linearised equation of state and sound-speed approximation.
    /// This is not a TEOS-10 implementation. All assumptions are explicit.
    pub fn sample(self, depth: f64, reference_density: f64) -> WaterSample {
        let depth = depth.max(0.0);
        let mix = 0.5 + 0.5 * ((depth - self.thermocline) / 12.0).dtanh();
        let temperature = self.surface_temperature * (1.0 - mix) + self.deep_temperature * mix;
        let density = reference_density - 0.2 * (temperature - 15.0)
            + 0.77 * (self.salinity - 35.0)
            + 0.0045 * depth;
        let sound_speed = 1449.2 + 4.6 * temperature - 0.055 * temperature * temperature
            + 0.00029 * temperature.powi(3)
            + (1.34 - 0.01 * temperature) * (self.salinity - 35.0)
            + 0.016 * depth;
        WaterSample {
            temperature,
            salinity: self.salinity,
            density,
            sound_speed,
        }
    }
}
pub struct ShearedCurrent {
    pub surface: Vec3,
    pub shear: f64,
    pub tide_period: f64,
}
impl CurrentField for ShearedCurrent {
    fn velocity(&self, p: Vec3, time: f64) -> Vec3 {
        let tide = if self.tide_period > 0.0 {
            0.8 + 0.2 * (std::f64::consts::TAU * time / self.tide_period).dcos()
        } else {
            1.0
        };
        self.surface * ((1.0 + self.shear * p.z).max(0.0) * tide)
    }
}
impl Ocean {
    pub fn surface(&self, x: f64, y: f64, time: f64) -> crate::waves::SurfaceSample {
        self.waves.sample(x, y, time)
    }
    pub fn water(&self, p: Vec3) -> WaterSample {
        self.profile.sample(-p.z, self.density)
    }
    pub fn velocity(&self, p: Vec3, time: f64) -> Vec3 {
        self.current.velocity(p, time) + self.waves.orbital_velocity(p, time)
    }
}
