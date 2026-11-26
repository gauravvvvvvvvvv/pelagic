//! Native editor chrome. Panel bounds are shared by painting, hit testing and resize.
use super::*;

const PANEL: u32 = 0x202024;
const INSET: u32 = 0x19191c;
const EDGE: u32 = 0x343438;
const BLUE: u32 = 0x73b5f4;

impl Painter {
    pub(super) fn workspace(&mut self, model: &Model, bottom: f32) {
        let width = self.width as f32 / self.scale;
        let layout = EditorLayout::new(
            width,
            bottom,
            model.layout,
            model.content_collapsed,
            model.viewport_maximized,
        );
        self.editor_layout = Some(layout);
        let session = model.session.as_ref().expect("workspace session");
        let scene = model.document.as_ref().map(|d| &d.scene);
        self.rect(0.0, 0.0, width, bottom, 0x101012);
        // Menu bar, document tab, then the main editor toolbar.
        self.rect(0.0, 0.0, width, 26.0, INSET);
        self.logo(8.0, 4.0, 26.0, 1.0, None);
        for (id, x, label) in [
            (236, 46.0, "File"),
            (237, 90.0, "Edit"),
            (238, 134.0, "Window"),
            (239, 200.0, "Tools"),
            (262, 246.0, "Build"),
            (264, 292.0, "Select"),
            (265, 350.0, "Actor"),
            (240, 398.0, "Help"),
        ] {
            self.editor_control(
                id,
                [
                    x,
                    1.0,
                    if id == 238 {
                        64.0
                    } else if id == 264 {
                        54.0
                    } else {
                        44.0
                    },
                    24.0,
                ],
                label,
                false,
                false,
                model,
            );
        }
        self.text(
            "PELAGIC  /  OCEAN ENGINE",
            width - 190.0,
            7.0,
            10.0,
            MUTED,
            false,
        );
        self.rect(0.0, 26.0, width, 28.0, 0x101012);
        self.rect(38.0, 28.0, 254.0, 26.0, PANEL);
        self.icon(0, 49.0, 34.0, BLUE);
        let name = format!(
            "{}{}",
            model.project.as_deref().unwrap_or("Untitled ocean"),
            if model.document.as_ref().is_some_and(|d| d.dirty()) {
                " *"
            } else {
                ""
            }
        );
        self.text(
            &self.fit_text(&name, 208.0, 11.5, false),
            72.0,
            34.0,
            11.5,
            TEXT,
            false,
        );
        self.rect(0.0, 54.0, width, 36.0, PANEL);
        self.editor_control(16, [8.0, 59.0, 62.0, 26.0], "Save", false, false, model);
        self.editor_control(17, [72.0, 59.0, 46.0, 26.0], "Undo", false, false, model);
        self.editor_control(18, [120.0, 59.0, 46.0, 26.0], "Redo", false, false, model);
        self.rect(174.0, 62.0, 1.0, 20.0, EDGE);
        self.editor_control(94, [184.0, 59.0, 70.0, 26.0], "+ Add", false, false, model);
        self.editor_control(
            243,
            [256.0, 59.0, 68.0, 26.0],
            "Import",
            false,
            false,
            model,
        );
        self.editor_control(
            92,
            [328.0, 59.0, 92.0, 26.0],
            "Experiments",
            model.inspector_page == 4,
            false,
            model,
        );
        self.rect(429.0, 62.0, 1.0, 20.0, EDGE);
        self.editor_control(
            11,
            [440.0, 59.0, 58.0, 26.0],
            if session.running { "Pause" } else { "Play" },
            session.running,
            false,
            model,
        );
        self.editor_control(12, [500.0, 59.0, 48.0, 26.0], "Step", false, false, model);
        self.editor_control(13, [550.0, 59.0, 48.0, 26.0], "Reset", false, false, model);
        self.editor_control(
            235,
            [width - 280.0, 59.0, 126.0, 26.0],
            session.backend_name(),
            false,
            false,
            model,
        );
        self.editor_control(
            7,
            [width - 145.0, 59.0, 133.0, 26.0],
            if model.dark_mode {
                "Light mode"
            } else {
                "Dark mode"
            },
            false,
            false,
            model,
        );
        self.rect(0.0, 90.0, width, 1.0, EDGE);

        // Viewport toolbar belongs to the viewport, not the application toolbar.
        self.rect(0.0, 92.0, layout.right, 28.0, INSET);
        let mode = [
            "Surface",
            "Underwater",
            "Section",
            "Results",
            "Sonar",
            "Atlas",
            "Mounted camera",
        ][model.viewport_mode.min(6) as usize];
        self.editor_control(244, [8.0, 94.0, 115.0, 23.0], mode, false, false, model);
        if matches!(model.viewport_mode, 0 | 1 | 6) {
            self.editor_control(
                57,
                [128.0, 94.0, 82.0, 23.0],
                if model.wireframe { "Wireframe" } else { "Lit" },
                model.wireframe,
                false,
                model,
            );
        } else {
            self.text("2D / Data view", 136.0, 100.0, 10.0, MUTED, false);
        }
        if matches!(model.viewport_mode, 0 | 1) {
            self.editor_control(
                285,
                [215.0, 94.0, 78.0, 23.0],
                model.transform_tool.label(),
                false,
                false,
                model,
            );
            self.editor_control(
                288,
                [300.0, 94.0, 70.0, 23.0],
                model.transform_snap.label(),
                model.transform_snap != crate::viewport_edit::Snap::Off,
                false,
                model,
            );
        }
        let quality = scene.map_or(0, |s| s.quality.min(2) as usize);
        self.editor_control(
            269,
            [layout.right - 220.0, 94.0, 90.0, 23.0],
            if model.viewport_maximized {
                "Restore"
            } else {
                "Maximize"
            },
            false,
            false,
            model,
        );
        self.editor_control(
            58,
            [layout.right - 123.0, 94.0, 115.0, 23.0],
            ["Scalability: Low", "Scalability: Med", "Scalability: High"][quality],
            false,
            false,
            model,
        );
        let area = layout.viewport;
        self.rect(area[0], area[1], area[2], area[3], 0x101012);
        match model.viewport_mode {
            2 => self.section(model, area),
            3 => self.results(model, area),
            4 => self.sonar(model, area),
            5 => self.atlas(model, area),
            _ => {
                self.viewport_area = Some(area);
                let frame = &session.frame;
                let fit = (area[2] / frame.width as f32).min(area[3] / frame.height as f32);
                let fw = frame.width as f32 * fit;
                let fh = frame.height as f32 * fit;
                let frame_rect = [
                    area[0] + (area[2] - fw) * 0.5,
                    area[1] + (area[3] - fh) * 0.5,
                    fw,
                    fh,
                ];
                self.viewport_camera = Some(session.editor_camera(frame_rect));
                self.filtered_frame(
                    frame,
                    ((area[0] + (area[2] - fw) * 0.5) * self.scale) as usize,
                    ((area[1] + (area[3] - fh) * 0.5) * self.scale) as usize,
                    (fw * self.scale) as usize,
                    (fh * self.scale) as usize,
                    fit * self.scale,
                );
                self.text(
                    "Perspective",
                    area[0] + 12.0,
                    area[1] + 12.0,
                    11.0,
                    TEXT,
                    false,
                );
                self.text(
                    "Click select   ·   1 Move   ·   2 Rotate Z   ·   3 Size   ·   F frame",
                    area[0] + 12.0,
                    area[1] + area[3] - 22.0,
                    10.0,
                    0xcfcfcf,
                    false,
                );
                // Axis indicator describes the world basis, not an editing gizmo.
                let p = [area[0] + area[2] - 35.0, area[1] + area[3] - 35.0];
                self.segment(p, [p[0] + 18.0, p[1]], 0xcb6767);
                self.segment(p, [p[0] - 12.0, p[1] + 10.0], 0x7dbd77);
                self.segment(p, [p[0], p[1] - 21.0], 0x73b5f4);
                self.text("Z", p[0] - 3.0, p[1] - 34.0, 9.0, 0x73b5f4, false);
                self.viewport_overlay(model);
            }
        }
        if !model.viewport_maximized {
            self.editor_outliner(model, layout.outliner);
            self.editor_details(model, layout.details);
        }
        if !model.content_collapsed && !model.viewport_maximized {
            self.editor_content(model, layout.content);
        }
        for (splitter, rect) in layout.splitters() {
            let focused = model.focus == Some(splitter.id()) || model.hover == Some(splitter.id());
            self.rect(
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                if focused { BLUE } else { 0x101012 },
            );
            self.accessible.push(AccessibleControl {
                id: splitter.id(),
                rect,
                label: splitter.label().into(),
                value: None,
                disabled: false,
            });
            self.hits.push(Hit {
                id: splitter.id(),
                rect,
            });
        }
        // Bottom toolbar: content, output log, current simulation and save state.
        self.rect(0.0, bottom - 24.0, width, 24.0, PANEL);
        self.rect(0.0, bottom - 24.0, width, 1.0, EDGE);
        self.editor_control(
            246,
            [5.0, bottom - 22.0, 108.0, 20.0],
            "Content Drawer",
            !model.content_collapsed && !model.viewport_maximized,
            false,
            model,
        );
        self.editor_control(
            245,
            [118.0, bottom - 22.0, 82.0, 20.0],
            "Output Log",
            model.content_tab == 1 && !model.content_collapsed && !model.viewport_maximized,
            false,
            model,
        );
        let status = if model.job_running {
            model.job_progress.clone()
        } else if !model.notice.is_empty() {
            model.notice.clone()
        } else {
            format!(
                "{}   {:.3} s   |   Tick {}",
                if session.running { "Playing" } else { "Paused" },
                session.world.time(),
                session.world.tick()
            )
        };
        self.text(
            &self.fit_text(&status, (width - 580.0).max(120.0), 10.0, false),
            220.0,
            bottom - 18.0,
            10.0,
            MUTED,
            false,
        );
        self.text(
            &format!(
                "{}×{}  /  {:.1} ms",
                session.frame.width,
                session.frame.height,
                model.last_frame_ms.max(session.render_ms)
            ),
            width - 350.0,
            bottom - 18.0,
            10.0,
            MUTED,
            false,
        );
        self.text(
            if model.document.as_ref().is_some_and(|d| d.dirty()) {
                "●  Unsaved"
            } else {
                "All saved"
            },
            width - 100.0,
            bottom - 18.0,
            10.0,
            TEXT,
            false,
        );
    }

