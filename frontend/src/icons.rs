//! Semantic icon vocabulary.
//!
//! Maps the app's UI concepts (`SAVE`, `DISPLAY`, `BASE`, …) to concrete
//! Material Symbols glyphs from `egui_material_icons`. Call sites refer to the
//! concept, never the raw glyph, so an icon choice is made once here and reused
//! consistently everywhere — and re-skinning a concept is a one-line change.

use eframe::egui;
use egui_material_icons::icons as mi;
pub(crate) use egui_material_icons::MaterialIcon;

// Query sections.
/// The query's source table.
pub(crate) const BASE: MaterialIcon = mi::ICON_TABLE_CHART;
pub(crate) const FILTER: MaterialIcon = mi::ICON_FILTER_ALT;
pub(crate) const SORT: MaterialIcon = mi::ICON_SWAP_VERT;
pub(crate) const DISPLAY: MaterialIcon = mi::ICON_KEY_VISUALIZER;

// Generic actions.
/// Overflow ("⋮") menu trigger.
pub(crate) const MORE: MaterialIcon = mi::ICON_MORE_VERT;
/// The explorer/sidebar (hamburger) toggle.
pub(crate) const MENU: MaterialIcon = mi::ICON_MENU;
/// A dropdown caret appended after a button's label.
pub(crate) const EXPAND: MaterialIcon = mi::ICON_EXPAND_MORE;
pub(crate) const ADD: MaterialIcon = mi::ICON_ADD;
pub(crate) const RENAME: MaterialIcon = mi::ICON_EDIT;
pub(crate) const EDIT: MaterialIcon = mi::ICON_EDIT;
pub(crate) const DELETE: MaterialIcon = mi::ICON_DELETE;
/// Remove an item, or close the now-playing bar.
pub(crate) const CLOSE: MaterialIcon = mi::ICON_CLOSE;
pub(crate) const CLEAR: MaterialIcon = mi::ICON_BACKSPACE;
pub(crate) const SAVE: MaterialIcon = mi::ICON_SAVE;
/// "Reset to default" / revert an in-progress edit.
pub(crate) const RESET: MaterialIcon = mi::ICON_UNDO;
/// (Re-)run the current query.
pub(crate) const RUN: MaterialIcon = mi::ICON_REFRESH;
/// Reload a list from the backend.
pub(crate) const REFRESH: MaterialIcon = mi::ICON_REFRESH;
/// A section's gear/options menu.
pub(crate) const OPTIONS: MaterialIcon = mi::ICON_SETTINGS;
/// The query-builder (wrench) toggle, and editing a preset's definition.
pub(crate) const BUILDER: MaterialIcon = mi::ICON_BUILD;

// Presets.
/// A saved preset, shown beside preset entries.
pub(crate) const PRESET: MaterialIcon = mi::ICON_APPROVAL;
/// Manage the whole preset library ("toolbox").
pub(crate) const MANAGE_PRESETS: MaterialIcon = mi::ICON_HANDYMAN;
/// Convert/merge a preset into custom text.
pub(crate) const CONVERT: MaterialIcon = mi::ICON_SWAP_HORIZ;

// Playback.
pub(crate) const PLAY: MaterialIcon = mi::ICON_PLAY_ARROW;
pub(crate) const PAUSE: MaterialIcon = mi::ICON_PAUSE;
pub(crate) const NEXT: MaterialIcon = mi::ICON_SKIP_NEXT;
/// Scroll the results to the now-playing track.
pub(crate) const LOCATE: MaterialIcon = mi::ICON_MY_LOCATION;

/// The egui font family that renders Material Symbols outline glyphs.
pub(crate) fn family() -> egui::FontFamily {
    egui::FontFamily::Name(egui_material_icons::FONT_FAMILY_OUTLINED.into())
}

/// A [`egui::FontId`] for painting an icon glyph at `size`.
pub(crate) fn font_id(size: f32) -> egui::FontId {
    egui::FontId::new(size, family())
}
