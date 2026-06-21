//! The top menu bar: the explorer toggle, the current query's name (with a
//! superscript unsaved-changes marker and an inline save button right after it),
//! the query options ("⋮") menu — which also holds the Base-table selector as a
//! submenu — and the builder toggles plus the run button. The builder toggles are
//! the Filter/Sort/Display buttons in sectioned mode, or a single "Querydown"
//! toggle in full-querydown mode.

use std::sync::{Arc, Mutex};

use eframe::egui;
use uuid::Uuid;

use crate::App;
use crate::button::{Button, SplitButton};
use crate::icons::{self, MaterialIcon};
use crate::now_playing::menu_item;
use crate::page::{DELETE_RED, QueryAction, QueryPage, explorer_button, inline_rename_field};
use crate::query_def::Section;
use crate::{Rename, RenameSurface};

/// At or below this query-page width, the menu bar switches to its compact
/// layout: the Filter/Sort/Display buttons drop their text labels and the
/// run/filter separator is hidden.
const COMPACT_MENU_BAR_WIDTH: f32 = 500.0;

/// An item chosen from the query page's options ("⋮") menu.
enum PageMenu {
    Action(QueryAction),
    Base(String),
    /// Switch the query into full-querydown mode (from the Base submenu's
    /// "Full query" item or the "Convert to full query" action).
    ConvertToFull,
}

/// A choice from the Base submenu: a specific base table, or full-querydown mode.
enum BaseChoice {
    Table(String),
    Full,
}

impl App {
    // One linear pass over the bar's widgets followed by the application of
    // their collected actions; splitting it up would just scatter the flags.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn render_menu_bar(&mut self, ui: &mut egui::Ui) {
        let panel_fill = ui.style().visuals.panel_fill;
        // On a narrow query page, drop the section buttons' text labels (and the
        // run/filter separator) to keep the bar's controls from crowding.
        let compact = ui.available_width() <= COMPACT_MENU_BAR_WIDTH;
        let current_id = self.current.query_id();
        let has_page = current_id.is_some();
        let name = self
            .current_page()
            .map_or(String::new(), |p| p.live.name.clone());
        let base_table = self
            .current_page()
            .map_or(String::new(), |p| p.live.definition.base.clone());
        let full_mode = self
            .current_page()
            .is_some_and(|p| p.live.definition.is_full());
        let full_editor_open = self.full_editor_open;
        let running = self
            .current_page()
            .is_some_and(|p| p.results.lock().unwrap().running);
        let unsaved = self.current_page().is_some_and(QueryPage::unsaved);
        // "Revert changes" is offered only for a saved query with unsaved edits.
        let show_revert = unsaved && self.current_page().is_some_and(QueryPage::is_persisted);
        let organizer_open = self.organizer.open;
        let builder_section = self.builder_section;
        let schema = Arc::clone(&self.schema);

        let mut toggle_organizer = false;
        let mut section_clicked = None;
        let mut section_menu_anchor = None;
        let mut toggle_full_editor = false;
        let mut convert_to_full = false;
        let mut base_choice = None;
        let mut run_now = false;
        let mut save_now = false;
        let mut begin_rename = false;
        let mut rename_commit = false;
        let mut rename_cancel = false;
        let mut want_rename = false;
        let mut want_revert = false;
        let mut want_delete = false;
        let rename = &mut self.rename;

