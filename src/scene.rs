//! Shared versioned authoring scene, used by headless and desktop workflows.
use crate as pelagic;
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path};
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub reference_mass: f64,
    pub reference_volume: f64,
    pub reference_xy: [f64; 2],
    pub reference_heading: f64,
    pub reference_force: Vec3,
    pub entities: Vec<crate::entities::Entity>,
    /// Embedded project mesh/material library referenced by stable handles.
    pub library: crate::assets::Library,
    pub reference_asset: Option<u64>,
    pub reference_material: Option<u64>,
    pub body_model: pelagic::dynamics::BodyModel,
    pub school: pelagic::life::SchoolConfig,
    pub particles: usize,
    pub sensors: pelagic::instruments::SensorConfig,
    pub flow_grid: Option<std::sync::Arc<pelagic::data::FlowGridData>>,
    pub provenance: pelagic::data::Provenance,
    pub name: String,
    pub density: f64,
    pub bottom_depth: f64,
    pub current: [f64; 3],
    pub attenuation: f64,
    pub vehicle: bool,
    pub vehicle_depth: f64,
    pub camera: [f64; 3],
    pub target: [f64; 3],
    pub seed: u64,
    pub wave_height: f64,
    pub wave_period: f64,
    pub wave_direction: f64,
    pub wave_spread: f64,
    /// Swell (in addition to the wind sea above): significant height (m,
    /// 0 = none), peak period (s) and direction of travel (degrees, same
    /// convention as `wave_direction`).
    pub swell_height: f64,
    pub swell_period: f64,
    pub swell_direction: f64,
    /// Wind 10 m above the sea (m/s, 0 = derive from the waves) and the
    /// direction it blows towards (degrees, as `wave_direction`).
    pub wind_speed: f64,
    pub wind_direction: f64,
    /// Where the sea state came from (e.g. a forecast and its time); empty
    /// when set by hand.
    pub sea_source: String,
    pub terrain: u8,
    pub terrain_material: u8,
    pub grid: Option<std::sync::Arc<pelagic::terrain::HeightGrid>>,
    pub temperature: f64,
    pub deep_temperature: f64,
    pub salinity: f64,
    pub thermocline: f64,
    pub shear: f64,
    pub tide_period: f64,
    pub sun_elevation: f64,
    /// Sun azimuth, degrees clockwise from north (scene Y is north).
    pub sun_azimuth: f64,
    /// Cloud cover fraction 0-1 (visual).
    pub clouds: f64,
    /// Wide, coarse terrain around the scene for distant coasts and
    /// mountains (visual only; physics uses `grid`).
    pub backdrop: Option<std::sync::Arc<pelagic::terrain::HeightGrid>>,
    /// Mapped seabed habitat (coral reef, seagrass; visual only).
    pub habitat: Option<std::sync::Arc<pelagic::habitat::Habitat>>,
    /// Satellite colours for land in the scene and in the backdrop
    /// (visual only).
    pub imagery: Option<std::sync::Arc<pelagic::habitat::Imagery>>,
    pub backdrop_imagery: Option<std::sync::Arc<pelagic::habitat::Imagery>>,
    pub quality: u8,
    pub structure: bool,
    pub structure_position: [f64; 3],
    pub structure_scale: f64,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            reference_mass: 12.0,
            reference_volume: 12.0 / 1025.0,
            reference_xy: [0.0, 0.0],
            reference_heading: 0.0,
            reference_force: Vec3::ZERO,
            entities: vec![],
            library: Default::default(),
            reference_asset: None,
            reference_material: None,
            body_model: pelagic::dynamics::BodyModel::rov(),
            school: Default::default(),
            particles: 64,
            sensors: Default::default(),
            flow_grid: None,
            provenance: Default::default(),
            name: "Untitled ocean".into(),
            density: 1025.0,
            bottom_depth: 40.0,
            current: [0.3, 0.0, 0.0],
            attenuation: 0.035,
            vehicle: true,
            vehicle_depth: 20.0,
            camera: [-95.0, -125.0, 34.0],
            target: [0.0, 15.0, 0.0],
            seed: 42,
            wave_height: 1.2,
            wave_period: 6.0,
            wave_direction: 35.0,
            wave_spread: 25.0,
            swell_height: 0.0,
            swell_period: 11.0,
            swell_direction: 35.0,
            wind_speed: 0.0,
            wind_direction: 35.0,
            sea_source: String::new(),
            terrain: 1,
            terrain_material: 0,
            grid: None,
            temperature: 22.0,
            deep_temperature: 6.0,
            salinity: 35.0,
            thermocline: 35.0,
            shear: 0.008,
            tide_period: 0.0,
            sun_elevation: 38.0,
            sun_azimuth: 150.0,
            clouds: 0.3,
            backdrop: None,
            habitat: None,
            imagery: None,
            backdrop_imagery: None,
            quality: 0,
            structure: true,
            structure_position: [-38.0, 58.0, 1.0],
            structure_scale: 1.0,
        }
    }
}
impl Scene {
    pub fn surface_demo() -> Self {
        let mut scene = Self {
            name: "Surface sea-state study".into(),
            vehicle: false,
            particles: 0,
            wave_height: 1.8,
            wave_period: 7.5,
            structure: true,
            ..Self::default()
        };
        scene.school.count = 0;
        scene.entities = vec![crate::entities::Entity::boat(1)];
        scene
    }
    pub fn sensor_demo() -> Self {
        let mut scene = Self::robotics_demo();
        scene.name = "Sonar and navigation experiment".into();
        scene.entities.clear();
        scene.school.count = 12;
        scene.particles = 32;
        scene.sensors.rate_hz = 10.0;
        scene.sensors.beams = 64;
        scene.sensors.latency_seconds = 0.1;
        scene.sensors.tilt_degrees = 75.0;
        scene
    }
    pub fn robotics_demo() -> Self {
        let mut s = Self {
            name: "Marine robotics lab".into(),
            terrain: 0,
            bottom_depth: 30.0,
            vehicle_depth: 12.0,
            wave_height: 0.4,
            current: [0.15, 0.0, 0.0],
            camera: [-9.0, -13.0, -7.0],
            target: [2.0, 1.0, -12.0],
            ..Default::default()
        };
        s.body_model.hold_depth = Some(12.0);
        s.body_model.hold_heading = Some(0.0);
        s.school.count = 24;
        s.school.center = Vec3::new(5.0, 2.0, -12.0);
        s.entities = vec![
            crate::entities::Entity::boat(1),
            crate::entities::Entity::drifter(2),
        ];
        s
    }
    pub fn ecology_demo() -> Self {
        let mut s = Self {
            name: "Pelagic schooling lab".into(),
            camera: [-8.0, -15.0, -14.0],
            target: [5.0, 2.0, -18.0],
            vehicle: false,
            structure: false,
            ..Default::default()
        };
        s.school.count = 96;
        s
    }
    pub fn reference_entity(&self) -> crate::entities::Entity {
        let mut e = crate::entities::Entity::rov(
            1,
            Vec3::new(
                self.reference_xy[0],
                self.reference_xy[1],
                -self.vehicle_depth,
            ),
        );
        e.name = "Reference vehicle".into();
        e.model = self.body_model.clone();
        e.mass = self.reference_mass;
        e.volume = self.reference_volume;
        e.heading = self.reference_heading;
        e.force = self.reference_force;
        e.asset = self.reference_asset;
        e.material = self.reference_material;
        e
    }
    pub fn set_reference(&mut self, e: &crate::entities::Entity) {
        self.reference_xy = [e.position.x, e.position.y];
        self.vehicle_depth = -e.position.z;
        self.reference_mass = e.mass;
        self.reference_volume = e.volume;
        self.reference_heading = e.heading;
        self.reference_force = e.force;
        self.body_model = e.model.clone();
        self.reference_asset = e.asset;
        self.reference_material = e.material;
    }
    pub fn validate(&self) -> Result<(), String> {
        self.reference_entity().validate()?;
        self.body_model.validate().map_err(str::to_string)?;
        self.school.validate().map_err(str::to_string)?;
        self.sensors.validate().map_err(str::to_string)?;
        if self.entities.len() > 15 || self.particles > 512 {
            return Err(
                "Scene supports 16 bodies including reference, 256 fish, and 512 tracers.".into(),
            );
        }
        self.library.validate()?;
        let mut ids = std::collections::BTreeSet::new();
        for e in &self.entities {
            e.validate()?;
            if !ids.insert(e.id) {
                return Err("Entity IDs must be unique.".into());
            }
        }
        for (name, asset, material) in std::iter::once((
            "Reference vehicle",
            self.reference_asset,
            self.reference_material,
        ))
        .chain(
            self.entities
                .iter()
                .map(|e| (e.name.as_str(), e.asset, e.material)),
        ) {
            if asset.is_some_and(|id| self.library.mesh(id).is_none()) {
                return Err(format!("{name} refers to a mesh missing from the library."));
            }
            if material.is_some_and(|id| self.library.material(id).is_none()) {
                return Err(format!(
                    "{name} refers to a material missing from the library."
                ));
            }
        }
        if let Some(flow) = &self.flow_grid {
            if flow.values.len() > 131072 {
                return Err("Imported flow exceeds 131072 samples.".into());
            }
            flow.field().map_err(str::to_string)?;
        }
        let p = &self.provenance;
        if !p.origin_latitude.is_finite()
            || !p.origin_longitude.is_finite()
            || p.origin_latitude.abs() > 85.0
            || p.origin_longitude.abs() > 180.0
            || [
                &p.source,
                &p.source_url,
                &p.licence,
                &p.retrieved_utc,
                &p.resolution,
                &p.classification,
            ]
            .iter()
            .any(|s| s.len() > 2048)
        {
            return Err("Invalid data provenance or local geographic origin.".into());
        }
        if let Some(g) = &self.grid
            && (g.columns > 257
                || g.rows > 257
                || !(0.1..=5000.0).contains(&g.spacing)
                || g.elevations
                    .iter()
                    .any(|v| !v.is_finite() || !(-11000.0..=9000.0).contains(v)))
        {
            return Err("Terrain requires at most 257×257 finite elevations (−11000–9000 m), with spacing 0.1–1000 m.".into());
        }
        if self
            .structure_position
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 256.0)
            || !self.structure_scale.is_finite()
            || !(0.1..=5.0).contains(&self.structure_scale)
        {
            return Err("Structure must stay in the local region, with scale 0.1–5.".into());
        }
        for (label, v, lo, hi) in [
            ("Wave height", self.wave_height, 0.0, 15.0),
            ("Swell height", self.swell_height, 0.0, 12.0),
            ("Swell period", self.swell_period, 4.0, 30.0),
            ("Swell direction", self.swell_direction, -360.0, 360.0),
            ("Wind speed", self.wind_speed, 0.0, 60.0),
            ("Wind direction", self.wind_direction, -360.0, 360.0),
            ("Wave period", self.wave_period, 2.0, 20.0),
            ("Wave direction", self.wave_direction, -360.0, 360.0),
            ("Directional spread", self.wave_spread, 0.0, 90.0),
            ("Temperature", self.temperature, -2.0, 35.0),
            ("Deep temperature", self.deep_temperature, -2.0, 35.0),
            ("Salinity", self.salinity, 0.0, 42.0),
            ("Thermocline depth", self.thermocline, 1.0, 500.0),
            ("Current shear", self.shear, -0.05, 0.05),
            ("Tide period", self.tide_period, 0.0, 86400.0),
            ("Sun elevation", self.sun_elevation, -6.0, 90.0),
            ("Sun azimuth", self.sun_azimuth, -360.0, 360.0),
            ("Cloud cover", self.clouds, 0.0, 1.0),
        ] {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("{label} must be {lo}–{hi}."));
            }
        }
        if self.terrain > 3 || self.quality > 2 || self.terrain_material > 2 {
            return Err("Unsupported terrain or quality setting.".into());
        }
        if self.terrain == 3 && self.grid.is_none() {
            return Err("Import a terrain grid first.".into());
        }
        let steep = |h: f64, t: f64| h > 0.1 * 9.80665 * t * t / std::f64::consts::TAU;
        if self.wave_height.hypot(self.swell_height) > self.bottom_depth * 0.25
            || steep(self.wave_height, self.wave_period)
            || steep(self.swell_height, self.swell_period)
        {
            return Err(
                "This sea state exceeds the supported linear-wave steepness/depth envelope.".into(),
            );
        }
        if self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("Use a project name of 1–80 characters.".into());
        }
        if !self.density.is_finite() || !(900.0..=1300.0).contains(&self.density) {
            return Err("Density must be 900–1300 kg/m³.".into());
        }
        if !self.bottom_depth.is_finite() || !(1.0..=11000.0).contains(&self.bottom_depth) {
            return Err("Seabed depth must be 1–11000 m.".into());
        }
        if !self.vehicle_depth.is_finite()
            || self.vehicle_depth <= 0.0
            || (self.vehicle && self.vehicle_depth >= self.bottom_depth)
        {
            return Err("Vehicle depth must be between the surface and seabed.".into());
        }
        if !self.attenuation.is_finite() || !(0.0..=5.0).contains(&self.attenuation) {
            return Err("Attenuation must be 0–5 per metre.".into());
        }
        if self
            .current
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 20.0)
        {
            return Err("Current components must be between −20 and 20 m/s.".into());
        }
        if self
            .camera
            .iter()
            .chain(self.target.iter())
            .any(|v| !v.is_finite() || v.abs() > 100000.0)
        {
            return Err("Invalid camera coordinates.".into());
        }
        let dx = self.target[0] - self.camera[0];
        let dy = self.target[1] - self.camera[1];
        if dx.hypot(dy) < 0.01 {
            return Err("Camera must have a horizontal viewing direction.".into());
        }
        Ok(())
    }
    pub fn value(&self) -> Value {
        let grid = self.grid.as_ref().map(|g| grid_value(g));
        let mut value = json!({"format":"pelagic-project","version":3,"name":self.name,"scene":{"density":self.density,"bottom_depth":self.bottom_depth,"current":self.current,"attenuation":self.attenuation,"vehicle":self.vehicle,"vehicle_depth":self.vehicle_depth,"seed":self.seed},"camera":{"position":self.camera,"target":self.target},"environment":{"wave_height":self.wave_height,"wave_period":self.wave_period,"wave_direction":self.wave_direction,"wave_spread":self.wave_spread,"terrain":self.terrain,"temperature":self.temperature,"deep_temperature":self.deep_temperature,"salinity":self.salinity,"thermocline":self.thermocline,"shear":self.shear,"tide_period":self.tide_period,"sun_elevation":self.sun_elevation,"quality":self.quality,"grid":grid}});
        value["structure"] = json!({"enabled":self.structure,"position":self.structure_position,"scale":self.structure_scale});
        value["environment"]["terrain_material"] = json!(self.terrain_material);
        value["environment"]["swell_height"] = json!(self.swell_height);
        value["environment"]["swell_period"] = json!(self.swell_period);
        value["environment"]["swell_direction"] = json!(self.swell_direction);
        value["environment"]["wind_speed"] = json!(self.wind_speed);
        value["environment"]["wind_direction"] = json!(self.wind_direction);
        value["environment"]["sea_source"] = json!(self.sea_source);
        value["environment"]["sun_azimuth"] = json!(self.sun_azimuth);
        value["environment"]["clouds"] = json!(self.clouds);
        value["environment"]["backdrop"] = self
            .backdrop
            .as_ref()
            .map_or(Value::Null, |g| grid_value(g));
        value["environment"]["habitat"] = self.habitat.as_ref().map_or(Value::Null, |h| h.value());
        value["environment"]["imagery"] = self.imagery.as_ref().map_or(Value::Null, |i| i.value());
        value["environment"]["backdrop_imagery"] = self
            .backdrop_imagery
            .as_ref()
            .map_or(Value::Null, |i| i.value());
        value["version"] = json!(5);
        value["library"] = json!(self.library);
        value["engine_build"] = json!(crate::BUILD_ID);
        value["reference"] = json!(self.reference_entity());
        value["entities"] = json!(self.entities);
        value["body_model"] = json!(self.body_model);
        value["school"] = json!(self.school);
        value["particles"] = json!(self.particles);
        value["sensors"] = json!(self.sensors);
        value["flow_grid"] = json!(self.flow_grid);
        value["provenance"] = json!(self.provenance);
        value["metadata"] = json!({"engine_version":env!("CARGO_PKG_VERSION"),"units":"SI","coordinates":"local metres; X east, Y north, Z up; surface Z=0","wave_model":"linear finite-depth; 32-component JONSWAP-shaped spectrum v1","water_profile":"illustrative linear density and empirical sound speed v1","terrain_source":if self.terrain==3 {"embedded user grid"} else {"synthetic preset"}});
        value
    }
    pub fn from_value(v: &Value) -> Result<Self, String> {
        if v["format"] != "pelagic-project" {
            return Err("Not a Pelagic project.".into());
        }
        let version = v["version"].as_u64().ok_or("Project version is missing.")?;
        let mut result = Self {
            name: v["name"].as_str().ok_or("Project name is missing.")?.into(),
            ..Self::default()
        };
        if (2..=5).contains(&version) {
            let num = |key: &str| {
                v["scene"][key]
                    .as_f64()
                    .ok_or_else(|| format!("Missing or invalid {key}."))
            };
            result.density = num("density")?;
            result.bottom_depth = num("bottom_depth")?;
            result.attenuation = num("attenuation")?;
            result.vehicle_depth = num("vehicle_depth")?;
            result.vehicle = v["scene"]["vehicle"]
                .as_bool()
                .ok_or("Missing vehicle setting.")?;
            result.seed = v["scene"]["seed"].as_u64().ok_or("Missing seed.")?;
            result.current = vector(&v["scene"]["current"])?;
            result.camera = vector(&v["camera"]["position"])?;
            result.target = vector(&v["camera"]["target"])?;
        } else if version != 1 {
            return Err(format!("Project version {version} is not supported."));
        }
        if version < 3 {
            result.terrain = 0;
            result.wave_height = 0.0;
            result.structure = false;
        } else {
            result.structure = v["structure"]["enabled"].as_bool().unwrap_or(false);
            result.terrain_material = v["environment"]["terrain_material"]
                .as_u64()
                .unwrap_or(0)
                .try_into()
                .map_err(|_| "Invalid terrain material")?;
            if !v["structure"]["position"].is_null() {
                result.structure_position = vector(&v["structure"]["position"])?;
            }
            result.structure_scale = v["structure"]["scale"].as_f64().unwrap_or(1.0);
            let n = |key: &str| {
                v["environment"][key]
                    .as_f64()
                    .ok_or_else(|| format!("Missing environment setting: {key}"))
            };
            result.wave_height = n("wave_height")?;
            result.wave_period = n("wave_period")?;
            result.wave_direction = n("wave_direction")?;
            result.wave_spread = n("wave_spread")?;
            // Optional (added with live sea states); older files have none.
            let or = |key: &str, default: f64| v["environment"][key].as_f64().unwrap_or(default);
            result.swell_height = or("swell_height", 0.0);
            result.swell_period = or("swell_period", 11.0);
            result.swell_direction = or("swell_direction", result.wave_direction);
            result.wind_speed = or("wind_speed", 0.0);
            result.wind_direction = or("wind_direction", result.wave_direction);
            result.sea_source = v["environment"]["sea_source"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(200)
                .collect();
            result.temperature = n("temperature")?;
            result.deep_temperature = n("deep_temperature")?;
            result.salinity = n("salinity")?;
            result.thermocline = n("thermocline")?;
            result.shear = n("shear")?;
            result.tide_period = n("tide_period")?;
            result.sun_elevation = n("sun_elevation")?;
            result.terrain = v["environment"]["terrain"]
                .as_u64()
                .filter(|v| *v <= 3)
                .ok_or("Invalid terrain")? as u8;
            result.quality = v["environment"]["quality"]
                .as_u64()
                .filter(|v| *v <= 2)
                .ok_or("Invalid quality")? as u8;
            result.grid = grid_from(&v["environment"]["grid"])?.map(std::sync::Arc::new);
            // Optional visual layers (added with the realistic renderer).
            result.sun_azimuth = or("sun_azimuth", 150.0);
            result.clouds = or("clouds", 0.3);
            result.backdrop = grid_from(&v["environment"]["backdrop"])?.map(std::sync::Arc::new);
            let h = &v["environment"]["habitat"];
            if !h.is_null() {
                result.habitat = Some(std::sync::Arc::new(pelagic::habitat::Habitat::from_value(
                    h,
                )?));
            }
            for (key, slot) in [
                ("imagery", &mut result.imagery),
                ("backdrop_imagery", &mut result.backdrop_imagery),
            ] {
                let i = &v["environment"][key];
                if !i.is_null() {
                    *slot = Some(std::sync::Arc::new(pelagic::habitat::Imagery::from_value(
                        i,
                    )?));
                }
            }
        }
        if version < 4 {
            result.entities.clear();
            result.school.count = 0;
            result.particles = 0;
            result.body_model = Default::default();
        } else {
            let reference: crate::entities::Entity =
                serde_json::from_value(v["reference"].clone()).map_err(|e| e.to_string())?;
            reference.validate()?;
            result.set_reference(&reference);
            let mut entities = v["entities"].clone();
            if version == 4 {
                // v4 embedded an optional mesh in each entity; move each into the
                // library (identical geometry shares one asset) and keep a handle.
                for e in entities.as_array_mut().ok_or("Invalid entity list.")? {
                    let mesh = e
                        .as_object_mut()
                        .and_then(|o| o.remove("mesh"))
                        .unwrap_or(Value::Null);
                    if !mesh.is_null() {
                        let mesh: crate::assets::Mesh =
                            serde_json::from_value(mesh).map_err(|e| e.to_string())?;
                        mesh.validate().map_err(str::to_string)?;
                        let name = if mesh.source.trim().is_empty() {
                            e["name"].as_str().unwrap_or("Imported mesh").to_string()
                        } else {
                            mesh.source.clone()
                        };
                        let id = result.library.add_mesh(&name, std::sync::Arc::new(mesh))?;
                        e["asset"] = json!(id);
                    }
                }
            } else {
                result.library =
                    serde_json::from_value(v["library"].clone()).map_err(|e| e.to_string())?;
            }
            result.entities = serde_json::from_value(entities).map_err(|e| e.to_string())?;
            let legacy_model: crate::dynamics::BodyModel =
                serde_json::from_value(v["body_model"].clone()).map_err(|e| e.to_string())?;
            if legacy_model != reference.model {
                return Err("Conflicting reference body models in project.".into());
            }
            result.school =
                serde_json::from_value(v["school"].clone()).map_err(|e| e.to_string())?;
            result.sensors =
                serde_json::from_value(v["sensors"].clone()).map_err(|e| e.to_string())?;
            result.particles = v["particles"]
                .as_u64()
                .filter(|n| *n <= 512)
                .ok_or("Invalid tracer count")? as usize;
            result.flow_grid =
                serde_json::from_value(v["flow_grid"].clone()).map_err(|e| e.to_string())?;
            result.provenance =
                serde_json::from_value(v["provenance"].clone()).map_err(|e| e.to_string())?;
        }
        result.validate()?;
        Ok(result)
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut data = Vec::new();
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(16_777_217)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if data.len() > 16_777_216 {
            return Err("Project exceeds the 16 MB limit.".into());
        }
        Self::from_value(
            &serde_json::from_slice(&data).map_err(|e| format!("Invalid project: {e}"))?,
        )
    }
}
fn grid_value(g: &pelagic::terrain::HeightGrid) -> Value {
    json!({"columns":g.columns,"rows":g.rows,"origin":g.origin,"spacing":g.spacing,"elevations":g.elevations})
}
fn grid_from(g: &Value) -> Result<Option<pelagic::terrain::HeightGrid>, String> {
    if g.is_null() {
        return Ok(None);
    }
    let columns = g["columns"]
        .as_u64()
        .filter(|n| (2..=257).contains(n))
        .ok_or("Grid width must be 2–257.")? as usize;
    let rows = g["rows"]
        .as_u64()
        .filter(|n| (2..=257).contains(n))
        .ok_or("Grid height must be 2–257.")? as usize;
    let origin = g["origin"]
        .as_array()
        .filter(|v| v.len() == 2)
        .ok_or("Invalid grid origin")?;
    let origin = [
        origin[0].as_f64().ok_or("Invalid origin")?,
        origin[1].as_f64().ok_or("Invalid origin")?,
    ];
    let elevations = g["elevations"]
        .as_array()
        .ok_or("Invalid grid elevations")?
        .iter()
        .map(|v| v.as_f64().ok_or("Invalid height"))
        .collect::<Result<Vec<_>, _>>()?;
    pelagic::terrain::HeightGrid::new(
        columns,
        rows,
        origin,
        g["spacing"].as_f64().ok_or("Invalid spacing")?,
        elevations,
    )
    .map(Some)
    .map_err(str::to_string)
}
fn vector(v: &Value) -> Result<[f64; 3], String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or("Expected three coordinates.")?;
    Ok([
        a[0].as_f64().ok_or("Invalid coordinate.")?,
        a[1].as_f64().ok_or("Invalid coordinate.")?,
        a[2].as_f64().ok_or("Invalid coordinate.")?,
    ])
}

