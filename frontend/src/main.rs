use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_ipc::reader::StreamReader;
use clap::Parser;
use eframe::egui;
use eframe::egui::emath::TSTransform;

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

#[derive(Parser)]
struct Cli {
    /// UI scale factor (e.g. 1.5, 2)
    #[arg(long, short)]
    scale: Option<f32>,
}

fn main() -> eframe::Result {
    let cli = Cli::parse();

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Collectune",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            if let Some(s) = cli.scale {
                cc.egui_ctx.set_pixels_per_point(s);
            }
            let mut fonts = egui::FontDefinitions::default();
            egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Bold);
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(App::default()))
        }),
    )
}

#[derive(Default)]
struct QueryState {
    rows: Vec<String>,
    error: Option<String>,
    running: bool,
}

struct App {
    query_text: String,
    state: Arc<Mutex<QueryState>>,
    selection: HashSet<usize>,
    selection_anchor: Option<usize>,
    organizer_open: bool,
    organizer_dragging: bool,
    organizer_drag_dx: f32,
    organizer_drag_start_progress: f32,
    organizer_dragged_progress: f32,
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
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let panel_fill = ctx.style().visuals.panel_fill;

        let (anim_target, anim_time) = if self.organizer_dragging {
            (self.organizer_dragged_progress, 0.0)
        } else if self.organizer_open {
            (1.0, ORGANIZER_ANIM_TIME)
        } else {
            (0.0, ORGANIZER_ANIM_TIME)
        };
        let progress = ctx.animate_value_with_time(
            egui::Id::new("organizer_anim"),
            anim_target,
            anim_time,
        );
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
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.query_text)
                    .desired_width(f32::INFINITY)
                    .desired_rows(6)
                    .font(egui::TextStyle::Monospace),
            );

            let running = self.state.lock().unwrap().running;

            ui.horizontal(|ui| {
                if ui.add_enabled(!running, egui::Button::new("Run")).clicked() {
                    self.run_query(ctx);
                }
                if running {
                    ui.spinner();
                }
            });

            let state = self.state.lock().unwrap();

            if let Some(err) = &state.error {
                ui.colored_label(egui::Color32::RED, err);
            }

            let mut clicked: Option<(usize, egui::Modifiers)> = None;
            if !state.rows.is_empty() {
                let rows = &state.rows;
                let selection = &self.selection;
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                let padding = 6.0;
                let row_height_padded = row_height + padding * 2.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height_padded, rows.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for index in range {
                            let resp = row_widget(
                                ui,
                                &rows[index],
                                selection.contains(&index),
                                row_height_padded,
                            );
                            if resp.clicked() {
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

fn row_widget(ui: &mut egui::Ui, text: &str, selected: bool, height: f32) -> egui::Response {
    let desired = egui::vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let visuals = ui.visuals();
    let bg = if selected {
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

    ui.painter().rect_filled(rect, 0.0, bg);

    // thin separator line at the bottom of the row
    let sep_color = visuals.widgets.noninteractive.bg_stroke.color;
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, sep_color),
    );

    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        font_id,
        visuals.text_color(),
    );

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
            self.organizer_dragged_progress = (self.organizer_drag_start_progress
                + effective / ORGANIZER_WIDTH)
                .clamp(0.0, 1.0);
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
        }

        std::thread::spawn(move || {
            let result = execute_query(&query, &state, &ctx);
            let mut s = state.lock().unwrap();
            if let Err(e) = result {
                s.error = Some(e);
            }
            s.running = false;
            ctx.request_repaint();
        });
    }
}

fn execute_query(
    query: &str,
    state: &Mutex<QueryState>,
    ctx: &egui::Context,
) -> Result<(), String> {
    let resp = ureq::post("http://localhost:3000/query")
        .send_string(query)
        .map_err(|e| match e {
            ureq::Error::Status(_, resp) => resp.into_string().unwrap_or_else(|e| e.to_string()),
            other => other.to_string(),
        })?;

    let reader = StreamReader::try_new(resp.into_reader(), None).map_err(|e| e.to_string())?;

    ctx.request_repaint();

    let fmt_opts = FormatOptions::default();
    for batch_result in reader {
        let batch = batch_result.map_err(|e| e.to_string())?;
        let formatters: Vec<_> = batch
            .columns()
            .iter()
            .map(|col| ArrayFormatter::try_new(col.as_ref(), &fmt_opts))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;

        let mut s = state.lock().unwrap();
        for row in 0..batch.num_rows() {
            let mut line = String::new();
            for (i, fmt) in formatters.iter().enumerate() {
                if i > 0 {
                    line.push(' ');
                }
                line.push_str(&fmt.value(row).to_string());
            }
            s.rows.push(line);
        }
        drop(s);
        ctx.request_repaint();
    }

    Ok(())
}