        egui::Panel::top("menu_bar")
            .exact_size(30.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::same(0)),
            )
            .show_inside(ui, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    if explorer_button(ui, organizer_open) {
                        toggle_organizer = true;
                    }
                    if has_page {
                        // Keep the name tucked close to the explorer toggle for
                        // density: zero the spacing egui would insert before the
                        // controls layout, leaving just this small explicit gap.
                        let item_spacing_x = ui.spacing().item_spacing.x;
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.add_space(3.0);
                        // Lay out the right-hand controls first so they claim their
                        // space; whatever's left in the middle then goes to the
                        // (truncating) query name and the conditional save button.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Restore normal inter-widget spacing inside the controls.
                            ui.spacing_mut().item_spacing.x = item_spacing_x;
                            ui.add_space(8.0);
                            match draw_page_menu_button(
                                ui,
                                &base_table,
                                full_mode,
                                show_revert,
                                &schema,
                            ) {
                                Some(PageMenu::Base(table)) => base_choice = Some(table),
                                Some(PageMenu::ConvertToFull) => convert_to_full = true,
                                Some(PageMenu::Action(QueryAction::Rename)) => want_rename = true,
                                Some(PageMenu::Action(QueryAction::Revert)) => want_revert = true,
                                Some(PageMenu::Action(QueryAction::Delete)) => want_delete = true,
                                None => {}
                            }
                            // Full mode replaces the Filter/Sort/Display section
                            // toggles with a single, menu-less "Querydown" toggle.
                            if full_mode {
                                toggle_full_editor =
                                    draw_querydown_button(ui, full_editor_open, compact);
                            } else {
                                let sections = draw_section_buttons(ui, builder_section, compact);
                                section_clicked = sections.clicked;
                                section_menu_anchor = sections.menu;
                            }
                            if !compact {
                                ui.separator();
                            }
                            if Button::icon(icons::RUN)
                                .enabled(!running)
                                .spin(running)
                                .show(ui)
                                .clicked()
                            {
                                run_now = true;
                            }
                            // The name + save button fill the remaining middle,
                            // laid out left-aligned.
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    // Reserve room for the save button (shown only
                                    // when there are unsaved changes) so the name
                                    // truncates before colliding with it.
                                    let save_reserve = if unsaved {
                                        crate::button::SIZE + ui.spacing().item_spacing.x
                                    } else {
                                        0.0
                                    };
                                    let name_avail = (ui.available_width() - save_reserve).max(0.0);
                                    (begin_rename, rename_commit, rename_cancel) = draw_page_name(
                                        ui, &name, unsaved, rename, current_id, name_avail,
                                    );
                                    if unsaved && Button::icon(icons::SAVE).show(ui).clicked() {
                                        save_now = true;
                                    }
                                },
                            );
                        });
                    }
                });
            });

        // Hand the active section's menu trigger to the builder panel (rendered
        // just below this bar, in the same frame) so it can anchor that section's
        // options popup to the toolbar button.
        self.section_menu_anchor = section_menu_anchor;

        if toggle_organizer {
            self.organizer.open = !self.organizer.open;
        }
        // The section buttons are toggles: clicking the open one closes the builder.
        if let Some(section) = section_clicked {
            self.builder_section = if self.builder_section == Some(section) {
                None
            } else {
                Some(section)
            };
        }
        // The "Querydown" button is a toggle for the full-query editor panel.
        if toggle_full_editor {
            self.full_editor_open = !self.full_editor_open;
        }
        if convert_to_full {
            self.convert_current_to_full();
        }
        if let Some(table) = base_choice
            && self
                .current_page()
                .is_some_and(|p| p.live.definition.base != table)
        {
            // Changing the base invalidates the existing filter/sort/display
            // (presets are scoped to a base table and custom code references its
            // columns), so reset them, seeding the new base's default presets.
            let definition = self.definition_for_base(table);
            if let Some(page) = self.current_page_mut() {
                page.live.definition = definition;
            }
            run_now = true;
        }
        if run_now {
            self.run_query(ui.ctx());
        }
        if save_now {
            self.save_current();
        }
        // Commit/cancel an in-progress rename before starting a new one.
        if rename_commit {
            self.commit_rename();
        }
        if rename_cancel {
            self.cancel_rename();
        }
        if let Some(id) = current_id {
            if begin_rename || want_rename {
                self.begin_rename(id, RenameSurface::Page);
            }
            if want_revert {
                self.revert_query(id);
            }
            if want_delete {
                self.request_delete(id);
            }
        }
    }
}

