//! The top menu bar: the explorer toggle, the current query's name (with a
//! superscript unsaved-changes marker and an inline save button right after it),
//! the query options ("⋮") menu — which also holds the Base-table selector as a
//! submenu — and the Filter/Sort/Display builder toggles plus the run button.

use std::sync::{Arc, Mutex};

use eframe::egui;
use uuid::Uuid;

use crate::App;
use crate::button::Button;
use crate::icons::{self, MaterialIcon};
use crate::now_playing::menu_item;
use crate::page::{
    DELETE_RED, QueryAction, QueryPage, explorer_button, inline_rename_field, unsaved_marker_format,
};
use crate::query_def::Section;
use crate::{Rename, RenameSurface};

/// An item chosen from the query page's options ("⋮") menu.
enum PageMenu {
    Action(QueryAction),
    Base(String),
}

impl App {
    // One linear pass over the bar's widgets followed by the application of
    // their collected actions; splitting it up would just scatter the flags.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn render_menu_bar(&mut self, ui: &mut egui::Ui) {
        let panel_fill = ui.style().visuals.panel_fill;
        let current_id = self.current.query_id();
        let has_page = current_id.is_some();
        let name = self
            .current_page()
            .map_or(String::new(), |p| p.live.name.clone());
        let base_table = self
            .current_page()
            .map_or(String::new(), |p| p.live.definition.base.clone());
        let running = self
            .current_page()
            .is_some_and(|p| p.results.lock().unwrap().running);
        let unsaved = self.current_page().is_some_and(QueryPage::unsaved);
        let organizer_open = self.organizer.open;
        let builder_section = self.builder_section;
        let schema = Arc::clone(&self.schema);

        let mut toggle_organizer = false;
        let mut section_clicked = None;
        let mut base_choice = None;
        let mut run_now = false;
        let mut save_now = false;
        let mut begin_rename = false;
        let mut rename_commit = false;
        let mut rename_cancel = false;
        let mut want_rename = false;
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
                        ui.add_space(6.0);
                        (begin_rename, rename_commit, rename_cancel) =
                            draw_page_name(ui, &name, unsaved, rename, current_id);
                        // The save button sits immediately after the name, shown
                        // only while there are unsaved changes.
                        if unsaved && Button::icon(icons::SAVE).show(ui).clicked() {
                            save_now = true;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if has_page {
                            match draw_page_menu_button(ui, &base_table, &schema) {
                                Some(PageMenu::Base(table)) => base_choice = Some(table),
                                Some(PageMenu::Action(QueryAction::Rename)) => want_rename = true,
                                Some(PageMenu::Action(QueryAction::Delete)) => want_delete = true,
                                None => {}
                            }
                            section_clicked = draw_section_buttons(ui, builder_section);
                            ui.separator();
                            if Button::icon(icons::RUN)
                                .enabled(!running)
                                .spin(running)
                                .show(ui)
                                .clicked()
                            {
                                run_now = true;
                            }
                        }
                    });
                });
            });

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
        if let Some(table) = base_choice
            && let Some(page) = self.current_page_mut()
        {
            page.live.definition.base = table;
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
        let mut job = egui::text::LayoutJob::default();
        job.append(
            name,
            0.0,
            egui::TextFormat {
                font_id: egui::TextStyle::Body.resolve(ui.style()),
                color: ui.visuals().text_color(),
                ..Default::default()
            },
        );
        if unsaved {
            job.append(icons::UNSAVED.codepoint, 1.0, unsaved_marker_format());
        }
        let begin = ui
            .add(egui::Label::new(job).sense(egui::Sense::click()))
            .double_clicked();
        (begin, false, false)
    }
}

/// The page's "⋮" options button, opening a menu with the Base-table submenu and
/// the Rename/Delete actions. Returns the chosen item, if any.
fn draw_page_menu_button(
    ui: &mut egui::Ui,
    base_table: &str,
    schema: &Arc<Mutex<Option<String>>>,
) -> Option<PageMenu> {
    let dots = Button::icon(icons::MORE).show(ui);
    egui::Popup::menu(&dots)
        .align(egui::RectAlign::TOP_END)
        .show(|ui| {
            ui.set_width(150.0);
            let mut chosen = None;
            if let Some(table) = base_submenu(ui, base_table, schema) {
                chosen = Some(PageMenu::Base(table));
            }
            if menu_item(ui, icons::RENAME, "Rename", true, None).clicked() {
                chosen = Some(PageMenu::Action(QueryAction::Rename));
            }
            if menu_item(ui, icons::DELETE, "Delete", true, Some(DELETE_RED)).clicked() {
                chosen = Some(PageMenu::Action(QueryAction::Delete));
            }
            chosen
        })
        .and_then(|inner| inner.inner)
}

/// The "Base ▶" submenu listing every table in the schema. Returns the newly
/// chosen base table, if any.
fn base_submenu(
    ui: &mut egui::Ui,
    base_table: &str,
    schema: &Arc<Mutex<Option<String>>>,
) -> Option<String> {
    let label = format!("{}  Base", icons::BASE.codepoint);
    let (_, inner) = egui::containers::menu::SubMenuButton::new(label).ui(ui, |ui| {
        ui.set_min_width(140.0);
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
            if ui.selectable_label(table == base_table, &table).clicked() {
                choice = Some(table);
            }
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

/// Draws the Filter/Sort/Display toggle buttons in the `ui`'s layout direction.
/// Returns the clicked section, if any.
fn draw_section_buttons(ui: &mut egui::Ui, open: Option<Section>) -> Option<Section> {
    let mut clicked = None;
    let mut sections = [Section::Filter, Section::Sort, Section::Display];
    if ui.layout().main_dir() == egui::Direction::RightToLeft {
        sections.reverse();
    }
    for section in sections {
        if Button::icon(section_icon(section))
            .label(section.label())
            .active(open == Some(section))
            .show(ui)
            .clicked()
        {
            clicked = Some(section);
        }
    }
    clicked
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
