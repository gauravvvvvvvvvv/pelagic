//! Screen-reader (AccessKit / UI Automation) naming for the GPU shell.
//!
//! egui exposes every widget through AccessKit. Visible labels here start
//! with icon glyphs ("💾 Save", "🌊  Ocean environment") and tab titles are
//! padded with spaces, which screen readers would read aloud. `clean`
//! removes those decorations from each frame's tree update, and `name` gives
//! icon-only or unlabelled controls an explicit accessible name.
use egui::accesskit;

/// Sets the accessible name of a widget after egui has described it.
pub fn name(response: &egui::Response, label: &str) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_label(label));
}

/// Arrows, technical symbols, shapes, dingbats and emoji used as icons.
fn decorative(c: char) -> bool {
    c.is_whitespace() || matches!(c as u32, 0x2190..=0x2BFF | 0x1F000..=0x1FAFF | 0xFE0F | 0x200D)
}

/// The label without leading/trailing icon glyphs and padding. Labels that
/// are only decoration are left unchanged (they need `name` instead).
pub fn plain(label: &str) -> Option<&str> {
    let trimmed = label.trim_matches(decorative);
    (trimmed != label && !trimmed.is_empty()).then_some(trimmed)
}

/// True for non-empty text made only of icon glyphs and spacing.
pub fn decoration_only(text: &str) -> bool {
    !text.is_empty() && text.chars().all(decorative)
}

/// Strips icon decorations from every label in a tree update.
pub fn clean(update: &mut accesskit::TreeUpdate) {
    for (_, node) in &mut update.nodes {
        if let Some(label) = node.label().and_then(plain).map(str::to_owned) {
            node.set_label(label);
        }
        // Static text carries its words in the value, not the label.
        if node.role() == accesskit::Role::Label {
            if let Some(value) = node.value().and_then(plain).map(str::to_owned) {
                node.set_value(value);
            } else if node.value().is_some_and(decoration_only) {
                // Brand marks, badges and status dots: not read aloud.
                node.clear_value();
                node.set_hidden();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn icons_and_padding_are_removed_but_values_are_kept() {
        assert_eq!(plain("💾 Save"), Some("Save"));
        assert_eq!(plain("🌊  Ocean environment"), Some("Ocean environment"));
        assert_eq!(plain("  Viewport  "), Some("Viewport"));
        assert_eq!(plain("⏸ Pause"), Some("Pause"));
        assert_eq!(plain("◆ PELAGIC"), Some("PELAGIC"));
        assert_eq!(plain("⛰ 3D tilt"), Some("3D tilt"));
        // Already plain, numeric or icon-only labels are untouched.
        assert_eq!(plain("Save"), None);
        assert_eq!(plain("-8.00 m"), None);
        assert_eq!(plain("↶"), None);
        assert_eq!(plain("Asset library demo •"), None);
        assert!(decoration_only("◆") && decoration_only(" ● "));
        assert!(!decoration_only("Save") && !decoration_only(""));
    }
}