    fn editor_control(
        &mut self,
        id: usize,
        rect: [f32; 4],
        label: &str,
        active: bool,
        outlined: bool,
        model: &Model,
    ) {
        let [x, y, w, h] = rect;
        let disabled = match id {
            17 => !model.document.as_ref().is_some_and(|d| d.can_undo()),
            418..=420 => model.selected_library.is_none(),
            18 => !model.document.as_ref().is_some_and(|d| d.can_redo()),
            _ => false,
        };
        let hover = if disabled { 0.0 } else { self.hover_amount[id] };
        let press = self.press_amount[id];
        let bg = if active { 0x213b54 } else { PANEL };
        let fill = blend(
            blend(self.theme_color(bg), self.theme_color(0x38383e), hover),
            self.theme_color(INSET),
            press,
        );
        if outlined {
            self.rounded_rect([x, y, w, h], 3.0, EDGE);
        }
        let edge = if outlined { 1.0 / self.scale } else { 0.0 };
        if outlined || active || hover > 0.0 || press > 0.0 {
            self.rounded_rect_raw(
                [x + edge, y + edge, w - edge * 2.0, h - edge * 2.0],
                3.0,
                fill,
            );
        }
        let color = if disabled {
            0x777777
        } else if id == 11 {
            0x94cf86
        } else {
            TEXT
        };
        let icon = matches!(id, 11 | 16);
        let shown = self.fit_text(label, w - if icon { 30.0 } else { 16.0 }, 11.0, false);
        if id == 11 {
            let yy = y + h * 0.5;
            for i in 0..7 {
                self.rect(
                    x + 8.0 + i as f32,
                    yy - (7 - i) as f32 * 0.6,
                    1.0,
                    (7 - i) as f32 * 1.2,
                    color,
                );
            }
        } else if id == 16 {
            self.rect(x + 7.0, y + 7.0, 12.0, 12.0, MUTED);
            self.rect(x + 10.0, y + 8.0, 6.0, 4.0, INSET);
            self.rect(x + 10.0, y + 15.0, 6.0, 4.0, INSET);
        }
        self.text(
            &shown,
            x + if icon { 24.0 } else { 8.0 },
            y + (h - 13.0) * 0.5,
            11.0,
            color,
            false,
        );
        if model.focus == Some(id) {
            self.rect(x + 2.0, y + h - 2.0, w - 4.0, 1.0, BLUE);
        }
        self.accessible.push(AccessibleControl {
            id,
            rect,
            label: label.into(),
            value: None,
            disabled,
        });
        if !disabled {
            self.hits.push(Hit { id, rect });
        }
    }

