//! Bounded geometry-only OBJ import. Embedded meshes keep projects portable.
use crate::math::Vec3;
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Mesh {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[usize; 3]>,
    pub source: String,
}
impl Mesh {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.vertices.len() < 3
            || self.vertices.len() > 20000
            || self.triangles.is_empty()
            || self.triangles.len() > 40000
            || self
                .vertices
                .iter()
                .any(|v| !v.is_finite() || v.length() > 1e6)
            || self
                .triangles
                .iter()
                .flatten()
                .any(|&i| i >= self.vertices.len())
            || self.source.len() > 256
        {
            return Err("invalid or oversized mesh");
        }
        Ok(())
    }
    pub fn parse_obj(text: &str, source: &str) -> Result<Self, &'static str> {
        if text.len() > 8_388_608 {
            return Err("OBJ exceeds 8 MB");
        }
        let mut mesh = Self {
            vertices: vec![],
            triangles: vec![],
            source: source.chars().take(256).collect(),
        };
        for line in text.lines() {
            let mut words = line.split_whitespace();
            match words.next() {
                Some("v") => {
                    let mut next = || {
                        words
                            .next()
                            .ok_or("missing vertex")?
                            .parse::<f64>()
                            .map_err(|_| "invalid vertex")
                    };
                    mesh.vertices.push(Vec3::new(next()?, next()?, next()?));
                }
                Some("f") => {
                    let mut face = vec![];
                    for word in words.take(65) {
                        let index = word
                            .split('/')
                            .next()
                            .ok_or("invalid face")?
                            .parse::<i64>()
                            .map_err(|_| "invalid face index")?;
                        let i = if index > 0 {
                            index - 1
                        } else if index < 0 {
                            mesh.vertices.len() as i64 + index
                        } else {
                            return Err("OBJ indices start at 1");
                        };
                        if i < 0 || i as usize >= mesh.vertices.len() {
                            return Err("face refers to unknown vertex");
                        }
                        face.push(i as usize);
                    }
                    if face.len() < 3 || face.len() > 64 {
                        return Err("face must have 3–64 vertices");
                    }
                    for i in 1..face.len() - 1 {
                        mesh.triangles.push([face[0], face[i], face[i + 1]]);
                    }
                }
                _ => {}
            }
            if mesh.vertices.len() > 20000 || mesh.triangles.len() > 40000 {
                return Err("OBJ exceeds geometry budget");
            }
        }
        mesh.validate()?;
        let mut min = mesh.vertices[0];
        let mut max = min;
        for p in &mesh.vertices {
            min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
            max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
        }
        let center = (min + max) * 0.5;
        let span = max - min;
        let extent = span.x.max(span.y).max(span.z) * 0.5;
        if extent < 1e-8 {
            return Err("mesh has zero extent");
        }
        for p in &mut mesh.vertices {
            *p = (*p - center) / extent;
        }
        Ok(mesh)
    }
    /// Stretches a centred, uniformly normalised mesh so it fills the unit
    /// box on every axis (entities draw it scaled by their body half sizes),
    /// returning the original per-axis proportions (largest axis = 1).
    /// Flat axes keep a small minimum proportion and are not stretched.
    pub fn fill_unit_box(&mut self) -> [f64; 3] {
        let mut extent = [0.0f64; 3];
        for v in &self.vertices {
            extent[0] = extent[0].max(v.x.abs());
            extent[1] = extent[1].max(v.y.abs());
            extent[2] = extent[2].max(v.z.abs());
        }
        let largest = extent.iter().copied().fold(0.0, f64::max).max(1e-9);
        let scale = extent.map(|e| if e > 1e-6 { 1.0 / e } else { 1.0 });
        for v in &mut self.vertices {
            *v = Vec3::new(v.x * scale[0], v.y * scale[1], v.z * scale[2]);
        }
        extent.map(|e| (e / largest).clamp(0.02, 1.0))
    }
}
fn unit() -> [f64; 3] {
    [1.0; 3]
}

/// Maximum mesh assets in one project library.
pub const MAX_MESH_ASSETS: usize = 32;
/// Maximum project materials (built-in presets are copied in on first use).
pub const MAX_MATERIALS: usize = 24;
/// Combined vertex budget for every mesh in a project library.
pub const LIBRARY_VERTEX_BUDGET: usize = 160_000;

/// Simple metallic/roughness surface description shared by every renderer.
/// `base_color` is sRGB (0–255). The CPU reference renderer uses base colour
/// only; the GPU preview additionally uses roughness and metallic.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Material {
    pub id: u64,
    pub name: String,
    pub base_color: [u8; 3],
    pub roughness: f64,
    pub metallic: f64,
}
impl Material {
    pub fn new(id: u64, name: &str, base_color: [u8; 3], roughness: f64, metallic: f64) -> Self {
        Self {
            id,
            name: name.into(),
            base_color,
            roughness,
            metallic,
        }
    }
    /// Built-in starting materials. Their IDs are zero until copied into a
    /// project library, which then assigns a stable project ID.
    pub fn presets() -> Vec<Self> {
        vec![
            Self::new(0, "Safety yellow paint", [232, 178, 32], 0.45, 0.0),
            Self::new(0, "Hull white gelcoat", [226, 229, 226], 0.28, 0.0),
            Self::new(0, "Anodised aluminium", [168, 176, 184], 0.32, 1.0),
            Self::new(0, "Black rubber", [28, 30, 32], 0.9, 0.0),
            Self::new(0, "Signal orange", [236, 106, 28], 0.5, 0.0),
            Self::new(0, "Weathered copper", [176, 112, 74], 0.55, 1.0),
            Self::new(0, "Marine-growth green", [74, 104, 70], 0.95, 0.0),
        ]
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 {
            return Err("Material IDs must be positive.".into());
        }
        if self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("Material names must be 1–80 printable characters.".into());
        }
        if !self.roughness.is_finite() || !(0.02..=1.0).contains(&self.roughness) {
            return Err("Material roughness must be 0.02–1.".into());
        }
        if !self.metallic.is_finite() || !(0.0..=1.0).contains(&self.metallic) {
            return Err("Material metallic must be 0–1.".into());
        }
        Ok(())
    }
    /// Base colour in linear 0–1 space for physically based shading.
    pub fn linear(&self) -> [f32; 3] {
        self.base_color.map(|v| (v as f32 / 255.0).powf(2.2))
    }
    /// Base colour in sRGB 0–255 floats for the CPU reference renderer.
    pub fn srgb(&self) -> [f32; 3] {
        self.base_color.map(f32::from)
    }
}

