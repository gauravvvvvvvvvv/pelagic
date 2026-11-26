//! Versioned scene documents, bounded edit history and crash-safe replacement.
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub use pelagic::scene::Scene;
pub struct Document {
    pub scene: Scene,
    pub path: Option<PathBuf>,
    saved: Option<Scene>,
    undo: Vec<Scene>,
    redo: Vec<Scene>,
}
impl Document {
    pub fn new(scene: Scene) -> Self {
        Self {
            scene,
            path: None,
            saved: None,
            undo: vec![],
            redo: vec![],
        }
    }
    pub fn open(path: &Path) -> Result<Self, String> {
        let scene = Scene::read(path)?;
        Ok(Self {
            saved: Some(scene.clone()),
            scene,
            path: Some(path.into()),
            undo: vec![],
            redo: vec![],
        })
    }
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.scene)
    }
    pub fn edit(&mut self, next: Scene) -> Result<(), String> {
        next.validate()?;
        if serde_json::to_vec_pretty(&next.value())
            .map_err(|e| e.to_string())?
            .len()
            > 16_000_000
        {
            return Err("Project assets exceed the portable document budget. Reduce geometry/current-grid size before committing.".into());
        }
        if next != self.scene {
            self.undo.push(self.scene.clone());
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.scene = next;
        }
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        if let Some(s) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.scene, s));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self) -> bool {
        if let Some(s) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.scene, s));
            true
        } else {
            false
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        self.scene.validate()?;
        atomic_json(path, &self.scene.value())?;
        self.path = Some(path.into());
        self.saved = Some(self.scene.clone());
        Ok(())
    }
    pub fn recover(&self, path: &Path) -> Result<(), String> {
        atomic_json(
            path,
            &json!({"scene":self.scene.value(),"original_path":self.path.as_ref().map(|p|p.to_string_lossy())}),
        )
    }
    pub fn restore(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(16_777_217)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 16_777_216 {
            return Err("Recovery file too large.".into());
        }
        let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        let mut doc = Self::new(Scene::from_value(&v["scene"])?);
        doc.path = v["original_path"].as_str().map(PathBuf::from);
        Ok(doc)
    }
}

static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Configure process-wide isolation before constructing any editor, feed or
/// cache worker. This also keeps isolated sessions away from saved API keys.
pub fn configure_storage(path: &Path) -> Result<(), String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    DATA_DIR
        .set(absolute)
        .map_err(|_| "Application storage is already configured.".into())
}

pub fn data_dir() -> PathBuf {
    if let Some(root) = DATA_DIR.get() {
        return root.clone();
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Pelagic")
}
pub fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if bytes.len() > 16_777_216 {
        return Err("Project/recovery exceeds 16 MB.".into());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("Choose an absolute project path.")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".pelagic-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            };
            let src: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let dst: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                MoveFileExW(
                    src.as_ptr(),
                    dst.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_and_embedded_terrain_roundtrip_without_precision_loss() {
        let grid = pelagic::terrain::HeightGrid::new(
            2,
            2,
            [-10.0, -10.0],
            20.0,
            vec![-30.0, -25.0, -40.0, -38.0],
        )
        .unwrap();
        let scene = Scene {
            terrain: 3,
            grid: Some(std::sync::Arc::new(grid)),
            wave_direction: 123.25,
            temperature: 19.0,
            shear: 0.015,
            ..Scene::default()
        };
        scene.validate().unwrap();
        let text = serde_json::to_string(&scene.value()).unwrap();
        let loaded = Scene::from_value(&serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(loaded, scene);
        let mut doc = Document::new(scene.clone());
        let mut next = scene.clone();
        next.wave_height = 2.0;
        doc.edit(next.clone()).unwrap();
        assert!(doc.undo());
        assert_eq!(doc.scene, scene);
        assert!(doc.redo());
        assert_eq!(doc.scene, next);
        let mut bad = scene.value();
        bad["environment"]["grid"]["spacing"] = json!(0.001);
        assert!(Scene::from_value(&bad).is_err());
    }
    #[test]
    fn migration_validation_and_versioning() {
        let old = json!({"format":"pelagic-project","version":1,"name":"Legacy"});
        let scene = Scene::from_value(&old).unwrap();
        assert_eq!(Scene::from_value(&scene.value()).unwrap(), scene);
        let mut bad = scene.value();
        bad["version"] = json!(999);
        assert!(Scene::from_value(&bad).is_err());
        bad = scene.value();
        bad["scene"]["density"] = json!(-1);
        assert!(Scene::from_value(&bad).is_err());
    }
    #[test]
    fn saving_history_and_recovery_preserve_scene() {
        let folder = std::env::temp_dir().join(format!(
            "pelagic-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = folder.join("scene.pelagic");
        let recovery = folder.join("recovery.json");
        let mut doc = Document::new(Scene::default());
        doc.save(&path).unwrap();
        assert!(!doc.dirty());
        let mut next = doc.scene.clone();
        next.bottom_depth = 80.0;
        next.camera = [-40.0, -10.0, -2.0];
        doc.edit(next.clone()).unwrap();
        assert!(doc.dirty());
        doc.recover(&recovery).unwrap();
        assert_eq!(Document::restore(&recovery).unwrap().scene, next);
        assert!(doc.undo());
        assert!(!doc.dirty());
        assert!(doc.redo());
        doc.save(&path).unwrap();
        assert_eq!(Document::open(&path).unwrap().scene, next);
        doc.scene.density = 0.0;
        assert!(doc.save(&path).is_err());
        assert_eq!(Document::open(&path).unwrap().scene, next);
        fs::remove_dir_all(folder).unwrap();
    }
}
