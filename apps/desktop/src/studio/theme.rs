//! Pelagic editor theme: deep ocean-slate surfaces, one teal accent, clear
//! hierarchy (chrome < panels < cards < fields), comfortable spacing and
//! Geist text. Colour constants are shared by every panel.
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};
use std::sync::Arc;

/// App chrome: menu bar, toolbar, status bar.
pub const BG: Color32 = Color32::from_rgb(10, 15, 21);
/// Panel bodies.
pub const PANEL: Color32 = Color32::from_rgb(16, 24, 32);
/// Tab strips and panel headers.
pub const HEADER: Color32 = Color32::from_rgb(12, 19, 26);
/// Grouped property cards inside panels.
pub const CARD: Color32 = Color32::from_rgb(21, 31, 41);
/// Text fields and value boxes.
pub const FIELD: Color32 = Color32::from_rgb(9, 14, 19);
pub const HOVER: Color32 = Color32::from_rgb(31, 46, 59);
pub const BORDER: Color32 = Color32::from_rgb(30, 43, 55);
/// The one accent: interactive state, selection, primary actions.
pub const ACCENT: Color32 = Color32::from_rgb(45, 197, 210);
/// Text drawn on the accent colour.
pub const ON_ACCENT: Color32 = Color32::from_rgb(4, 28, 33);
pub const TEXT: Color32 = Color32::from_rgb(214, 223, 231);
pub const STRONG: Color32 = Color32::from_rgb(242, 247, 250);
pub const MUTED: Color32 = Color32::from_rgb(124, 140, 153);
pub const GOOD: Color32 = Color32::from_rgb(98, 210, 140);
pub const WARN: Color32 = Color32::from_rgb(242, 182, 92);
pub const BAD: Color32 = Color32::from_rgb(240, 113, 103);
pub const X_AXIS: Color32 = Color32::from_rgb(232, 86, 78);
pub const Y_AXIS: Color32 = Color32::from_rgb(118, 200, 72);
pub const Z_AXIS: Color32 = Color32::from_rgb(70, 140, 245);
/// Viewport selection boxes: warm, to stand out against blue water.
pub const SELECT: Color32 = Color32::from_rgb(255, 176, 64);

pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "geist".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/Geist.ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "geist".into());
    // Icon/symbol fallback from the system (not redistributed).
    let windows = std::env::var_os("WINDIR").map_or("C:\\Windows".into(), std::path::PathBuf::from);
    for (name, file) in [
        ("symbols", "seguisym.ttf"),
        ("emoji", "seguiemj.ttf"),
        ("ui", "segoeui.ttf"),
    ] {
        if let Ok(bytes) = std::fs::read(windows.join("Fonts").join(file)) {
            fonts
                .font_data
                .insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.into());
            }
        }
    }
    ctx.set_fonts(fonts);

    let mut v = Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = Color32::from_rgb(19, 28, 37);
    v.code_bg_color = FIELD;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = Color32::from_rgb(24, 104, 114);
    v.selection.stroke = Stroke::new(1.0, STRONG);
    v.hyperlink_color = ACCENT;
    v.override_text_color = Some(TEXT);
    v.striped = false;
    let r = CornerRadius::same(6);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.weak_bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = Color32::from_rgb(26, 38, 50);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(26, 38, 50);
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(52, 74, 92));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, STRONG);
    v.widgets.hovered.corner_radius = r;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = Color32::from_rgb(20, 60, 68);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(20, 60, 68);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, STRONG);
    v.widgets.active.corner_radius = r;
    v.widgets.open.bg_fill = Color32::from_rgb(24, 36, 48);
    v.widgets.open.weak_bg_fill = Color32::from_rgb(24, 36, 48);
    v.widgets.open.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.open.corner_radius = r;
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = v.clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size = egui::vec2(40.0, 24.0);
        style.spacing.window_margin = egui::Margin::same(10);
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.indent = 16.0;
        style.spacing.combo_width = 120.0;
        style.spacing.scroll.bar_width = 9.0;
        style.spacing.scroll.floating = true;
        for (text_style, size) in [
            (egui::TextStyle::Body, 13.5),
            (egui::TextStyle::Button, 13.5),
            (egui::TextStyle::Small, 11.5),
            (egui::TextStyle::Heading, 17.0),
            (egui::TextStyle::Monospace, 12.5),
        ] {
            if let Some(font) = style.text_styles.get_mut(&text_style) {
                font.size = size;
            }
        }
    });
}

/// Primary call-to-action button (accent fill, dark text).
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .color(ON_ACCENT)
            .strong(),
    )
    .fill(ACCENT)
    .corner_radius(CornerRadius::same(6))
}

/// A rounded group container for related toolbar controls.
pub fn group<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(Color32::from_rgb(16, 24, 32))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(4, 3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.horizontal(add).inner
        })
        .inner
}

/// Section heading inside a panel: small, uppercase, muted.
pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .small()
            .strong()
            .color(MUTED),
    );
}

/// A small status chip: coloured dot and label.
pub fn chip(ui: &mut egui::Ui, colour: Color32, text: &str) -> egui::Response {
    egui::Frame::new()
        .fill(Color32::from_rgb(18, 27, 36))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(egui::RichText::new("●").color(colour).small());
                ui.label(egui::RichText::new(text).small().color(TEXT));
            });
        })
        .response
}