use crate::{
    World,
    math::Vec3,
    ocean::{Ocean, ShearedCurrent, WaterProfile},
};
const DT: f64 = 1.0 / 60.0;
fn vec(v: [f64; 3]) -> Vec3 {
    Vec3::new(v[0], v[1], v[2])
}
/// The scene's sea: wind sea plus optional swell, with its wind speed.
pub fn sea_state(scene: &Scene) -> pelagic::waves::SeaState {
    let mut sea = pelagic::waves::SeaState::spectrum(
        scene.wave_height,
        scene.wave_period,
        scene.wave_direction.to_radians(),
        scene.wave_spread.to_radians(),
        scene.bottom_depth,
        scene.seed,
    )
    .expect("validated wave settings");
    if scene.swell_height > 0.0 {
        sea = sea.with(
            pelagic::waves::SeaState::swell(
                scene.swell_height,
                scene.swell_period,
                scene.swell_direction.to_radians(),
                10f64.to_radians(),
                scene.bottom_depth,
                scene.seed,
            )
            .expect("validated swell settings"),
        );
    }
    sea.wind_speed = scene.wind_speed;
    sea
}
pub fn world(scene: &Scene) -> Result<World, String> {
    scene.validate()?;
    let ocean = Ocean {
        bottom: Box::new(
            scene
                .grid
                .as_ref()
                .filter(|_| scene.terrain == 3)
                .map(|g| (**g).clone())
                .unwrap_or_else(|| {
                    pelagic::terrain::HeightGrid::procedural(scene.terrain, scene.bottom_depth)
                }),
        ),
        current: if let Some(data) = &scene.flow_grid {
            Box::new(data.field().expect("validated flow grid"))
        } else {
            Box::new(ShearedCurrent {
                surface: vec(scene.current),
                shear: scene.shear,
                tide_period: scene.tide_period,
            })
        },
        density: scene.density,
        attenuation: scene.attenuation,
        waves: sea_state(scene),
        profile: WaterProfile {
            surface_temperature: scene.temperature,
            deep_temperature: scene.deep_temperature,
            salinity: scene.salinity,
            thermocline: scene.thermocline,
        },
        structures: if scene.structure {
            let p = vec(scene.structure_position);
            let s = scene.structure_scale;
            let mut boxes = vec![pelagic::geometry::StaticBox {
                center: p,
                half: Vec3::new(15.0, 3.0, 0.7) * s,
                color: [150.0, 142.0, 119.0],
            }];
            for x in [-12.0, 12.0] {
                for y in [-2.0, 2.0] {
                    boxes.push(pelagic::geometry::StaticBox {
                        center: p + Vec3::new(x, y, -9.0) * s,
                        half: Vec3::new(0.55, 0.55, 9.0) * s,
                        color: [80.0, 91.0, 89.0],
                    });
                }
            }
            boxes
        } else {
            vec![]
        },
        ..Ocean::default()
    };
    let mut world = World::new(ocean, DT, scene.seed).expect("reference step");
    let mut bind = |body: u64, asset: Option<u64>, material: Option<u64>| {
        if let Some(a) = asset.and_then(|id| scene.library.mesh(id)) {
            world.ocean.assets.insert(body, a.mesh.clone());
        }
        if let Some(m) = material.and_then(|id| scene.library.material(id)) {
            world.ocean.materials.insert(body, m.clone());
        }
    };
    if scene.vehicle {
        bind(0, scene.reference_asset, scene.reference_material);
    }
    for e in &scene.entities {
        bind(e.id, e.asset, e.material);
    }
    if scene.vehicle {
        let mut vehicle = scene.reference_entity().spawn();
        vehicle.id = 0;
        vehicle.asset = vehicle.asset.map(|_| 0);
        world.add_vehicle(vehicle);
    }
    for e in &scene.entities {
        world.add_vehicle(e.spawn());
    }
    if scene.school.count > 0 {
        world.schools.push(
            pelagic::life::School::new(scene.school.clone(), scene.seed ^ 0x4567)
                .expect("validated school"),
        );
    }
    let mut seed = scene.seed ^ 0x7654;
    for _ in 0..scene.particles {
        world.particles.push(pelagic::life::Particle {
            position: scene.school.center
                + Vec3::new(
                    pelagic::life::random(&mut seed) - 0.5,
                    pelagic::life::random(&mut seed) - 0.5,
                    pelagic::life::random(&mut seed) - 0.5,
                ) * 20.0,
            age: 0.0,
        });
    }
    Ok(world)
}
