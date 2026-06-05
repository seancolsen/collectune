//! The "now playing" bottom bar and the bookkeeping that ties a playing track
//! back to the query page it was played from.

use std::time::Duration;

use eframe::egui;
use uuid::Uuid;

use crate::{ACCENT_BLUE, App};

#[derive(Clone)]
pub(crate) struct CurrentTrack {
    /// The query page this track was played from (results are per-page).
    pub(crate) source_page: Uuid,
    pub(crate) id: String,
    pub(crate) row_index: Option<usize>,
    pub(crate) title: Option<String>,
    pub(crate) artist_names: Vec<String>,
}

#[derive(Clone, Copy)]
enum MenuAction {
    Next,
    Close,
    Locate,
}

impl App {
    pub(crate) fn render_now_playing(&mut self, ctx: &egui::Context) {
        let snapshot = self.current_track.lock().unwrap().clone();
        let Some(ct) = snapshot else {
            return;
        };

        let playing = self.audio.is_playing();
        let position = self.audio.position();
        let duration = self.audio.duration();

        if playing {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else if self.audio.has_ended() {
            if let Some((source, next_idx, id)) = self.next_track_info() {
                self.play_track(source, next_idx, &id, ctx);
            } else {
                *self.current_track.lock().unwrap() = None;
            }
            ctx.request_repaint();
            return;
        }

        let (toggle, action) = self.paint_now_playing_bar(ctx, &ct, playing, position, duration);

        if toggle {
            if playing {
                self.audio.pause();
            } else {
                self.audio.play();
            }
        }

        match action {
            Some(MenuAction::Next) => {
                if let Some((source, next_idx, id)) = self.next_track_info() {
                    self.play_track(source, next_idx, &id, ctx);
                }
            }
            Some(MenuAction::Close) => {
                self.audio.pause();
                *self.current_track.lock().unwrap() = None;
            }
            Some(MenuAction::Locate) => {
                if let Some(idx) = ct.row_index {
                    // Switch to the page the track lives on, then scroll to it.
                    self.current = crate::CurrentPage::Query(ct.source_page);
                    self.pending_scroll_to_row = Some(idx);
                    ctx.request_repaint();
                }
            }
            None => {}
        }
    }

    /// Re-locates the current track's row index within its source page's results,
    /// e.g. after that page was re-run and the rows changed.
    pub(crate) fn maybe_revalidate_current_track_index(&mut self) {
        let source = {
            let guard = self.current_track.lock().unwrap();
            let Some(ct) = guard.as_ref() else {
                return;
            };
            ct.source_page
        };
        let Some(results) = self.page_results(source) else {
            return;
        };
        let (needs, track_id_column, running, lineage_done) = {
            let s = results.lock().unwrap();
            (
                s.needs_revalidation,
                s.track_id_column,
                s.running,
                s.lineage_done,
            )
        };
        if !needs || running || !lineage_done {
            return;
        }

        let mut ct_guard = self.current_track.lock().unwrap();
        let Some(ct) = ct_guard.as_mut() else {
            drop(ct_guard);
            results.lock().unwrap().needs_revalidation = false;
            return;
        };

        let Some(col) = track_id_column else {
            ct.row_index = None;
            drop(ct_guard);
            results.lock().unwrap().needs_revalidation = false;
            return;
        };

        let s = results.lock().unwrap();
        let rows = &s.rows;
        let id = ct.id.as_str();

        if let Some(idx) = ct.row_index
            && rows.get(idx).and_then(|r| r.get(col)).map(String::as_str) == Some(id)
        {
            drop(s);
            drop(ct_guard);
            results.lock().unwrap().needs_revalidation = false;
            return;
        }

        let scan_limit = rows.len().min(1000);
        let mut found: Option<usize> = None;
        for (i, row) in rows.iter().take(scan_limit).enumerate() {
            if row.get(col).map(String::as_str) == Some(id) {
                found = Some(i);
                break;
            }
        }
        ct.row_index = found;
        drop(s);
        drop(ct_guard);
        results.lock().unwrap().needs_revalidation = false;
    }

    /// Paints the bottom "now playing" bar and reports whether playback should
    /// toggle and which menu action (if any) the user picked.
    fn paint_now_playing_bar(
        &self,
        ctx: &egui::Context,
        ct: &CurrentTrack,
        playing: bool,
        position: f64,
        duration: Option<f64>,
    ) -> (bool, Option<MenuAction>) {
        let panel_fill = ctx.style().visuals.panel_fill;
        let sheet_fill = {
            let [r, g, b, a] = panel_fill.to_array();
            egui::Color32::from_rgba_unmultiplied(
                r.saturating_sub(8),
                g.saturating_sub(8),
                b.saturating_sub(8),
                a,
            )
        };

        let mut toggle = false;
        let mut action: Option<MenuAction> = None;
        egui::TopBottomPanel::bottom("now_playing")
            .exact_height(40.0)
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(0))
                    .fill(sheet_fill),
            )
            .show(ctx, |ui| {
                let full = ui.available_rect_before_wrap();
                let pad_x = 8.0;
                let timeline_height = 4.0;
                let timeline_bottom_pad = 2.0;
                let above_timeline_h = full.height() - timeline_height - timeline_bottom_pad;

                let (t, a) =
                    self.draw_now_playing_controls(ui, full, pad_x, above_timeline_h, ct, playing);
                toggle = t;
                action = a;

                draw_now_playing_text(ui, full, pad_x, above_timeline_h, ct);

                let progress = match (duration, position) {
                    (Some(d), p) if d > 0.0 => (p / d).clamp(0.0, 1.0) as f32,
                    _ => 0.0,
                };
                draw_now_playing_timeline(ui, full, timeline_height, timeline_bottom_pad, progress);
            });

