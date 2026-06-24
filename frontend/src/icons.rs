//! Semantic icon vocabulary.
//!
//! Maps the app's UI concepts (`SAVE`, `DISPLAY`, `BASE`, …) to concrete
//! Material Symbols glyphs from `egui_material_icons`. Call sites refer to the
//! concept, never the raw glyph, so an icon choice is made once here and reused
//! consistently everywhere — and re-skinning a concept is a one-line change.

use eframe::egui;
pub(crate) use egui_material_icons::MaterialIcon;
use egui_material_icons::icons as mi;

// Query sections.
/// The query's source table.
pub(crate) const BASE: MaterialIcon = mi::ICON_PSYCHIATRY;
/// An individual table, e.g. in the Base submenu's list of tables.
pub(crate) const TABLE: MaterialIcon = mi::ICON_TABLE;
pub(crate) const FILTER: MaterialIcon = mi::ICON_FILTER_ALT;
pub(crate) const SORT: MaterialIcon = mi::ICON_SWAP_VERT;
pub(crate) const DISPLAY: MaterialIcon = mi::ICON_KEY_VISUALIZER;
/// Full-querydown mode: the single-input raw-query editor and its toggle.
pub(crate) const QUERYDOWN: MaterialIcon = mi::ICON_CODE;

// Generic actions.
/// Overflow ("⋮") menu trigger.
pub(crate) const MORE: MaterialIcon = mi::ICON_MORE_VERT;
/// The explorer/sidebar toggle when the explorer is open (closes it).
pub(crate) const EXPLORER_CLOSE: MaterialIcon = mi::ICON_LEFT_PANEL_CLOSE;
/// The explorer/sidebar toggle when the explorer is closed (opens it).
pub(crate) const EXPLORER_OPEN: MaterialIcon = mi::ICON_LEFT_PANEL_OPEN;
/// The superscript asterisk marking a query with unsaved changes.
pub(crate) const UNSAVED: MaterialIcon = mi::ICON_EMERGENCY;
pub(crate) const ADD: MaterialIcon = mi::ICON_ADD;
pub(crate) const RENAME: MaterialIcon = mi::ICON_EDIT;
pub(crate) const EDIT: MaterialIcon = mi::ICON_EDIT;
/// Duplicate a query into a new copy.
pub(crate) const DUPLICATE: MaterialIcon = mi::ICON_CONTENT_COPY;
pub(crate) const DELETE: MaterialIcon = mi::ICON_DELETE;
/// Remove an item, or close the now-playing bar.
pub(crate) const CLOSE: MaterialIcon = mi::ICON_CLOSE;
pub(crate) const CLEAR: MaterialIcon = mi::ICON_BACKSPACE;
pub(crate) const SAVE: MaterialIcon = mi::ICON_SAVE;
/// "Reset to default" / revert an in-progress edit.
pub(crate) const RESET: MaterialIcon = mi::ICON_UNDO;
/// Revert a saved query's unsaved changes back to its last-saved version.
pub(crate) const REVERT: MaterialIcon = mi::ICON_UNDO;
/// (Re-)run the current query.
pub(crate) const RUN: MaterialIcon = mi::ICON_REFRESH;
/// Reload a list from the backend.
pub(crate) const REFRESH: MaterialIcon = mi::ICON_REFRESH;
/// Expanded disclosure arrow on a collapsible preset card.
pub(crate) const EXPAND_OPEN: MaterialIcon = mi::ICON_EXPAND_MORE;
/// Collapsed disclosure arrow on a collapsible preset card.
pub(crate) const EXPAND_CLOSED: MaterialIcon = mi::ICON_CHEVRON_RIGHT;

// Presets.
/// A saved preset, shown beside preset entries.
pub(crate) const PRESET: MaterialIcon = mi::ICON_APPROVAL;
/// The built-in "Shuffle" sorting preset, and its "Reshuffle" action.
pub(crate) const SHUFFLE: MaterialIcon = mi::ICON_SHUFFLE;
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

/// The egui font family that renders Material Symbols glyphs. Derived from an
/// icon so it always matches the active style (filled vs outline) selected by
/// the crate's feature flags, keeping that choice to the one line in Cargo.toml.
pub(crate) fn family() -> egui::FontFamily {
    BASE.font_family()
}

/// A [`egui::FontId`] for painting an icon glyph at `size`.
pub(crate) fn font_id(size: f32) -> egui::FontId {
    egui::FontId::new(size, family())
}
