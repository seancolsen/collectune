//! The top menu bar (organizer toggle, current query name, save/run/gear) and
//! the collapsible query-definition editor below it.

use eframe::egui;

use crate::App;
use crate::page::{QueryPage, explorer_button};

impl App {
    pub(crate) fn render_menu_bar(&mut self, ctx: &egui::Context) {
        let panel_fill = ctx.style().visuals.panel_fill;
        let has_page = self.current.query_id().is_some();
        let name = self
            .current_page()
            .map_or(String::new(), |p| p.live.name.clone());
        let running = self
            .current_page()
            .is_some_and(|p| p.results.lock().unwrap().running);
        let unsaved = self.current_page().is_some_and(QueryPage::unsaved);
        let config_open = self.config_open;

        let mut toggle_organizer = false;
        let mut toggle_config = false;
        let mut run_now = false;
        let mut save_now = false;

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
                    if explorer_button(ui) {
                        toggle_organizer = true;
                    }
                    if has_page {
                        ui.add_space(6.0);
                        ui.label(&name);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if paint_gear(ui, config_open).clicked() {
                            toggle_config = true;
                        }
                        (run_now, save_now) =
                            paint_run_save(ui, has_page && !running, running, unsaved);
                    });
                });
            });

        if toggle_organizer {
            self.organizer.open = !self.organizer.open;
        }
        if toggle_config {
            self.config_open = !self.config_open;
        }
        if run_now {
            self.run_query(ctx);
        }
        if save_now {
            self.save_current();
        }
    }

    pub(crate) fn render_config_panel(&mut self, ctx: &egui::Context) {
        let config_height = ctx.available_rect().height() * 0.3;
        let mut run_now = false;
        egui::TopBottomPanel::top("config")
            .exact_height(config_height)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(page) = self.current_page_mut() else {
                        ui.weak("No query selected.");
                        return;
                    };
                    let running = page.results.lock().unwrap().running;
                    let text_edit_resp = ui.add(
                        egui::TextEdit::multiline(&mut page.live.definition)
                            .desired_width(f32::INFINITY)
                            .desired_rows(6)
                            .font(egui::TextStyle::Monospace),
                    );
                    if !running
                        && text_edit_resp.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl)
                    {
                        run_now = true;
                    }
                });
            });
        if run_now {
            self.run_query(ctx);
        }
    }
}

/// Paints the gear (config) toggle, manually rendered for its custom active fill.
fn paint_gear(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let gear_font = egui::FontId::new(18.0, egui::FontFamily::Name("phosphor-fill".into()));
    let (gear_rect, gear_resp) =
        ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    if ui.is_rect_visible(gear_rect) {
        if active {
            ui.painter()
                .rect_filled(gear_rect, 4.0, ui.visuals().text_color());
        } else if gear_resp.hovered() {
            ui.painter()
                .rect_filled(gear_rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        let icon_color = if active {
            egui::Color32::WHITE
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            gear_rect.center(),
            egui::Align2::CENTER_CENTER,
            egui_phosphor::fill::GEAR_SIX,
            gear_font,
            icon_color,
        );
    }
    gear_resp
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
