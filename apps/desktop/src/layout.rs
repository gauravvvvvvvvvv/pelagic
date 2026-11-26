//! Editor-only layout state, measured in device-independent pixels.
//! It never changes projects, camera transforms or simulation state.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LayoutSettings {
    pub sidebar_width: Option<f32>,
    pub outliner_height: Option<f32>,
    pub content_height: Option<f32>,
}
impl LayoutSettings {
    pub fn sanitized(mut self) -> Self {
        for value in [
            &mut self.sidebar_width,
            &mut self.outliner_height,
            &mut self.content_height,
        ] {
            *value = value.filter(|v| v.is_finite() && (100.0..=2000.0).contains(v));
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Splitter {
    Sidebar,
    Outliner,
    Content,
}
impl Splitter {
    pub fn id(self) -> usize {
        match self {
            Self::Sidebar => 282,
            Self::Outliner => 283,
            Self::Content => 284,
        }
    }
    pub fn from_id(id: usize) -> Option<Self> {
        match id {
            282 => Some(Self::Sidebar),
            283 => Some(Self::Outliner),
            284 => Some(Self::Content),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Sidebar => "Sidebar width",
            Self::Outliner => "Outliner height",
            Self::Content => "Content Browser height",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EditorLayout {
    pub viewport: [f32; 4],
    pub outliner: [f32; 4],
    pub details: [f32; 4],
    pub content: [f32; 4],
    pub right: f32,
    pub maximized: bool,
    width: f32,
    height: f32,
}
impl EditorLayout {
    pub fn new(
        width: f32,
        height: f32,
        settings: LayoutSettings,
        collapsed: bool,
        maximized: bool,
    ) -> Self {
        let settings = settings.sanitized();
        let defaults = if width < 1200.0 { 300.0 } else { 340.0 };
        let side = settings
            .sidebar_width
            .unwrap_or(defaults)
            .clamp(300.0, (width - 600.0).clamp(300.0, 520.0));
        let right = if maximized { width } else { width - side };
        let drawer = if collapsed || maximized {
            0.0
        } else {
            settings
                .content_height
                .unwrap_or((height * 0.27).clamp(156.0, 240.0))
                .clamp(156.0, (height - 324.0).max(156.0))
        };
        // Keep 292 DIP for Details: five property rows, category and an action.
        let outliner_h = settings
            .outliner_height
            .unwrap_or((height * 0.31).clamp(196.0, 320.0))
            .clamp(148.0, (height - 412.0).max(148.0));
        let content_y = height - 24.0 - drawer;
        Self {
            viewport: [4.0, 120.0, right - 8.0, content_y - 124.0],
            outliner: [
                right + 2.0,
                92.0,
                if maximized { 0.0 } else { side - 2.0 },
                outliner_h,
            ],
            details: [
                right + 2.0,
                96.0 + outliner_h,
                if maximized { 0.0 } else { side - 2.0 },
                height - 120.0 - outliner_h,
            ],
            content: [0.0, content_y, right, drawer],
            right,
            maximized,
            width,
            height,
        }
    }
    pub fn splitters(self) -> Vec<(Splitter, [f32; 4])> {
        if self.maximized {
            return vec![];
        }
        let mut items = vec![
            (
                Splitter::Sidebar,
                [self.right - 2.0, 92.0, 4.0, self.height - 116.0],
            ),
            (
                Splitter::Outliner,
                [
                    self.right + 2.0,
                    self.outliner[1] + self.outliner[3],
                    self.width - self.right - 2.0,
                    4.0,
                ],
            ),
        ];
        if self.content[3] > 0.0 {
            items.push((
                Splitter::Content,
                [0.0, self.content[1] - 4.0, self.right - 2.0, 4.0],
            ));
        }
        items
    }
    pub fn range(self, splitter: Splitter) -> (f32, f32, f32) {
        match splitter {
            Splitter::Sidebar => (
                self.width - self.right,
                300.0,
                (self.width - 600.0).clamp(300.0, 520.0),
            ),
            Splitter::Outliner => (self.outliner[3], 148.0, (self.height - 412.0).max(148.0)),
            Splitter::Content => (
                self.content[3].max(156.0),
                156.0,
                (self.height - 324.0).max(156.0),
            ),
        }
    }
    pub fn set_value(self, settings: &mut LayoutSettings, splitter: Splitter, value: f32) {
        if !value.is_finite() {
            return;
        }
        let (_, min, max) = self.range(splitter);
        let target = match splitter {
            Splitter::Sidebar => &mut settings.sidebar_width,
            Splitter::Outliner => &mut settings.outliner_height,
            Splitter::Content => &mut settings.content_height,
        };
        *target = Some(value.clamp(min, max));
    }
    pub fn drag(self, settings: &mut LayoutSettings, splitter: Splitter, point: [f32; 2]) {
        let value = match splitter {
            Splitter::Sidebar => self.width - point[0],
            Splitter::Outliner => point[1] - 94.0,
            Splitter::Content => self.height - 24.0 - point[1],
        };
        self.set_value(settings, splitter, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extreme_saved_sizes_keep_all_regions_usable() {
        for (w, h) in [(900.0, 620.0), (1100.0, 720.0), (1920.0, 990.0)] {
            for requested in [None, Some(100.0), Some(2000.0), Some(f32::NAN)] {
                let settings = LayoutSettings {
                    sidebar_width: requested,
                    outliner_height: requested,
                    content_height: requested,
                };
                for collapsed in [false, true] {
                    let layout = EditorLayout::new(w, h, settings, collapsed, false);
                    assert!(layout.viewport[2] >= 592.0 && layout.viewport[3] >= 160.0);
                    assert!(layout.details[3] >= 292.0 && layout.details[2] >= 298.0);
                    for area in [
                        layout.viewport,
                        layout.outliner,
                        layout.details,
                        layout.content,
                    ] {
                        assert!(area.iter().all(|x| x.is_finite()));
                        assert!(
                            area[0] >= 0.0 && area[1] >= 0.0 && area[2] >= 0.0 && area[3] >= 0.0
                        );
                        assert!(area[0] + area[2] <= w && area[1] + area[3] <= h - 24.0 + 0.01);
                    }
                    assert!(layout.viewport[0] + layout.viewport[2] < layout.outliner[0]);
                    assert!(layout.outliner[1] + layout.outliner[3] < layout.details[1]);
                    let full = EditorLayout::new(w, h, settings, collapsed, true);
                    assert!(full.splitters().is_empty());
                    assert_eq!(full.viewport[2], w - 8.0);
                    assert_eq!(full.content[3], 0.0);
                }
            }
        }
    }
}
