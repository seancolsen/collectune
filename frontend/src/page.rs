//! The "pages" abstraction: the organizer is a page switcher, and everything in
//! the rest of the app renders the currently selected page. A query page is the
//! first (and currently only) page type; settings/playlist/artist pages will
//! follow the same shape.

use std::sync::{Arc, Mutex};

use eframe::egui;
use uuid::Uuid;

use crate::QueryState;
use crate::icons;
use crate::now_playing::menu_item;
use crate::rpc::Query;

/// Red used for the destructive "Delete" affordances (menu item + dialog button).
pub(crate) const DELETE_RED: egui::Color32 = egui::Color32::from_rgb(0xC0, 0x39, 0x2B);

/// An action chosen from a query's Rename/Delete menu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryAction {
    Rename,
    Delete,
}

/// Renders the shared Rename/Delete menu body. Used both for the sidebar row's
/// `⋮`/right-click menu and the query page's `⋮` menu, so the two stay identical.
/// Returns the chosen action, if any.
pub(crate) fn query_actions_menu(ui: &mut egui::Ui) -> Option<QueryAction> {
    ui.set_width(130.0);
    let mut action = None;
    if menu_item(ui, icons::RENAME, "Rename", true, None).clicked() {
        action = Some(QueryAction::Rename);
    }
    if menu_item(ui, icons::DELETE, "Delete", true, Some(DELETE_RED)).clicked() {
        action = Some(QueryAction::Delete);
    }
    action
}

/// The result of a frame of inline-rename editing.
#[derive(Default)]
pub(crate) struct RenameOutcome {
    pub(crate) commit: bool,
    pub(crate) cancel: bool,
}

/// Renders an inline single-line rename field into `buffer`. On the first frame
/// (`take_focus`) it grabs focus and selects all text. Pressing Enter or clicking
/// away commits; pressing Escape cancels. The `id` keeps the widget's state stable
/// across frames; `width` sizes the field.
pub(crate) fn inline_rename_field(
    ui: &mut egui::Ui,
    buffer: &mut String,
    take_focus: &mut bool,
    id: egui::Id,
    width: f32,
) -> RenameOutcome {
    let mut output = egui::TextEdit::singleline(buffer)
        .id(id)
        .desired_width(width)
        .show(ui);

    if *take_focus {
        output.response.request_focus();
        let end = buffer.chars().count();
        let range = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(end),
        );
        output.state.cursor.set_char_range(Some(range));
        output.state.store(ui.ctx(), output.response.id);
        *take_focus = false;
    }

    let mut outcome = RenameOutcome::default();
    // egui's TextEdit surrenders focus on Enter (→ commit) but ignores Escape, so
    // we detect Escape ourselves and release focus to dismiss the field.
    if output.response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome.cancel = true;
        ui.memory_mut(|m| m.surrender_focus(output.response.id));
    } else if output.response.lost_focus() {
        outcome.commit = true;
    }
    outcome
}

/// Which page the app is currently showing. A query page requires a concrete
/// query id; `Welcome` is the placeholder shown when no query is open (e.g.
/// before any query has been created).
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurrentPage {
    #[default]
    Welcome,
    Query(Uuid),
}

impl CurrentPage {
    /// The id of the open query, if a query page is currently showing.
    pub(crate) fn query_id(self) -> Option<Uuid> {
        match self {
            CurrentPage::Query(id) => Some(id),
            CurrentPage::Welcome => None,
        }
    }
}

/// Renders the explorer (organizer) toggle — the "☰" button shown at the
/// top-left of every page. Every page type renders it by calling this one
/// helper, which is what keeps the button identical (look and behaviour)
/// across page types. When `active` (the organizer is open) it gets a dark
/// filled background with a light glyph, mirroring the gear's active state.
/// Returns `true` when clicked.
pub(crate) fn explorer_button(ui: &mut egui::Ui, active: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if active {
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().text_color());
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let icon_color = if active {
            egui::Color32::WHITE
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::MENU.codepoint,
            icons::font_id(18.0),
            icon_color,
        );
    }
    resp.clicked()
}

/// A single query page: an editable `live` query plus the `saved` snapshot it was
/// last persisted from, and a cache of its results.
pub(crate) struct QueryPage {
    /// The editable working copy. Edits to the definition update this.
    pub(crate) live: Query,
    /// The last-saved version, or `None` if the query has never been persisted.
    pub(crate) saved: Option<Query>,
    /// Cached query results, shared with the async fetch task.
    pub(crate) results: Arc<Mutex<QueryState>>,
    /// Whether a result fetch has been kicked off for the current results cache.
    pub(crate) results_fetched: bool,
}

impl QueryPage {
    /// Builds a page from a persisted query (its live and saved versions match).
    pub(crate) fn persisted(query: Query) -> Self {
        Self {
            live: query.clone(),
            saved: Some(query),
            results: Arc::new(Mutex::new(QueryState::default())),
            results_fetched: false,
        }
    }

    /// Builds a brand-new ephemeral page that has never been saved.
    pub(crate) fn ephemeral(query: Query) -> Self {
        Self {
            live: query,
            saved: None,
            results: Arc::new(Mutex::new(QueryState::default())),
            results_fetched: false,
        }
    }

    /// Derived: a page is unsaved whenever its live version differs from the
    /// last-saved version (a never-saved page is always unsaved).
    pub(crate) fn unsaved(&self) -> bool {
        self.saved.as_ref() != Some(&self.live)
    }

    /// Whether this query exists in the backend database.
    pub(crate) fn is_persisted(&self) -> bool {
        self.saved.is_some()
    }
}
