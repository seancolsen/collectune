use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;
use eframe::egui::emath::TSTransform;

mod audio;
mod http;
mod lineage;
#[cfg(target_arch = "wasm32")]
mod web;

use audio::AudioPlayer;

const ORGANIZER_WIDTH: f32 = 200.0;
const ORGANIZER_ANIM_TIME: f32 = 0.1;
/// Leftward pointer velocity (px/s) that counts as a swipe-to-close flick,
/// even if the cumulative drag distance is small.
const ORGANIZER_SWIPE_VELOCITY: f32 = 400.0;
/// Static-friction scale for the drawer drag. Small finger movements (well
/// below this) produce ~no drawer motion, so vertical scroll gestures inside
/// the drawer aren't mistaken for a close-swipe. Past a few times this value,
/// the drawer tracks the finger 1:1 (offset by a constant amount).
const ORGANIZER_DRAG_FRICTION: f32 = 16.0;

pub fn setup_fonts(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::light());
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Bold);
    // Load fill as a separate named family so it doesn't overwrite bold's "phosphor" key.
    fonts.font_data.insert(
        "phosphor-fill".into(),
        egui_phosphor::Variant::Fill.font_data().into(),
    );
    fonts.families.insert(
        egui::FontFamily::Name("phosphor-fill".into()),
        vec!["phosphor-fill".into()],
    );
    ctx.set_fonts(fonts);
}

#[derive(Default)]
pub(crate) struct QueryState {
    pub(crate) rows: Vec<Vec<String>>,
    pub(crate) error: Option<String>,
    pub(crate) running: bool,
    pub(crate) track_id_column: Option<usize>,
    pub(crate) lineage_done: bool,
    pub(crate) needs_revalidation: bool,
}

#[derive(Clone)]
pub(crate) struct CurrentTrack {
    pub(crate) id: String,
    pub(crate) row_index: Option<usize>,
    pub(crate) title: Option<String>,
    pub(crate) artist_names: Vec<String>,
}

const ACCENT_BLUE: egui::Color32 = egui::Color32::from_rgb(0x2E, 0x7C, 0xF6);

pub struct App {
    query_text: String,
    state: Arc<Mutex<QueryState>>,
    selection: HashSet<usize>,
    selection_anchor: Option<usize>,
    organizer_open: bool,
    organizer_dragging: bool,
    organizer_drag_dx: f32,
    organizer_drag_start_progress: f32,
    organizer_dragged_progress: f32,
    config_open: bool,
    current_track: Arc<Mutex<Option<CurrentTrack>>>,
    audio: Box<dyn AudioPlayer>,
    pending_scroll_to_row: Option<usize>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            query_text: String::new(),
            state: Arc::new(Mutex::new(QueryState::default())),
            selection: HashSet::new(),
            selection_anchor: None,
            organizer_open: false,
            organizer_dragging: false,
            organizer_drag_dx: 0.0,
            organizer_drag_start_progress: 0.0,
            organizer_dragged_progress: 0.0,
            config_open: false,
            current_track: Arc::new(Mutex::new(None)),
            audio: audio::new_player(),
            pending_scroll_to_row: None,
        }
    }
}

impl eframe::App for App {
    #[allow(clippy::too_many_lines)]
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let panel_fill = ctx.style().visuals.panel_fill;

        let (anim_target, anim_time) = if self.organizer_dragging {
            (self.organizer_dragged_progress, 0.0)
        } else if self.organizer_open {
            (1.0, ORGANIZER_ANIM_TIME)
        } else {
            (0.0, ORGANIZER_ANIM_TIME)
        };
        let progress =
            ctx.animate_value_with_time(egui::Id::new("organizer_anim"), anim_target, anim_time);
        let organizer_offset = progress * ORGANIZER_WIDTH;

