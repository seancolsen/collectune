//! The top menu bar (organizer toggle, current query name, save/run, and the
//! query-builder entry points). On wide screens the Base/Filter/Sort/Display
//! buttons are inlined into the bar; on narrow screens they live in a second-
//! row toolbar toggled by the wrench button.

use std::sync::{Arc, Mutex};

use eframe::egui;
use uuid::Uuid;

use crate::page::{
    QueryAction, QueryPage, explorer_button, inline_rename_field, query_actions_menu,
};
use crate::query_def::Section;
use crate::{ACCENT_BLUE, App, Rename, RenameSurface};

/// At or above this viewport width the Base/Filter/Sort/Display buttons are
/// inlined into the top toolbar; below it they move to a second-row toolbar
/// that the wrench button toggles.
const INLINE_SECTIONS_MIN_WIDTH: f32 = 850.0;

/// Light-blue background of the active (open) section button.
const ACTIVE_SECTION_BG: egui::Color32 = egui::Color32::from_rgb(0xBB, 0xD9, 0xFB);

impl App {
    // One linear pass over the bar's widgets followed by the application of
    // their collected actions; splitting it up would just scatter the flags.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        let panel_fill = ctx.style().visuals.panel_fill;
        let inline_sections = ctx.viewport_rect().width() >= INLINE_SECTIONS_MIN_WIDTH;
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
        let builder_open = self.builder_section.is_some();
        let builder_section = self.builder_section;
        let schema = Arc::clone(&self.schema);

        let mut toggle_organizer = false;
        let mut toggle_builder = false;
        let mut section_clicked = None;
        let mut base_choice = None;
        let mut run_now = false;
        let mut save_now = false;
        let mut begin_rename = false;
        let mut rename_commit = false;
        let mut rename_cancel = false;
        let mut menu_action = None;
        let rename = &mut self.rename;

        egui::TopBottomPanel::top("menu_bar")
            .exact_height(30.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    if explorer_button(ui, organizer_open) {
                        toggle_organizer = true;
                    }
                    if has_page {
                        ui.add_space(6.0);
                        (begin_rename, rename_commit, rename_cancel) =
                            draw_page_name(ui, &name, rename, current_id);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if has_page && let Some(a) = draw_page_menu_button(ui) {
                            menu_action = Some(a);
                        }
                        if has_page {
                            if inline_sections {
                                section_clicked =
                                    draw_section_buttons(ui, builder_section).or(section_clicked);
                                base_choice = draw_base_button(ui, &base_table, &schema);
                                ui.separator();
                            } else if wrench_button(ui, builder_open).clicked() {
                                toggle_builder = true;
                            }
                        }
                        (run_now, save_now) =
                            paint_run_save(ui, has_page && !running, running, unsaved);
                    });
                });
            });

        // The narrow-screen second-row toolbar holding the section buttons,
        // shown only while the builder is open.
        if has_page && !inline_sections && builder_open {
            egui::TopBottomPanel::top("section_bar")
                .exact_height(30.0)
                .show_separator_line(false)
                .show(ctx, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add_space(4.0);
                        base_choice = draw_base_button(ui, &base_table, &schema);
                        section_clicked =
                            draw_section_buttons(ui, builder_section).or(section_clicked);
                    });
                });
        }

        if toggle_organizer {
            self.organizer.open = !self.organizer.open;
        }
        if toggle_builder {
            self.builder_section = if builder_open {
                None
            } else {
                Some(self.last_builder_section)
            };
        }
        if let Some(section) = section_clicked {
            // Wide screens: the buttons are toggles (click again to close).
            // Narrow screens: they're tabs — the wrench is the only way out.
            self.builder_section = if inline_sections && self.builder_section == Some(section) {
                None
            } else {
                Some(section)
            };
            if let Some(section) = self.builder_section {
                self.last_builder_section = section;
            }
        }
        if let Some(table) = base_choice
            && let Some(page) = self.current_page_mut()
        {
            page.live.definition.base = table;
        }
        if run_now {
            self.run_query(ctx);
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
            if begin_rename || menu_action == Some(QueryAction::Rename) {
                self.begin_rename(id, RenameSurface::Page);
            }
            if menu_action == Some(QueryAction::Delete) {
                self.request_delete(id);
            }
        }
    }
}

/// Renders the page's query name: an inline rename field when this query is being
/// renamed from the page, otherwise a double-clickable label (double-click begins
/// a rename). Returns `(begin_rename, rename_commit, rename_cancel)`.
fn draw_page_name(
    ui: &mut egui::Ui,
    name: &str,
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
        let begin = ui
            .add(egui::Label::new(name).sense(egui::Sense::click()))
            .double_clicked();
        (begin, false, false)
    }
}