        (toggle, action)
    }

    /// Draws the play/pause button and overflow menu on the right of the bar.
    fn draw_now_playing_controls(
        &self,
        ui: &egui::Ui,
        full: egui::Rect,
        pad_x: f32,
        above_timeline_h: f32,
        ct: &CurrentTrack,
        playing: bool,
    ) -> (bool, Option<MenuAction>) {
        let mut toggle = false;
        let mut action: Option<MenuAction> = None;

        let icon_font = egui::FontId::new(18.0, egui::FontFamily::Name("phosphor-fill".into()));
        let icon_char = if playing {
            egui_phosphor::fill::PAUSE
        } else {
            egui_phosphor::fill::PLAY
        };
        let menu_icon_char = egui_phosphor::fill::DOTS_THREE_OUTLINE_VERTICAL;
        let visuals = ui.visuals().clone();

        let button_size = egui::vec2(26.0, 26.0);
        let button_gap = 4.0;
        let buttons_center_y = full.min.y + above_timeline_h * 0.5;
        let menu_btn_rect = egui::Rect::from_min_size(
            egui::pos2(
                full.max.x - pad_x - button_size.x,
                buttons_center_y - button_size.y * 0.5,
            ),
            button_size,
        );
        let play_btn_rect = egui::Rect::from_min_size(
            egui::pos2(
                menu_btn_rect.min.x - button_gap - button_size.x,
                buttons_center_y - button_size.y * 0.5,
            ),
            button_size,
        );
        let play_resp = ui.interact(
            play_btn_rect,
            ui.id().with("now_playing_toggle"),
            egui::Sense::click(),
        );
        ui.painter().text(
            play_btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon_char,
            icon_font.clone(),
            visuals.text_color(),
        );
        if play_resp.clicked() {
            toggle = true;
        }
        let menu_resp = ui.interact(
            menu_btn_rect,
            ui.id().with("now_playing_menu"),
            egui::Sense::click(),
        );
        ui.painter().text(
            menu_btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            menu_icon_char,
            icon_font,
            visuals.text_color(),
        );

        let can_next = self.next_track_info().is_some();
        let can_locate = ct.row_index.is_some();
        egui::Popup::menu(&menu_resp)
            .align(egui::RectAlign::TOP_END)
            .width(130.0)
            .show(|ui| {
                if menu_item(ui, egui_phosphor::fill::SKIP_FORWARD, "Next", can_next).clicked() {
                    action = Some(MenuAction::Next);
                }
                if menu_item(ui, egui_phosphor::bold::X, "Close", true).clicked() {
                    action = Some(MenuAction::Close);
                }
                if menu_item(ui, egui_phosphor::fill::CROSSHAIR, "Locate", can_locate).clicked() {
                    action = Some(MenuAction::Locate);
                }
                let _ = menu_item(ui, egui_phosphor::fill::PENCIL, "Edit", true);
            });

        (toggle, action)
    }

    /// The track immediately after the current one within its source page's
    /// results, as `(source_page, row_index, track_id)`.
    fn next_track_info(&self) -> Option<(Uuid, usize, String)> {
        let (source, cur_idx) = {
            let guard = self.current_track.lock().unwrap();
            let ct = guard.as_ref()?;
            (ct.source_page, ct.row_index?)
        };
        let results = self.page_results(source)?;
        let s = results.lock().unwrap();
        let col = s.track_id_column?;
        let next_idx = cur_idx + 1;
        let id = s.rows.get(next_idx)?.get(col)?.clone();
        if id.is_empty() {
            return None;
        }
        Some((source, next_idx, id))
    }
}