        egui::TopBottomPanel::top("menu_bar")
            .exact_height(30.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(panel_fill)
                    .inner_margin(egui::Margin::same(0)),
            )
            .show(ctx, |ui| {
                let running = self.state.lock().unwrap().running;
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(egui_phosphor::bold::LIST).size(18.0),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.organizer_open = !self.organizer_open;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        // Gear button — rightmost, manually painted for custom active style.
                        let gear_font =
                            egui::FontId::new(18.0, egui::FontFamily::Name("phosphor-fill".into()));
                        let (gear_rect, gear_resp) =
                            ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
                        if ui.is_rect_visible(gear_rect) {
                            if self.config_open {
                                ui.painter()
                                    .rect_filled(gear_rect, 4.0, ui.visuals().text_color());
                            } else if gear_resp.hovered() {
                                ui.painter().rect_filled(
                                    gear_rect,
                                    4.0,
                                    ui.visuals().widgets.hovered.weak_bg_fill,
                                );
                            }
                            let icon_color = if self.config_open {
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
                        if gear_resp.clicked() {
                            self.config_open = !self.config_open;
                        }
                        // Run button — to the left of gear.
                        if ui
                            .add_enabled(
                                !running,
                                egui::Button::new(
                                    egui::RichText::new(egui_phosphor::bold::ARROWS_CLOCKWISE)
                                        .size(18.0),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.run_query(ctx);
                        }
                        if running {
                            ui.spinner();
                        }
                    });
                });
            });

        if self.config_open {
            let config_height = ctx.available_rect().height() * 0.3;
            egui::TopBottomPanel::top("config")
                .exact_height(config_height)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let text_edit_resp = ui.add(
                            egui::TextEdit::multiline(&mut self.query_text)
                                .desired_width(f32::INFINITY)
                                .desired_rows(6)
                                .font(egui::TextStyle::Monospace),
                        );
                        let running = self.state.lock().unwrap().running;
                        if !running
                            && text_edit_resp.has_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl)
                        {
                            self.run_query(ctx);
                        }
                    });
                });
        }

        self.render_now_playing(ctx);
        self.maybe_revalidate_current_track_index();

        egui::CentralPanel::default().show(ctx, |ui| {
            let state = self.state.lock().unwrap();

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
                let current_row = self
                    .current_track
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|c| c.row_index);
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
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
                self.play_track(index, id, ctx);
            }
        });

        ctx.set_transform_layer(
            egui::LayerId::background(),
            TSTransform::from_translation(egui::vec2(organizer_offset, 0.0)),
        );

        // Render while dragging even at progress == 0 so the widget that
        // owns the in-flight drag stays mounted and `drag_stopped` fires on
        // release — otherwise pulling the drawer fully closed before letting
        // go strands `organizer_dragging` at true.
        if progress > 0.0 || self.organizer_dragging {
            let viewport = ctx.viewport_rect();

            // The scrim covers the full viewport — we don't try to align its
            // left edge with the (animated) main-UI translation. The organizer
            // Area below sits on Order::Foreground and paints over the scrim
            // wherever the drawer currently is, so the user only ever sees
            // darkening on the main-UI portion. Decoupling the scrim from
            // organizer_offset eliminates the sub-frame chase between the
            // layer transform and the Area position that produced a visible
            // sliver of un-darkened main UI during opening.
            let scrim_alpha = (progress * 120.0).clamp(0.0, 255.0) as u8;

            egui::Area::new(egui::Id::new("organizer_scrim"))
                .order(egui::Order::Middle)
                .fixed_pos(viewport.min)
                .constrain(false)
                .interactable(true)
                .show(ctx, |ui| {
                    let (rect, resp) =
                        ui.allocate_exact_size(viewport.size(), egui::Sense::click_and_drag());
                    ui.painter().rect_filled(
                        rect,
                        0.0,
                        egui::Color32::from_black_alpha(scrim_alpha),
                    );
                    if resp.clicked() {
                        self.organizer_open = false;
                    }
                    self.handle_organizer_swipe(ctx, &resp, progress);
                });

            let organizer_x = organizer_offset - ORGANIZER_WIDTH;
            let screen_height = viewport.height();

            egui::Area::new(egui::Id::new("organizer"))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::pos2(organizer_x, 0.0))
                .constrain(false)
                .interactable(true)
                .show(ctx, |ui| {
                    let frame_rect = egui::Rect::from_min_size(
                        egui::pos2(organizer_x, 0.0),
                        egui::vec2(ORGANIZER_WIDTH, screen_height),
                    );
                    ui.set_min_size(egui::vec2(ORGANIZER_WIDTH, screen_height));
                    ui.painter().rect_filled(frame_rect, 0.0, panel_fill);

                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label("Organizer");
                    });

                    let drag = ui.interact(
                        frame_rect,
                        egui::Id::new("organizer_drag"),
                        egui::Sense::drag(),
                    );
                    self.handle_organizer_swipe(ctx, &drag, progress);
                });
        }
    }
}

