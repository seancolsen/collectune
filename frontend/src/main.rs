use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_ipc::reader::StreamReader;
use clap::Parser;
use eframe::egui;

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
}

impl Default for App {
    fn default() -> Self {
        Self {
            query_text: String::new(),
            state: Arc::new(Mutex::new(QueryState::default())),
            selection: HashSet::new(),
            selection_anchor: None,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height, rows.len(), |ui, range| {
                        for index in range {
                            let resp = row_widget(
                                ui,
                                &rows[index],
                                selection.contains(&index),
                                row_height,
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
    }
}

fn row_widget(ui: &mut egui::Ui, text: &str, selected: bool, height: f32) -> egui::Response {
    let desired = egui::vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let visuals = ui.visuals();
    let bg = if selected {
        Some(visuals.selection.bg_fill)
    } else if response.hovered() {
        Some(visuals.widgets.hovered.weak_bg_fill)
    } else {
        None
    };

    if let Some(color) = bg {
        ui.painter().rect_filled(rect, 0.0, color);
    }

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