    fn editor_outliner(&mut self, model: &Model, area: [f32; 4]) {
        let [x, y, w, h] = area;
        self.outliner_area = Some(area);
        self.rect(x, y, w, h, PANEL);
        self.rect(x, y, w, 26.0, INSET);
        self.text("World Outliner", x + 10.0, y + 7.0, 11.5, TEXT, false);
        self.editor_control(
            247,
            [x + w - 52.0, y + 2.0, 23.0, 22.0],
            "↑",
            false,
            false,
            model,
        );
        self.editor_control(
            248,
            [x + w - 27.0, y + 2.0, 23.0, 22.0],
            "↓",
            false,
            false,
            model,
        );
        self.editor_text_input(
            254,
            [x + 8.0, y + 33.0, w - 16.0, 24.0],
            "Search actors",
            &model.outliner_filter,
            model,
        );
        self.rect(x, y + 64.0, w, 20.0, 0x29292d);
        self.text("Actor label", x + 28.0, y + 68.0, 10.0, MUTED, false);
        self.text("Type", x + w - 75.0, y + 68.0, 10.0, MUTED, false);
        let scene = &model.document.as_ref().expect("workspace document").scene;
        let mut rows: Vec<(usize, String, &str, bool, u8)> = vec![(
            14,
            "Ocean environment".into(),
            "Ocean",
            model.inspector_page == 0 && model.inspect_ocean,
            1,
        )];
        rows.push((
            255,
            "Seabed".into(),
            "Terrain",
            model.inspector_page == 0 && model.environment_tab == 3,
            0,
        ));
        rows.push((
            90,
            "Marine life".into(),
            "School",
            model.inspector_page == 2,
            3,
        ));
        rows.push((91, "Sensors".into(), "Sensor", model.inspector_page == 3, 4));
        if scene.vehicle {
            rows.push((
                300,
                "Reference vehicle".into(),
                if scene.reference_asset.is_some() {
                    "Mesh"
                } else {
                    "ROV"
                },
                model.inspector_page == 1 && model.selected_entity.is_none(),
                2,
            ));
        }
        for (i, e) in scene.entities.iter().enumerate() {
            rows.push((
                301 + i,
                e.name.clone(),
                if e.asset.is_some() {
                    "Mesh"
                } else {
                    ["ROV", "Vessel", "Drifter"][e.kind.min(2) as usize]
                },
                model.inspector_page == 1 && model.selected_entity == Some(i),
                2,
            ));
        }
        if scene.structure {
            rows.push((
                249,
                "Harbour structure".into(),
                "Mesh",
                model.inspect_structure && model.inspector_page == 0,
                0,
            ));
        }
        let query = model.outliner_filter.to_lowercase();
        rows.retain(|(_, name, kind, _, _)| {
            format!("{name} {kind}").to_lowercase().contains(&query)
        });
        let count = ((h - 106.0) / 21.0).floor().max(1.0) as usize;
        let scroll = model.outliner_scroll.min(rows.len().saturating_sub(count));
        for (index, (id, label, kind, selected, icon)) in
            rows.iter().skip(scroll).take(count).enumerate()
        {
            let yy = y + 85.0 + index as f32 * 21.0;
            let fill = if *selected {
                0x213b54
            } else if model.hover == Some(*id) {
                0x343438
            } else if index % 2 == 0 {
                PANEL
            } else {
                0x29292d
            };
            self.rect(x + 1.0, yy, w - 2.0, 21.0, fill);
            self.icon(
                *icon,
                x + 9.0,
                yy + 4.0,
                if *selected { BLUE } else { 0xc1a66a },
            );
            self.text(
                &self.fit_text(label, w - 108.0, 11.0, false),
                x + 29.0,
                yy + 4.0,
                11.0,
                TEXT,
                false,
            );
            self.text(kind, x + w - 75.0, yy + 4.0, 10.0, MUTED, false);
            self.accessible.push(AccessibleControl {
                id: *id,
                rect: [x + 1.0, yy, w - 2.0, 21.0],
                label: label.clone(),
                value: None,
                disabled: false,
            });
            self.hits.push(Hit {
                id: *id,
                rect: [x + 1.0, yy, w - 2.0, 21.0],
            });
        }
        self.text(
            &format!(
                "{} actors   |   {} shown",
                rows.len(),
                rows.len().min(count)
            ),
            x + 9.0,
            y + h - 16.0,
            10.0,
            MUTED,
            false,
        );
    }