/// The page's "⋮" actions button in the menu bar, opening the shared Rename/Delete
/// menu for the current query. Returns the chosen action, if any.
fn draw_page_menu_button(ui: &mut egui::Ui) -> Option<QueryAction> {
    let dots = ui.add(
        egui::Button::new(egui::RichText::new(egui_phosphor::bold::DOTS_THREE_VERTICAL).size(18.0))
            .frame(false),
    );
    let mut action = None;
    if let Some(inner) = egui::Popup::menu(&dots)
        .align(egui::RectAlign::TOP_END)
        .show(query_actions_menu)
        && inner.inner.is_some()
    {
        action = inner.inner;
    }
    action
}

/// The icon (and whether it's from the fill variant) for a section's button.
fn section_icon(section: Section) -> (&'static str, bool) {
    match section {
        Section::Filter => (egui_phosphor::fill::FUNNEL, true),
        Section::Sort => (egui_phosphor::bold::ARROWS_DOWN_UP, false),
        Section::Display => (egui_phosphor::fill::TEXT_COLUMNS, true),
    }
}

/// Draws the Filter/Sort/Display buttons in the `ui`'s layout direction.
/// Returns the clicked section, if any.
fn draw_section_buttons(ui: &mut egui::Ui, open: Option<Section>) -> Option<Section> {
    let mut clicked = None;
    let mut sections = [Section::Filter, Section::Sort, Section::Display];
    if ui.layout().main_dir() == egui::Direction::RightToLeft {
        sections.reverse();
    }
    for section in sections {
        let (icon, fill) = section_icon(section);
        if icon_label_button(
            ui,
            icon,
            fill,
            section.label(),
            open == Some(section),
            false,
        )
        .clicked()
        {
            clicked = Some(section);
        }
    }
    clicked
}

/// The "Base ▾" button and its dropdown listing every table in the schema.
/// Returns the newly chosen base table, if any.
fn draw_base_button(
    ui: &mut egui::Ui,
    base_table: &str,
    schema: &Arc<Mutex<Option<String>>>,
) -> Option<String> {
    let resp = icon_label_button(ui, egui_phosphor::fill::LEGO, true, "Base", false, true);
    let mut choice = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(120.0);
        let tables = schema
            .lock()
            .unwrap()
            .as_deref()
            .map(table_names)
            .unwrap_or_default();
        if tables.is_empty() {
            ui.weak("Loading schema…");
            return;
        }
        for table in tables {
            if ui.selectable_label(table == base_table, &table).clicked() {
                choice = Some(table);
            }
        }
    });
    choice
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

/// An icon + label toolbar button, with an optional dropdown caret. `active`
/// gives it the light-blue toggled background.
pub(crate) fn icon_label_button(
    ui: &mut egui::Ui,
    icon: &str,
    fill_icon: bool,
    label: &str,
    active: bool,
    caret: bool,
) -> egui::Response {
    let color = ui.visuals().text_color();
    let icon_family = if fill_icon {
        egui::FontFamily::Name("phosphor-fill".into())
    } else {
        egui::FontFamily::Proportional
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        icon,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(14.0, icon_family),
            color,
            ..Default::default()
        },
    );
    job.append(
        label,
        5.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color,
            ..Default::default()
        },
    );
    if caret {
        job.append(
            egui_phosphor::bold::CARET_DOWN,
            4.0,
            egui::TextFormat {
                font_id: egui::FontId::proportional(10.0),
                color,
                ..Default::default()
            },
        );
    }
    let button = if active {
        egui::Button::new(job).fill(ACTIVE_SECTION_BG)
    } else {
        egui::Button::new(job).frame(false)
    };
    ui.add(button)
}

/// Paints the wrench (query-builder) toggle shown on narrow screens, mirroring
/// the explorer button's manual rendering but with a blue active fill.
fn wrench_button(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let font = egui::FontId::new(18.0, egui::FontFamily::Name("phosphor-fill".into()));
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        if active {
            ui.painter().rect_filled(rect, 4.0, ACCENT_BLUE);
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
            egui_phosphor::fill::WRENCH,
            font,
            icon_color,
        );
    }
    resp
}

/// Paints the run button (with spinner) and, when there are unsaved changes, the
/// save button to its left. Returns `(run_clicked, save_clicked)`.
fn paint_run_save(
    ui: &mut egui::Ui,
    run_enabled: bool,
    running: bool,
    unsaved: bool,
) -> (bool, bool) {
    let run = ui
        .add_enabled(
            run_enabled,
            egui::Button::new(
                egui::RichText::new(egui_phosphor::bold::ARROWS_CLOCKWISE).size(18.0),
            )
            .frame(false),
        )
        .clicked();
    if running {
        ui.spinner();
    }
    let mut save = false;
    if unsaved
        && ui
            .add(
                egui::Button::new(egui::RichText::new(egui_phosphor::bold::FLOPPY_DISK).size(18.0))
                    .frame(false),
            )
            .clicked()
    {
        save = true;
    }
    (run, save)
}