/// Renders the page's query name: an inline rename field when this query is being
/// renamed from the page, otherwise a double-clickable label (double-click begins
/// a rename) carrying a superscript unsaved-changes marker. Returns
/// `(begin_rename, rename_commit, rename_cancel)`.
fn draw_page_name(
    ui: &mut egui::Ui,
    name: &str,
    unsaved: bool,
    rename: &mut Option<Rename>,
    current_id: Option<Uuid>,
    max_width: f32,
) -> (bool, bool, bool) {
    let editing = rename
        .as_mut()
        .filter(|r| r.surface == RenameSurface::Page && Some(r.id) == current_id);
    if let Some(state) = editing {
        let res = inline_rename_field(
            ui,
            &mut state.buffer,
            &mut state.take_focus,
            egui::Id::new("page-rename"),
            160.0,
        );
        (false, res.commit, res.cancel)
    } else {
        // Lay out the name truncated to the available width (reserving room for
        // the unsaved marker so it survives truncation), then render it via a
        // `Label` for the double-click-to-rename interaction. We hand `Label` a
        // pre-laid galley because it otherwise overrides a `LayoutJob`'s wrap
        // with the ui's full available width.
        let font_id = egui::TextStyle::Body.resolve(ui.style());
        let color = ui.visuals().text_color();
        let (name_galley, marker_galley) =
            crate::page::layout_query_name(ui, name, unsaved, font_id, color, max_width);
        let name_resp = ui.add(egui::Label::new(name_galley).sense(egui::Sense::click()));
        if let Some(marker) = marker_galley {
            // Allocate the marker's space so the save button follows it, and paint
            // it top-aligned with the name for the raised superscript look.
            let (mrect, _) = ui.allocate_exact_size(
                egui::vec2(marker.size().x + crate::page::MARKER_GAP, marker.size().y),
                egui::Sense::hover(),
            );
            ui.painter().galley(
                egui::pos2(mrect.left() + crate::page::MARKER_GAP, name_resp.rect.top()),
                marker,
                color,
            );
        }
        (name_resp.double_clicked(), false, false)
    }
}

/// The page's "⋮" options button, opening a menu with the Base-table submenu and
/// the Rename/Revert/Delete actions. "Revert changes" is shown only when
/// `show_revert` (a saved query with unsaved edits). Returns the chosen item, if any.
fn draw_page_menu_button(
    ui: &mut egui::Ui,
    base_table: &str,
    full_mode: bool,
    show_revert: bool,
    schema: &Arc<Mutex<Option<String>>>,
) -> Option<PageMenu> {
    let dots = Button::icon(icons::MORE).show(ui);
    egui::Popup::menu(&dots)
        .align(egui::RectAlign::TOP_END)
        .show(|ui| {
            ui.set_width(210.0);
            let mut chosen = None;
            match base_submenu(ui, base_table, full_mode, schema) {
                Some(BaseChoice::Table(table)) => chosen = Some(PageMenu::Base(table)),
                Some(BaseChoice::Full) => chosen = Some(PageMenu::ConvertToFull),
                None => {}
            }
            // Convert-to-full is a no-op once already in full mode, so hide it then.
            if !full_mode
                && menu_item(ui, icons::QUERYDOWN, "Convert to full query", true, None).clicked()
            {
                chosen = Some(PageMenu::ConvertToFull);
            }
            if menu_item(ui, icons::RENAME, "Rename", true, None).clicked() {
                chosen = Some(PageMenu::Action(QueryAction::Rename));
            }
            if show_revert && menu_item(ui, icons::REVERT, "Revert changes", true, None).clicked() {
                chosen = Some(PageMenu::Action(QueryAction::Revert));
            }
            if menu_item(ui, icons::DELETE, "Delete", true, Some(DELETE_RED)).clicked() {
                chosen = Some(PageMenu::Action(QueryAction::Delete));
            }
            chosen
        })
        .and_then(|inner| inner.inner)
}