    fn editor_text_input(
        &mut self,
        id: usize,
        rect: [f32; 4],
        label: &str,
        value: &str,
        model: &Model,
    ) {
        let [x, y, w, h] = rect;
        let focused = model.editing.as_ref().is_some_and(|e| e.0 == id);
        self.rounded_rect(rect, 3.0, if focused { BLUE } else { EDGE });
        self.rounded_rect([x + 1.0, y + 1.0, w - 2.0, h - 2.0], 2.0, INSET);
        let value = model
            .editing
            .as_ref()
            .filter(|e| e.0 == id)
            .map_or(value, |e| e.1.as_str());
        if focused && model.input_selected {
            self.rect(x + 5.0, y + 4.0, w - 10.0, h - 8.0, 0x213b54);
        }
        let shown = if value.is_empty() && !focused {
            label
        } else {
            value
        };
        self.text(
            &self.fit_text(shown, w - 14.0, 11.0, true),
            x + 7.0,
            y + 5.0,
            11.0,
            if value.is_empty() && !focused {
                MUTED
            } else {
                TEXT
            },
            false,
        );
        if focused && !model.input_selected {
            let prefix = value
                .get(..model.input_cursor.min(value.len()))
                .unwrap_or(value);
            let font = self.font.as_scaled(PxScale::from(11.0 * self.scale * 1.16));
            let advance = prefix
                .chars()
                .map(|c| font.h_advance(self.font.glyph_id(c)))
                .sum::<f32>()
                / self.scale;
            self.rect(
                (x + 7.0 + advance).min(x + w - 5.0),
                y + 5.0,
                1.0 / self.scale,
                h - 10.0,
                TEXT,
            );
        }
        self.accessible.push(AccessibleControl {
            id,
            rect,
            label: label.into(),
            value: Some(value.into()),
            disabled: false,
        });
        self.hits.push(Hit { id, rect });
    }

    fn editor_property(
        &mut self,
        id: usize,
        label: &str,
        value: f64,
        area: [f32; 3],
        model: &Model,
    ) {
        let [x, y, w] = area;
        self.rect(x, y, w, 29.0, PANEL);
        self.rect(x, y + 28.0, w, 1.0, 0x29292d);
        self.text(
            &self.fit_text(label, w * 0.57 - 14.0, 10.5, false),
            x + 10.0,
            y + 8.0,
            10.5,
            0xcfcfcf,
            false,
        );
        self.editor_text_input(
            id,
            [x + w * 0.57, y + 3.0, w * 0.43 - 10.0, 23.0],
            label,
            &value.to_string(),
            model,
        );
    }