/// Draws the track title and artist names on the left of the now-playing bar.
fn draw_now_playing_text(
    ui: &egui::Ui,
    full: egui::Rect,
    pad_x: f32,
    above_timeline_h: f32,
    ct: &CurrentTrack,
) {
    let visuals = ui.visuals();
    let title_font = egui::FontId::proportional(13.0);
    let artist_font = egui::FontId::proportional(11.0);
    let text_left = full.min.x + pad_x;
    let title_h = 14.0;
    let line_gap = 1.0;
    let artist_h = 11.0;
    let total_text_h = title_h + line_gap + artist_h;
    let text_top = full.min.y + above_timeline_h * 0.5 - total_text_h * 0.5;
    let title = ct.title.as_deref().unwrap_or("");
    ui.painter().text(
        egui::pos2(text_left, text_top),
        egui::Align2::LEFT_TOP,
        title,
        title_font,
        visuals.text_color(),
    );
    let artists = ct.artist_names.join(", ");
    ui.painter().text(
        egui::pos2(text_left, text_top + title_h + line_gap),
        egui::Align2::LEFT_TOP,
        artists,
        artist_font,
        visuals.weak_text_color(),
    );
}

/// Draws the played/unplayed timeline pills near the bottom of the bar.
fn draw_now_playing_timeline(
    ui: &egui::Ui,
    full: egui::Rect,
    timeline_height: f32,
    timeline_bottom_pad: f32,
    progress: f32,
) {
    let timeline_pad_x = 2.0;
    let track_rect = egui::Rect::from_min_size(
        egui::pos2(
            full.min.x + timeline_pad_x,
            full.max.y - timeline_bottom_pad - timeline_height,
        ),
        egui::vec2(full.width() - timeline_pad_x * 2.0, timeline_height),
    );
    let gap = 4.0;
    let played_w = track_rect.width() * progress;
    let rounding = timeline_height * 0.5;
    let unplayed_color = egui::Color32::from_rgba_unmultiplied(46, 124, 246, 70);

    if played_w > 0.0 {
        let played_rect = egui::Rect::from_min_size(
            track_rect.min,
            egui::vec2((played_w - gap * 0.5).max(0.0), timeline_height),
        );
        if played_rect.width() > 0.0 {
            ui.painter().rect_filled(played_rect, rounding, ACCENT_BLUE);
        }
    }
    let unplayed_start = track_rect.min.x + (played_w + gap * 0.5).max(0.0);
    if unplayed_start < track_rect.max.x {
        let unplayed_rect = egui::Rect::from_min_max(
            egui::pos2(unplayed_start, track_rect.min.y),
            egui::pos2(track_rect.max.x, track_rect.max.y),
        );
        ui.painter()
            .rect_filled(unplayed_rect, rounding, unplayed_color);
    }
}

fn menu_item(ui: &mut egui::Ui, icon: &str, label: &str, enabled: bool) -> egui::Response {
    let row_height = 28.0;
    let icon_size = 16.0;
    let label_size = 13.0;
    let row_width = ui.available_width();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(row_width, row_height), sense);

    let visuals = ui.visuals();
    if enabled && resp.hovered() {
        ui.painter()
            .rect_filled(rect, 4.0, visuals.widgets.hovered.weak_bg_fill);
    }

    let text_color = if enabled {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };

    let icon_font = egui::FontId::new(icon_size, egui::FontFamily::Name("phosphor-fill".into()));
    let label_font = egui::FontId::proportional(label_size);
    let pad_x = 10.0;
    let icon_x = rect.left() + pad_x;
    ui.painter().text(
        egui::pos2(icon_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon,
        icon_font,
        text_color,
    );
    ui.painter().text(
        egui::pos2(icon_x + icon_size + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        label_font,
        text_color,
    );

    resp
}
