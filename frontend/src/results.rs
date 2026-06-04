//! The central panel: renders the current query page's result rows, and handles
//! row selection and double-click-to-play.

use eframe::egui;

use crate::{ACCENT_BLUE, App};

impl App {
    pub(crate) fn render_results(&mut self, ctx: &egui::Context) {
        let Some(current_id) = self.current else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.weak("Select or create a query.");
                });
            });
            return;
        };
        let Some(results) = self.page_results(current_id) else {
            egui::CentralPanel::default().show(ctx, |_ui| {});
            return;
        };

        egui::CentralPanel::default().show(ctx, |ui| {
            let state = results.lock().unwrap();

            if let Some(err) = &state.error {
                ui.colored_label(egui::Color32::RED, err);
            }

            let mut clicked: Option<(usize, egui::Modifiers)> = None;
            let mut double_clicked: Option<(usize, String)> = None;
            if !state.rows.is_empty() {
                let pending_locate = self
                    .pending_scroll_to_row
                    .take()
                    .filter(|i| *i < state.rows.len());
                if let Some(idx) = pending_locate {
                    self.selection.clear();
                    self.selection.insert(idx);
                    self.selection_anchor = Some(idx);
                }
                let rows = &state.rows;
                let selection = &self.selection;
                let track_id_column = state.track_id_column;
                // Only highlight the now-playing row when it belongs to this page.
                let current_row = {
                    let ct = self.current_track.lock().unwrap();
                    ct.as_ref()
                        .filter(|c| c.source_page == current_id)
                        .and_then(|c| c.row_index)
                };
                let row_height = ui.text_style_height(&egui::TextStyle::Body);
                let padding = 6.0;
                let sub_line_height = if track_id_column.is_some() {
                    row_height
                } else {
                    0.0
                };
                let row_height_padded = row_height + sub_line_height + padding * 2.0;
                let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false, false]);
                if let Some(idx) = pending_locate {
                    let viewport_h = ui.available_height();
                    let target = (idx as f32 * row_height_padded)
                        - (viewport_h - row_height_padded).max(0.0) * 0.5;
                    scroll_area = scroll_area.vertical_scroll_offset(target.max(0.0));
                }
                scroll_area.show_rows(ui, row_height_padded, rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for index in range {
                        let cells = &rows[index];
                        let main_text = cells.join(" ");
                        let track_id =
                            track_id_column.and_then(|i| cells.get(i).map(String::as_str));
                        let is_current = current_row == Some(index);
                        let resp = row_widget(
                            ui,
                            &main_text,
                            track_id,
                            selection.contains(&index),
                            is_current,
                            row_height_padded,
                        );
                        if resp.double_clicked() {
                            if let Some(id) = track_id {
                                double_clicked = Some((index, id.to_string()));
                            }
                        } else if resp.clicked() {
                            let mods = ui.input(|i| i.modifiers);
                            clicked = Some((index, mods));
                        }
                    }
                });
            }
            drop(state);

            if let Some((index, mods)) = clicked {
                self.handle_row_click(index, mods);
            }
            if let Some((index, id)) = double_clicked {
                self.play_track(current_id, index, &id, ctx);
            }
        });
    }

    pub(crate) fn handle_row_click(&mut self, index: usize, modifiers: egui::Modifiers) {
        if modifiers.shift {
            let anchor = self.selection_anchor.unwrap_or(index);
            let (lo, hi) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            self.selection.clear();
            for i in lo..=hi {
                self.selection.insert(i);
            }
        } else if modifiers.command || modifiers.ctrl {
            if !self.selection.remove(&index) {
                self.selection.insert(index);
            }
            self.selection_anchor = Some(index);
        } else {
            self.selection.clear();
            self.selection.insert(index);
            self.selection_anchor = Some(index);
        }
    }
}

fn row_widget(
    ui: &mut egui::Ui,
    text: &str,
    track_id: Option<&str>,
    selected: bool,
    is_current: bool,
    height: f32,
) -> egui::Response {
    let desired = egui::vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let visuals = ui.visuals();
    let base_bg = if selected {
        let base = visuals.selection.bg_fill;
        if response.hovered() {
            egui::Color32::from_rgba_unmultiplied(
                base.r().saturating_sub(20),
                base.g().saturating_sub(20),
                base.b().saturating_sub(20),
                base.a(),
            )
        } else {
            base
        }
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        visuals.extreme_bg_color
    };

    ui.painter().rect_filled(rect, 0.0, base_bg);

    if is_current && !selected {
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(46, 124, 246, 16),
        );
    }

    if is_current {
        let accent_rect =
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height()));
        ui.painter().rect_filled(accent_rect, 0.0, ACCENT_BLUE);
    }

    // thin separator line at the bottom of the row
    let sep_color = visuals.widgets.noninteractive.bg_stroke.color;
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, sep_color),
    );

    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let line_height = ui.text_style_height(&egui::TextStyle::Body);
    let text_left = rect.left() + 8.0;

    if let Some(id) = track_id {
        let padding = (rect.height() - line_height * 2.0) * 0.5;
        let main_pos = egui::pos2(text_left, rect.top() + padding);
        let sub_pos = main_pos + egui::vec2(0.0, line_height);
        ui.painter().text(
            main_pos,
            egui::Align2::LEFT_TOP,
            text,
            font_id.clone(),
            visuals.text_color(),
        );
        ui.painter().text(
            sub_pos,
            egui::Align2::LEFT_TOP,
            format!("track.id: {id}"),
            font_id,
            visuals.weak_text_color(),
        );
    } else {
        ui.painter().text(
            egui::pos2(text_left, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            font_id,
            visuals.text_color(),
        );
    }

    response
}