/// The "Base ▶" submenu listing every table in the schema, followed (below a
/// separator) by the "Full query" option that switches the query into
/// full-querydown mode. The current selection is highlighted: a table in
/// sectioned mode, or the full-query option in `full_mode`. Returns the chosen
/// item, if any.
fn base_submenu(
    ui: &mut egui::Ui,
    base_table: &str,
    full_mode: bool,
    schema: &Arc<Mutex<Option<String>>>,
) -> Option<BaseChoice> {
    let label = format!("{}  Base", icons::BASE.codepoint);
    let (_, inner) = egui::containers::menu::SubMenuButton::new(label).ui(ui, |ui| {
        ui.set_min_width(170.0);
        let tables = schema
            .lock()
            .unwrap()
            .as_deref()
            .map(table_names)
            .unwrap_or_default();
        if tables.is_empty() {
            ui.weak("Loading schema…");
            return None;
        }
        let mut choice = None;
        for table in tables {
            let label = format!("{}  {table}", icons::TABLE.codepoint);
            // No table is the active base while in full mode.
            if ui
                .selectable_label(!full_mode && table == base_table, label)
                .clicked()
            {
                choice = Some(BaseChoice::Table(table));
            }
        }
        ui.separator();
        let label = format!("{}  Full query", icons::QUERYDOWN.codepoint);
        if ui.selectable_label(full_mode, label).clicked() {
            choice = Some(BaseChoice::Full);
        }
        choice
    });
    inner.and_then(|i| i.inner)
}

/// The icon for a section's button.
fn section_icon(section: Section) -> MaterialIcon {
    match section {
        Section::Filter => icons::FILTER,
        Section::Sort => icons::SORT,
        Section::Display => icons::DISPLAY,
    }
}

/// The outcome of drawing the Filter/Sort/Display toggle buttons.
struct SectionButtons {
    /// The section whose main area was clicked (toggles the builder), if any.
    clicked: Option<Section>,
    /// The active section's embedded "⋮" menu trigger, to anchor its options
    /// popup. At most one section is active, so at most one trigger exists.
    menu: Option<egui::Response>,
}

/// Draws the Filter/Sort/Display toggle buttons in the `ui`'s layout direction.
/// Each is a [`SplitButton`]: its main area toggles the builder section, and
/// while active it shows a menu trigger for that section's options. When
/// `compact`, the buttons drop their text labels (keeping their icons and, while
/// active, their menu triggers) to save room on a narrow bar.
fn draw_section_buttons(ui: &mut egui::Ui, open: Option<Section>, compact: bool) -> SectionButtons {
    let mut out = SectionButtons {
        clicked: None,
        menu: None,
    };
    let mut sections = [Section::Filter, Section::Sort, Section::Display];
    if ui.layout().main_dir() == egui::Direction::RightToLeft {
        sections.reverse();
    }
    for section in sections {
        let resp = SplitButton::new(section_icon(section), section.label())
            .active(open == Some(section))
            .show_label(!compact)
            .show(ui);
        if resp.main.clicked() {
            out.clicked = Some(section);
        }
        if let Some(menu) = resp.menu {
            out.menu = Some(menu);
        }
    }
    out
}

/// Draws the full-querydown mode's single "Querydown" toggle button (a menu-less
/// [`SplitButton`]), which opens/closes the full-query editor panel. When
/// `compact`, drops the text label to save room. Returns `true` when clicked.
fn draw_querydown_button(ui: &mut egui::Ui, open: bool, compact: bool) -> bool {
    SplitButton::new(icons::QUERYDOWN, "Querydown")
        .active(open)
        .show_label(!compact)
        .show_menu(false)
        .show(ui)
        .main
        .clicked()
}

/// The table names from the Querydown schema JSON.
fn table_names(schema_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(schema_json) else {
        return Vec::new();
    };
    let Some(tables) = value.get("tables").and_then(|t| t.as_array()) else {
        return Vec::new();
    };
    tables
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .map(str::to_string)
        .collect()
}