/// Models a static-friction-like resistance to drag motion: near-zero
/// response for small `dx`, smoothly approaching 1:1 response (offset by
/// `friction`) for `|dx|` much larger than `friction`.
fn static_friction(dx: f32, friction: f32) -> f32 {
    if friction <= 0.0 {
        return dx;
    }
    dx - friction * (dx / friction).tanh()
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

    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let line_height = ui.text_style_height(&egui::TextStyle::Monospace);
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

impl App {
    fn handle_organizer_swipe(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        current_progress: f32,
    ) {
        if resp.drag_started() {
            self.organizer_dragging = true;
            self.organizer_drag_dx = 0.0;
            self.organizer_drag_start_progress = current_progress;
            self.organizer_dragged_progress = current_progress;
        }
        if resp.dragged() {
            self.organizer_drag_dx += resp.drag_delta().x;
            let effective = static_friction(self.organizer_drag_dx, ORGANIZER_DRAG_FRICTION);
            self.organizer_dragged_progress =
                (self.organizer_drag_start_progress + effective / ORGANIZER_WIDTH).clamp(0.0, 1.0);
        }
        if resp.drag_stopped() {
            self.organizer_dragging = false;
            let velocity_x = ctx.input(|i| i.pointer.velocity().x);
            let effective = static_friction(self.organizer_drag_dx, ORGANIZER_DRAG_FRICTION);
            let flick = velocity_x <= -ORGANIZER_SWIPE_VELOCITY;
            let dragged_past_midpoint = effective <= -ORGANIZER_WIDTH / 2.0;
            if flick || dragged_past_midpoint {
                self.organizer_open = false;
            }
            self.organizer_drag_dx = 0.0;
        }
    }

    fn handle_row_click(&mut self, index: usize, modifiers: egui::Modifiers) {
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

    fn run_query(&mut self, ctx: &egui::Context) {
        let query = self.query_text.clone();
        let state = Arc::clone(&self.state);
        let ctx = ctx.clone();

        self.selection.clear();
        self.selection_anchor = None;

        {
            let mut s = state.lock().unwrap();
            s.rows.clear();
            s.error = None;
            s.running = true;
            s.track_id_column = None;
            s.lineage_done = false;
            s.needs_revalidation = true;
        }

        lineage::detect_track_column(query.clone(), Arc::clone(&state), ctx.clone());
        http::run_query(query, state, ctx);
    }

    fn play_track(&mut self, index: usize, id: String, ctx: &egui::Context) {
        {
            let mut ct = self.current_track.lock().unwrap();
            *ct = Some(CurrentTrack {
                id: id.clone(),
                row_index: Some(index),
                title: None,
                artist_names: Vec::new(),
            });
        }
        self.audio.load(&id);
        self.audio.play();
        http::fetch_track_metadata(id, Arc::clone(&self.current_track), ctx.clone());
    }

    fn maybe_revalidate_current_track_index(&mut self) {
        let (needs, track_id_column, running, lineage_done) = {
            let s = self.state.lock().unwrap();
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
            // Still clear the flag — no current track to revalidate.
            drop(ct_guard);
            self.state.lock().unwrap().needs_revalidation = false;
            return;
        };

        let Some(col) = track_id_column else {
            ct.row_index = None;
            drop(ct_guard);
            self.state.lock().unwrap().needs_revalidation = false;
            return;
        };

        let state = self.state.lock().unwrap();
        let rows = &state.rows;
        let id = ct.id.as_str();

        if let Some(idx) = ct.row_index
            && rows.get(idx).and_then(|r| r.get(col)).map(String::as_str) == Some(id)
        {
            drop(state);
            drop(ct_guard);
            self.state.lock().unwrap().needs_revalidation = false;
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
        drop(state);
        drop(ct_guard);
        self.state.lock().unwrap().needs_revalidation = false;
    }

    fn render_now_playing(&mut self, ctx: &egui::Context) {
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
            if let Some((next_idx, id)) = self.next_track_info() {
                self.play_track(next_idx, id, ctx);
            } else {
                *self.current_track.lock().unwrap() = None;
            }
            ctx.request_repaint();
            return;
        }

        let mut toggle = false;
        let mut action: Option<MenuAction> = None;
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

                let icon_font =
                    egui::FontId::new(18.0, egui::FontFamily::Name("phosphor-fill".into()));
                let icon_char = if playing {
                    egui_phosphor::fill::PAUSE
                } else {
                    egui_phosphor::fill::PLAY
                };
                let menu_icon_char = egui_phosphor::fill::DOTS_THREE_OUTLINE_VERTICAL;
                let visuals = ui.visuals().clone();

                // -- Buttons on the right (play/pause, then menu) --
                let button_size = egui::vec2(26.0, 26.0);
                let button_gap = 4.0;
                let timeline_height = 4.0;
                let timeline_bottom_pad = 2.0;
                let above_timeline_h = full.height() - timeline_height - timeline_bottom_pad;
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

                let next_info = self.next_track_info();
                let can_next = next_info.is_some();
                let can_locate = ct.row_index.is_some();
                egui::Popup::menu(&menu_resp)
                    .align(egui::RectAlign::TOP_END)
                    .width(130.0)
                    .show(|ui| {
                        if menu_item(ui, egui_phosphor::fill::SKIP_FORWARD, "Next", can_next)
                            .clicked()
                        {
                            action = Some(MenuAction::Next);
                        }
                        if menu_item(ui, egui_phosphor::bold::X, "Close", true).clicked() {
                            action = Some(MenuAction::Close);
                        }
                        if menu_item(ui, egui_phosphor::fill::CROSSHAIR, "Locate", can_locate)
                            .clicked()
                        {
                            action = Some(MenuAction::Locate);
                        }
                        let _ = menu_item(ui, egui_phosphor::fill::PENCIL, "Edit", true);
                    });

                // -- Title + artist text on the left --
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

                // -- Timeline pills near the bottom --
                let progress = match (duration, position) {
                    (Some(d), p) if d > 0.0 => (p / d).clamp(0.0, 1.0) as f32,
                    _ => 0.0,
                };
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
            });

        if toggle {
            if playing {
                self.audio.pause();
            } else {
                self.audio.play();
            }
        }

        match action {
            Some(MenuAction::Next) => {
                if let Some((next_idx, id)) = self.next_track_info() {
                    self.play_track(next_idx, id, ctx);
                }
            }
            Some(MenuAction::Close) => {
                self.audio.pause();
                *self.current_track.lock().unwrap() = None;
            }
            Some(MenuAction::Locate) => {
                if let Some(idx) = ct.row_index {
                    self.pending_scroll_to_row = Some(idx);
                    ctx.request_repaint();
                }
            }
            None => {}
        }
    }

    fn next_track_info(&self) -> Option<(usize, String)> {
        let cur_idx = self.current_track.lock().unwrap().as_ref()?.row_index?;
        let s = self.state.lock().unwrap();
        let col = s.track_id_column?;
        let next_idx = cur_idx + 1;
        let id = s.rows.get(next_idx)?.get(col)?.clone();
        if id.is_empty() {
            return None;
        }
        Some((next_idx, id))
    }
}

#[derive(Clone, Copy)]
enum MenuAction {
    Next,
    Close,
    Locate,
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
