//! Graphics settings for the 3D viewport (app preferences, not part of a
//! scene): quality preset, frame-rate cap, render scale and ambient sea
//! motion. Saved to `%LOCALAPPDATA%\Pelagic\graphics.json`.
use serde::{Deserialize, Serialize};

pub const PRESETS: [&str; 4] = ["Low", "Medium", "High", "Ultra"];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graphics {
    /// 0 Low, 1 Medium, 2 High, 3 Ultra.
    pub preset: u8,
    /// Wait for the display refresh (off = unlimited frame rate).
    pub vsync: bool,
    /// Viewport resolution relative to the screen (0.5-2.0).
    pub scale: f32,
    /// Keep the sea, clouds and caustics moving while the simulation is
    /// paused (visual only; bodies keep their paused pose).
    pub animate_paused: bool,
}

impl Graphics {
    /// Sensible defaults for the adapter: Ultra on a dedicated GPU, Medium
    /// on integrated graphics.
    pub fn for_adapter(discrete: bool) -> Self {
        Self {
            preset: if discrete { 3 } else { 1 },
            vsync: true,
            scale: 1.0,
            animate_paused: true,
        }
    }
    pub fn load(root: &std::path::Path, discrete: bool) -> Self {
        std::fs::read_to_string(root.join("graphics.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Self>(&t).ok())
            .map(Self::sanitized)
            .unwrap_or_else(|| Self::for_adapter(discrete))
    }
    pub fn save(&self, root: &std::path::Path) -> Result<(), String> {
        crate::project::atomic_json(
            &root.join("graphics.json"),
            &serde_json::to_value(self).map_err(|e| e.to_string())?,
        )
    }
    pub fn sanitized(self) -> Self {
        Self {
            preset: self.preset.min(3),
            scale: if self.scale.is_finite() {
                self.scale.clamp(0.5, 2.0)
            } else {
                1.0
            },
            ..self
        }
    }
    /// Most pixels the viewport renders (before the GPU's own limit).
    pub fn pixel_budget(&self) -> f32 {
        match self.preset {
            0 => 1_500_000.0,
            1 => 2_500_000.0,
            2 => 4_194_304.0,
            _ => 8_847_360.0,
        }
    }
    pub fn label(&self) -> String {
        format!(
            "Graphics: {}{}",
            PRESETS[self.preset as usize],
            if self.vsync { "" } else { " · uncapped" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_stay_in_the_selected_storage() {
        let root = std::env::temp_dir().join(format!("pelagic-graphics-{}", std::process::id()));
        let first = root.join("first");
        let second = root.join("second");
        let settings = Graphics {
            preset: 0,
            scale: 0.75,
            ..Graphics::for_adapter(true)
        };
        settings.save(&first).unwrap();
        assert_eq!(Graphics::load(&first, true), settings);
        assert_eq!(Graphics::load(&second, true), Graphics::for_adapter(true));
        assert!(!second.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn defaults_and_sanitising() {
        assert_eq!(Graphics::for_adapter(true).preset, 3);
        assert_eq!(Graphics::for_adapter(false).preset, 1);
        let g = Graphics {
            preset: 9,
            vsync: false,
            scale: 7.0,
            animate_paused: true,
        }
        .sanitized();
        assert_eq!((g.preset, g.scale), (3, 2.0));
        assert!(g.label().contains("Ultra") && g.label().contains("uncapped"));
        assert!(
            Graphics::for_adapter(true).pixel_budget()
                > Graphics::for_adapter(false).pixel_budget()
        );
    }
}