/// A named mesh in the project library, referenced by entities through `id`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshAsset {
    pub id: u64,
    pub name: String,
    pub mesh: std::sync::Arc<Mesh>,
    /// Source proportions (largest axis 1). The stored mesh fills its unit
    /// box; placing an instance sizes the body box with these proportions.
    #[serde(default = "unit")]
    pub proportions: [f64; 3],
}

/// Portable per-project asset library. Everything is embedded in the project:
/// no entry depends on an external path after import.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Library {
    #[serde(default)]
    pub meshes: Vec<MeshAsset>,
    #[serde(default)]
    pub materials: Vec<Material>,
}
impl Library {
    pub fn mesh(&self, id: u64) -> Option<&MeshAsset> {
        self.meshes.iter().find(|m| m.id == id)
    }
    pub fn material(&self, id: u64) -> Option<&Material> {
        self.materials.iter().find(|m| m.id == id)
    }
    pub fn material_mut(&mut self, id: u64) -> Option<&mut Material> {
        self.materials.iter_mut().find(|m| m.id == id)
    }
    pub fn vertex_count(&self) -> usize {
        self.meshes.iter().map(|m| m.mesh.vertices.len()).sum()
    }
    fn next_id<'a>(ids: impl Iterator<Item = &'a u64>) -> u64 {
        ids.max().copied().unwrap_or(0) + 1
    }
    /// Adds a mesh drawn as authored (proportions 1:1:1).
    pub fn add_mesh(&mut self, name: &str, mesh: std::sync::Arc<Mesh>) -> Result<u64, String> {
        self.add_mesh_with(name, mesh, [1.0; 3])
    }
    /// Adds a mesh, reusing an existing asset with identical geometry.
    pub fn add_mesh_with(
        &mut self,
        name: &str,
        mesh: std::sync::Arc<Mesh>,
        proportions: [f64; 3],
    ) -> Result<u64, String> {
        if let Some(existing) = self.meshes.iter().find(|m| *m.mesh == *mesh) {
            return Ok(existing.id);
        }
        if self.meshes.len() >= MAX_MESH_ASSETS {
            return Err(format!(
                "The project library holds at most {MAX_MESH_ASSETS} meshes."
            ));
        }
        let id = Self::next_id(self.meshes.iter().map(|m| &m.id));
        let name: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
        self.meshes.push(MeshAsset {
            id,
            name: if name.trim().is_empty() {
                format!("Mesh {id}")
            } else {
                name
            },
            mesh,
            proportions,
        });
        Ok(id)
    }
    /// Adds a material (for example a preset), reusing an existing material
    /// with the same name and surface values.
    pub fn add_material(&mut self, material: &Material) -> Result<u64, String> {
        if let Some(existing) = self.materials.iter().find(|m| {
            m.name == material.name
                && m.base_color == material.base_color
                && m.roughness == material.roughness
                && m.metallic == material.metallic
        }) {
            return Ok(existing.id);
        }
        if self.materials.len() >= MAX_MATERIALS {
            return Err(format!(
                "The project library holds at most {MAX_MATERIALS} materials."
            ));
        }
        let id = Self::next_id(self.materials.iter().map(|m| &m.id));
        let mut copy = material.clone();
        copy.id = id;
        copy.validate()?;
        self.materials.push(copy);
        Ok(id)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.meshes.len() > MAX_MESH_ASSETS || self.materials.len() > MAX_MATERIALS {
            return Err(format!(
                "Asset library supports {MAX_MESH_ASSETS} meshes and {MAX_MATERIALS} materials."
            ));
        }
        if self.vertex_count() > LIBRARY_VERTEX_BUDGET {
            return Err(format!(
                "Library meshes exceed the {LIBRARY_VERTEX_BUDGET}-vertex project budget."
            ));
        }
        let mut ids = std::collections::BTreeSet::new();
        for m in &self.meshes {
            if m.id == 0 || !ids.insert(m.id) {
                return Err("Mesh asset IDs must be positive and unique.".into());
            }
            if m.name.trim().is_empty()
                || m.name.chars().count() > 80
                || m.name.chars().any(char::is_control)
            {
                return Err("Mesh asset names must be 1–80 printable characters.".into());
            }
            m.mesh.validate().map_err(str::to_string)?;
            if m.proportions
                .iter()
                .any(|p| !p.is_finite() || !(0.02..=1.0).contains(p))
            {
                return Err("Mesh proportions must be 0.02–1.".into());
            }
        }
        ids.clear();
        for m in &self.materials {
            m.validate()?;
            if !ids.insert(m.id) {
                return Err("Material IDs must be unique.".into());
            }
        }
        Ok(())
    }
}