    fn editor_details(&mut self, model: &Model, area: [f32; 4]) {
        let [x, y, w, h] = area;
        self.rect(x, y, w, h, PANEL);
        self.rect(x, y, w, 27.0, INSET);
        self.text("Details", x + 10.0, y + 7.0, 11.5, TEXT, false);
        if model.selection_empty {
            self.text(
                "Select an actor to edit its properties.",
                x + 10.,
                y + 45.,
                11.,
                MUTED,
                false,
            );
            return;
        }
        let scene = model
            .transform_preview
            .as_ref()
            .unwrap_or(&model.document.as_ref().expect("workspace document").scene);
        let reference = scene.reference_entity();
        let entity = model
            .selected_entity
            .and_then(|i| scene.entities.get(i))
            .unwrap_or(&reference);
        let title = match model.inspector_page {
            1 => entity.name.as_str(),
            2 => "Marine life",
            3 => "Mounted sensors",
            4 => "Experiments",
            5 => "Atlas / regional data",
            _ if model.inspect_structure => "Harbour structure",
            _ => "Ocean environment",
        };
        self.icon(
            if model.inspector_page == 1 { 2 } else { 1 },
            x + 11.0,
            y + 39.0,
            BLUE,
        );
        self.text(
            &self.fit_text(title, w - 50.0, 12.0, false),
            x + 35.0,
            y + 40.0,
            12.0,
            TEXT,
            false,
        );
        let mut fields: Vec<(usize, &str, f64)> = vec![];
        let mut actions: Vec<(usize, &str)> = vec![];
        let mut appearance = None;
        let mut notice: Option<&str> = None;
        let mut mesh_label = String::new();
        let mut material_label = String::new();
        let (tab_id, category) = match model.inspector_page {
            1 => (122, crate::library::ENTITY_TABS[model.tool_tab.min(5)]),
            2 => (
                171,
                if model.tool_tab == 0 {
                    "School / population"
                } else {
                    "School / behaviour"
                },
            ),
            3 => (
                173,
                [
                    "Sonar",
                    "Navigation / timing",
                    "Camera / mount",
                    "Motion noise",
                ][model.tool_tab.min(3)],
            ),
            4 => (101, "Run actions"),
            5 => (93, "Geographic reference"),
            _ if model.inspect_structure => (94, "Structure / geometry"),
            _ => (
                32,
                [
                    "Water settings",
                    "Sea state / waves",
                    "Current field",
                    "Terrain",
                    "Water profile",
                ][model.environment_tab.min(4)],
            ),
        };
        self.editor_control(
            tab_id,
            [x + 7.0, y + 66.0, w - 14.0, 25.0],
            category,
            true,
            false,
            model,
        );
        match model.inspector_page {
            1 => {
                let material = entity.material.and_then(|id| scene.library.material(id));
                fields = match model.tool_tab {
                    5 => material.map_or(vec![], |m| {
                        vec![
                            (423, "Base colour R (0–255)", m.base_color[0] as f64),
                            (424, "Base colour G (0–255)", m.base_color[1] as f64),
                            (425, "Base colour B (0–255)", m.base_color[2] as f64),
                            (426, "Roughness (0.02–1)", m.roughness),
                            (427, "Metallic (0–1)", m.metallic),
                        ]
                    }),
                    4 => vec![],
                    3 => vec![
                        (296, "Body size X (m)", entity.model.half.x * 2.),
                        (297, "Body size Y (m)", entity.model.half.y * 2.),
                        (298, "Body size Z (m)", entity.model.half.z * 2.),
                    ],
                    1 => vec![
                        (114, "Mass (kg)", entity.mass),
                        (115, "Displaced volume (m3)", entity.volume),
                        (119, "Thrust limit (N)", entity.model.thrust_limit),
                        (120, "Actuator response (s)", entity.model.response_seconds),
                    ],
                    2 => vec![
                        (
                            117,
                            "Target depth (m)",
                            entity.model.hold_depth.unwrap_or(scene.vehicle_depth),
                        ),
                        (
                            118,
                            "Target heading (degrees)",
                            entity.model.hold_heading.unwrap_or(0.0).to_degrees(),
                        ),
                        (116, "Body X command (N)", entity.force.x),
                    ],
                    _ => vec![
                        (110, "Position X (m)", entity.position.x),
                        (111, "Position Y (m)", entity.position.y),
                        (112, "Position Z (m)", entity.position.z),
                        (113, "Heading (degrees)", entity.heading),
                    ],
                };
                if model.tool_tab == 4 {
                    appearance = Some((
                        entity
                            .asset
                            .and_then(|id| scene.library.mesh(id))
                            .map(|a| a.mesh.clone()),
                        [
                            entity.model.half.x,
                            entity.model.half.y,
                            entity.model.half.z,
                        ],
                        material.cloned().unwrap_or_else(super::default_material),
                    ));
                    mesh_label = format!(
                        "Mesh: {}",
                        entity
                            .asset
                            .and_then(|id| scene.library.mesh(id))
                            .map_or("Procedural (built-in)", |a| a.name.as_str())
                    );
                    material_label = format!(
                        "Material: {}",
                        material.map_or("Default look", |m| m.name.as_str())
                    );
                } else if model.tool_tab == 5 {
                    if material.is_none() {
                        notice = Some("Assign a material on the Appearance tab to edit it.");
                    } else {
                        notice = Some("Shared by every actor using this material.");
                    }
                } else {
                    actions.push((124, "Entity actions"));
                    actions.push((289, "Frame selected"));
                }
            }
            2 => {
                fields = if model.tool_tab == 0 {
                    vec![
                        (130, "Fish count (0–256)", scene.school.count as f64),
                        (131, "Swim speed (m/s)", scene.school.speed),
                        (132, "School radius (m)", scene.school.radius),
                        (133, "Preferred depth (m)", -scene.school.center.z),
                    ]
                } else {
                    vec![
                        (135, "Separation (m)", scene.school.separation),
                        (136, "Vehicle avoidance (m)", scene.school.avoidance),
                        (134, "Reference fish size (m)", scene.school.size),
                        (137, "Passive tracers (0–512)", scene.particles as f64),
                    ]
                }
            }
            3 => {
                let c = &scene.sensors;
                fields = match model.tool_tab {
                    1 => vec![
                        (160, "Sample rate (Hz)", c.rate_hz),
                        (168, "Position noise sigma (m)", c.position_noise),
                        (169, "Delivery latency (s)", c.latency_seconds),
                    ],
                    2 => vec![
                        (164, "Mount Z (m)", c.mount.z),
                        (165, "Mount X (m)", c.mount.x),
                        (175, "Mount Y (m)", c.mount.y),
                        (167, "Vertical camera FOV (deg)", c.camera_fov),
                    ],
                    3 => vec![
                        (176, "Velocity sigma (m/s)", c.velocity_sigma),
                        (177, "Gyro sigma (rad/s)", c.gyro_sigma),
                        (178, "Acceleration sigma (m/s2)", c.acceleration_sigma),
                    ],
                    _ => vec![
                        (161, "Max range (m)", c.range),
                        (162, "Beams (1–128)", c.beams as f64),
                        (163, "Range noise sigma (m)", c.range_noise),
                        (166, "Tilt below forward (deg)", c.tilt_degrees),
                        (174, "Fan angle (degrees)", c.fan_degrees),
                    ],
                };
                actions.push((172, "View / export sensor"));
            }
            4 => {
                fields = vec![
                    (180, "Batch duration (s)", model.batch_seconds.max(1.0)),
                    (
                        109,
                        "Playback sample (zero-based)",
                        model.playback.unwrap_or(0) as f64,
                    ),
                ];
                actions.extend([
                    (
                        100,
                        if model.session.as_ref().is_some_and(|s| s.capture.is_some()) {
                            "Stop recording"
                        } else {
                            "Record live run"
                        },
                    ),
                    (102, "Previous sample"),
                    (103, "Next sample"),
                ]);
            }
            5 => {
                fields = vec![
                    (
                        150,
                        "Origin latitude (degrees)",
                        scene.provenance.origin_latitude,
                    ),
                    (
                        151,
                        "Origin longitude (degrees)",
                        scene.provenance.origin_longitude,
                    ),
                ];
                actions.extend([
                    (145, "Open NOAA Goa region"),
                    (146, "Generate synthetic region"),
                    (147, "Import current field"),
                    (148, "Import geographic grid"),
                ]);
            }
            _ if model.inspect_structure => {
                fields = vec![
                    (80, "Position X (m)", scene.structure_position[0]),
                    (81, "Position Y (m)", scene.structure_position[1]),
                    (82, "Position Z (m)", scene.structure_position[2]),
                    (83, "Uniform scale", scene.structure_scale),
                ]
            }
            _ => match model.environment_tab {
                1 => {
                    fields = vec![
                        (50, "Significant height Hs (m)", scene.wave_height),
                        (51, "Peak period Tp (s)", scene.wave_period),
                        (52, "Wave direction (degrees)", scene.wave_direction),
                        (53, "Directional spread (degrees)", scene.wave_spread),
                    ];
                    actions.push((59, "Wind sea presets"));
                }
                2 if scene.flow_grid.is_some() => actions.push((149, "Use simple current")),
                2 => {
                    fields = vec![
                        (42, "Surface current X (m/s)", scene.current[0]),
                        (76, "Surface current Y (m/s)", scene.current[1]),
                        (77, "Surface current Z (m/s)", scene.current[2]),
                        (74, "Depth shear (1/m)", scene.shear),
                        (75, "Tide period (s, 0 = steady)", scene.tide_period),
                    ]
                }
                3 => {
                    actions.extend([
                        (
                            33,
                            ["Flat seabed", "Coastal shelf", "Canyon", "Imported grid"]
                                [scene.terrain.min(3) as usize],
                        ),
                        (67, "Import heightmap"),
                        (
                            64,
                            ["Material / automatic", "Material / sand", "Material / rock"]
                                [scene.terrain_material.min(2) as usize],
                        ),
                    ]);
                    if model.importing {
                        actions.push((69, "Cancel import"));
                    }
                    if let Some(g) = &scene.grid {
                        fields.push((68, "Grid spacing (m)", g.spacing));
                    }
                }
                4 => {
                    fields = vec![
                        (70, "Surface temperature (C)", scene.temperature),
                        (71, "Deep temperature (C)", scene.deep_temperature),
                        (72, "Salinity (g/kg)", scene.salinity),
                        (73, "Thermocline depth (m)", scene.thermocline),
                    ]
                }
                _ => {
                    fields = vec![
                        (40, "Nominal depth (m)", scene.bottom_depth),
                        (41, "Reference density (kg/m3)", scene.density),
                        (43, "Optical attenuation (1/m)", scene.attenuation),
                        (54, "Sun elevation (degrees)", scene.sun_elevation),
                    ]
                }
            },
        }
        let mut yy = y + 99.0;
        for (id, label, value) in fields {
            self.editor_property(id, label, value, [x, yy, w], model);
            yy += 29.0;
        }
        if appearance.is_some() {
            actions.push((421, &mesh_label));
            actions.push((422, &material_label));
        }
        for (id, label) in actions {
            self.editor_control(
                id,
                [x + 8.0, yy + 5.0, w - 16.0, 24.0],
                label,
                false,
                true,
                model,
            );
            yy += 30.0;
        }
        if let Some(text) = notice {
            self.text(
                &self.fit_text(text, w - 20.0, 10.0, false),
                x + 10.0,
                yy + 6.0,
                10.0,
                MUTED,
                false,
            );
            yy += 22.0;
        }
        if let Some((mesh, stretch, material)) = appearance {
            let th = (y + h - yy - 20.0).min(150.0);
            if th >= 48.0 {
                self.thumbnail(
                    mesh.as_ref(),
                    stretch,
                    &material,
                    [x + 8.0, yy + 8.0, w - 16.0, th],
                );
                yy += th + 8.0;
            }
        }
        if yy + 30.0 < y + h {
            self.rect(x, yy + 12.0, w, 1.0, EDGE);
            self.text(
                "Edits reset the preview run.",
                x + 10.0,
                yy + 21.0,
                10.0,
                MUTED,
                false,
            );
        }
    }

