//! The welcome page: shown when no query is open (e.g. before any query has been
//! created). Like every page type, it renders the explorer button at top-left so
//! the user can reach the organizer and create a query.

use eframe::egui;

use crate::App;
use crate::page::explorer_button;

impl App {
    /// The welcome page's top bar: just the explorer button, rendered identically
    /// to every other page's via the shared [`explorer_button`] helper.
    pub(crate) fn render_welcome_bar(&mut self, ctx: &egui::Context) {
        let panel_fill = ctx.style().visuals.panel_fill;
        let organizer_open = self.organizer.open;
        let mut toggle_organizer = false;

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
                });
            });

        if toggle_organizer {
            self.organizer.open = !self.organizer.open;
        }
    }
}

/// The welcome page's body: the app name, centered. Intentionally minimal for
/// now — we can flesh this out later.
pub(crate) fn render_welcome_center(ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.centered_and_justified(|ui| {
            ui.heading(egui::RichText::new("Collectune").size(48.0));
        });
    });
}