    fn editor_content(&mut self, model: &Model, area: [f32; 4]) {
        let [x, y, w, h] = area;
        self.rect(x, y, w, h, PANEL);
        self.rect(x, y, w, 27.0, INSET);
        self.editor_control(
            241,
            [x + 4.0, y + 2.0, 132.0, 24.0],
            "Content Browser",
            model.content_tab == 0,
            false,
            model,
        );
        self.editor_control(
            242,
            [x + 139.0, y + 2.0, 95.0, 24.0],
            "Output Log",
            model.content_tab == 1,
            false,
            model,
        );
        self.rect(x, y, w, 1.0, EDGE);
        if model.content_tab == 1 {
            let session = model.session.as_ref().unwrap();
            let mut lines = vec![
                format!(
                    "[Engine]  Native Rust studio  |  {}",
                    session.backend_name()
                ),
                format!(
                    "[World]   {} bodies, {} fish, {} tracers  |  fixed step 60 Hz",
                    session.world.vehicles().len(),
                    model.document.as_ref().unwrap().scene.school.count,
                    model.document.as_ref().unwrap().scene.particles
                ),
            ];
            if !model.job_progress.is_empty() {
                lines.push(format!("[Task]    {}", model.job_progress));
            }
            lines.push(format!(
                "[Frame]   {} × {}  |  {:.1} ms render refresh",
                session.frame.width, session.frame.height, session.render_ms
            ));
            let capacity = ((h - 50.0) / 20.0).floor().max(1.0) as usize;
            let remaining = capacity.saturating_sub(lines.len());
            let notices: Vec<_> = model.messages.iter().rev().take(remaining).collect();
            for notice in notices.into_iter().rev() {
                lines.push(format!("[Status]  {notice}"));
            }
            if model.messages.back() != Some(&model.notice)
                && !model.notice.is_empty()
                && lines.len() < capacity
            {
                lines.push(format!("[Status]  {}", model.notice));
            }
            for (i, line) in lines.iter().enumerate() {
                self.text(
                    &self.fit_text(line, w - 24.0, 11.0, false),
                    x + 12.0,
                    y + 39.0 + i as f32 * 20.0,
                    11.0,
                    if i == 2 { BLUE } else { MUTED },
                    false,
                );
            }
            return;
        }
        self.rect(x, y + 27.0, w, 30.0, PANEL);
        self.editor_control(
            94 + 256,
            [x + 7.0, y + 30.0, 56.0, 24.0],
            "+ Add",
            false,
            false,
            model,
        );
        self.editor_control(
            243 + 128,
            [x + 66.0, y + 30.0, 64.0, 24.0],
            "Import",
            false,
            false,
            model,
        );
        self.editor_control(
            16 + 256,
            [x + 133.0, y + 30.0, 68.0, 24.0],
            "Save all",
            false,
            false,
            model,
        );
        for (id, label, left, width) in [
            (418, "Place", 208.0, 54.0),
            (419, "Assign", 265.0, 60.0),
            (420, "Remove", 328.0, 64.0),
        ] {
            self.editor_control(
                id,
                [x + left, y + 30.0, width, 24.0],
                label,
                false,
                false,
                model,
            );
        }
        if w > 560.0 {
            let selection = model
                .selected_library
                .map(|item| crate::library::name(&model.document.as_ref().unwrap().scene, item))
                .unwrap_or_else(|| "Content / Project library".into());
            self.text(
                &self.fit_text(&selection, w - 490.0, 10.5, false),
                x + 400.0,
                y + 37.0,
                10.5,
                MUTED,
                false,
            );
        }
        self.editor_control(
            260,
            [x + w - 70.0, y + 30.0, 28.0, 24.0],
            "‹",
            false,
            false,
            model,
        );
        self.editor_control(
            261,
            [x + w - 37.0, y + 30.0, 28.0, 24.0],
            "›",
            false,
            false,
            model,
        );
        self.rect(x, y + 58.0, 138.0, h - 58.0, INSET);
        let row = ((h - 66.0) / 5.0).clamp(18.0, 22.0);
        for (i, (label, id)) in [
            ("All assets", 250),
            ("Meshes", 251),
            ("Materials", 417),
            ("World data", 252),
            ("Marine life", 253),
        ]
        .iter()
        .enumerate()
        {
            let filter = match *id {
                417 => 4,
                id => id - 250,
            };
            self.editor_control(
                *id,
                [x + 5.0, y + 62.0 + i as f32 * row, 128.0, row - 1.0],
                label,
                model.content_filter == filter,
                false,
                model,
            );
        }
        let scene = &model.document.as_ref().unwrap().scene;
        type Thumb = (
            Option<std::sync::Arc<pelagic::assets::Mesh>>,
            [f64; 3],
            pelagic::assets::Material,
        );
        let mut assets: Vec<(usize, String, String, u8, Option<Thumb>)> = vec![];
        if matches!(model.content_filter, 0 | 1) {
            for (i, asset) in scene.library.meshes.iter().enumerate() {
                let item = crate::library::Item::Mesh(asset.id);
                let uses = crate::library::usage(scene, item);
                assets.push((
                    crate::library::MESH_CARD + i,
                    asset.name.clone(),
                    format!(
                        "Mesh · {} tris · {uses} use{}",
                        asset.mesh.triangles.len(),
                        if uses == 1 { "" } else { "s" }
                    ),
                    2,
                    Some((
                        Some(asset.mesh.clone()),
                        asset.proportions,
                        super::default_material(),
                    )),
                ));
            }
        }
        if matches!(model.content_filter, 0 | 4) {
            for (i, (item, material)) in crate::library::material_items(scene)
                .into_iter()
                .enumerate()
            {
                // Presets are offered under Materials only, so "All" stays
                // focused on what the project actually contains.
                if model.content_filter == 0 && matches!(item, crate::library::Item::Preset(_)) {
                    continue;
                }
                let kind = match item {
                    crate::library::Item::Preset(_) => "Built-in preset".to_string(),
                    _ => {
                        let uses = crate::library::usage(scene, item);
                        format!("Material · {uses} use{}", if uses == 1 { "" } else { "s" })
                    }
                };
                assets.push((
                    crate::library::MATERIAL_CARD + i,
                    material.name.clone(),
                    kind,
                    1,
                    Some((None, [1.0; 3], material)),
                ));
            }
        }
        if matches!(model.content_filter, 0 | 2) {
            assets.extend([
                (
                    14 + 256,
                    "Ocean environment".into(),
                    "Ocean field".into(),
                    1,
                    None,
                ),
                (255 + 128, "Seabed".into(), "Terrain".into(), 0, None),
                (
                    91 + 256,
                    "Mounted sensors".into(),
                    "Sensor rig".into(),
                    4,
                    None,
                ),
            ]);
        }
        if matches!(model.content_filter, 0 | 1) {
            if scene.vehicle {
                assets.push((
                    400,
                    "Reference vehicle".into(),
                    if scene.reference_asset.is_some() {
                        "Actor · library mesh".into()
                    } else {
                        "Actor · procedural ROV".into()
                    },
                    2,
                    None,
                ));
            }
            for (i, e) in scene.entities.iter().enumerate() {
                assets.push((
                    401 + i,
                    e.name.clone(),
                    if e.asset.is_some() {
                        "Actor · library mesh".into()
                    } else {
                        "Actor · procedural".into()
                    },
                    2,
                    None,
                ));
            }
            if scene.structure {
                assets.push((
                    249 + 128,
                    "Harbour structure".into(),
                    "Static geometry".into(),
                    0,
                    None,
                ));
            }
        }
        if matches!(model.content_filter, 0 | 3) {
            assets.push((
                90 + 256,
                format!("Fish school · {}", scene.school.count),
                "School configuration".into(),
                3,
                None,
            ));
        }
        let available = ((w - 158.0) / 102.0).floor().max(1.0) as usize;
        let page = model
            .content_page
            .min(assets.len().saturating_sub(1) / available);
        let selected_card = model.selected_library.and_then(|item| match item {
            crate::library::Item::Mesh(id) => scene
                .library
                .meshes
                .iter()
                .position(|m| m.id == id)
                .map(|i| crate::library::MESH_CARD + i),
            _ => crate::library::material_items(scene)
                .iter()
                .position(|(i, _)| *i == item)
                .map(|i| crate::library::MATERIAL_CARD + i),
        });
        for (index, (id, name, kind, icon, thumb)) in assets
            .iter()
            .skip(page * available)
            .take(available)
            .enumerate()
        {
            let xx = x + 149.0 + index as f32 * 102.0;
            let yy = y + 66.0;
            let ch = (h - 84.0).clamp(70.0, 119.0);
            let chosen = selected_card == Some(*id);
            if chosen {
                self.rounded_rect([xx - 2.0, yy - 2.0, 98.0, ch + 4.0], 3.0, BLUE);
            }
            self.rect(
                xx,
                yy,
                94.0,
                ch,
                if chosen {
                    0x213b54
                } else if model.hover == Some(*id) {
                    0x38383e
                } else {
                    0x29292d
                },
            );
            if let Some((mesh, stretch, material)) = thumb {
                let th = (ch - 40.0).clamp(30.0, 74.0);
                self.thumbnail(
                    mesh.as_ref(),
                    *stretch,
                    material,
                    [xx + 1.0, yy + 1.0, 92.0, th],
                );
                self.text(
                    &self.fit_text(name, 82.0, 10.5, false),
                    xx + 6.0,
                    yy + th + 7.0,
                    10.5,
                    TEXT,
                    false,
                );
                if th + 34.0 <= ch {
                    self.text(
                        &self.fit_text(kind, 82.0, 9.0, false),
                        xx + 6.0,
                        yy + th + 24.0,
                        9.0,
                        MUTED,
                        false,
                    );
                }
                let rect = [xx, yy, 94.0, ch];
                self.accessible.push(AccessibleControl {
                    id: *id,
                    rect,
                    // No value: a value would expose the card as an edit field.
                    label: format!("Select library item: {name}"),
                    value: None,
                    disabled: false,
                });
                self.hits.push(Hit { id: *id, rect });
                continue;
            }
            self.icon(
                *icon,
                xx + 39.0,
                yy + 14.0,
                match icon {
                    1 => BLUE,
                    3 => 0x7dbd77,
                    _ => 0xc1a66a,
                },
            );
            self.rect(
                xx,
                yy + 45.0,
                94.0,
                2.0,
                match icon {
                    1 => 0x47748c,
                    3 => 0x56844e,
                    _ => 0x8d7450,
                },
            );
            self.text(
                &self.fit_text(name, 82.0, 10.5, false),
                xx + 6.0,
                yy + 53.0,
                10.5,
                TEXT,
                false,
            );
            if ch > 90.0 {
                self.text(
                    &self.fit_text(kind, 82.0, 9.0, false),
                    xx + 6.0,
                    yy + 72.0,
                    9.0,
                    MUTED,
                    false,
                );
            }
            let rect = [xx, yy, 94.0, ch];
            self.accessible.push(AccessibleControl {
                id: *id,
                rect,
                label: format!("Inspect asset: {name}"),
                value: None,
                disabled: false,
            });
            self.hits.push(Hit { id: *id, rect });
        }
        self.text(
            &format!(
                "{} assets   |   Page {} / {}",
                assets.len(),
                page + 1,
                assets.len().div_ceil(available).max(1)
            ),
            x + 151.0,
            y + h - 14.0,
            9.0,
            MUTED,
            false,
        );
    }

    fn icon(&mut self, kind: u8, x: f32, y: f32, color: u32) {
        let paths: Vec<([f32; 2], [f32; 2])> = match kind {
            1 => (0..3)
                .flat_map(|row| {
                    (0..3).map(move |i| {
                        (
                            [
                                i as f32 * 5.0,
                                row as f32 * 4.0 + if i % 2 == 0 { 1.0 } else { 0.0 },
                            ],
                            [
                                (i + 1) as f32 * 5.0,
                                row as f32 * 4.0 + if i % 2 == 0 { 0.0 } else { 1.0 },
                            ],
                        )
                    })
                })
                .collect(),
            3 => vec![
                ([1.0, 6.0], [7.0, 2.0]),
                ([7.0, 2.0], [12.0, 6.0]),
                ([12.0, 6.0], [7.0, 10.0]),
                ([7.0, 10.0], [1.0, 6.0]),
                ([12.0, 6.0], [16.0, 2.0]),
                ([16.0, 2.0], [16.0, 10.0]),
                ([16.0, 10.0], [12.0, 6.0]),
            ],
            4 => vec![
                ([7.0, 1.0], [7.0, 12.0]),
                ([1.0, 4.0], [13.0, 4.0]),
                ([3.0, 11.0], [7.0, 4.0]),
                ([7.0, 4.0], [11.0, 11.0]),
                ([3.0, 11.0], [11.0, 11.0]),
            ],
            _ => vec![
                ([1.0, 4.0], [8.0, 1.0]),
                ([8.0, 1.0], [15.0, 4.0]),
                ([15.0, 4.0], [8.0, 7.0]),
                ([8.0, 7.0], [1.0, 4.0]),
                ([1.0, 4.0], [1.0, 12.0]),
                ([1.0, 12.0], [8.0, 15.0]),
                ([8.0, 15.0], [15.0, 12.0]),
                ([15.0, 12.0], [15.0, 4.0]),
                ([8.0, 7.0], [8.0, 15.0]),
            ],
        };
        for (a, b) in paths {
            self.segment([x + a[0], y + a[1]], [x + b[0], y + b[1]], color);
        }
    }
}
