use std::sync::{Arc, Mutex};

use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_ipc::reader::StreamReader;
use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Collectune",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

#[derive(Default)]
struct QueryState {
    result_text: String,
    error: Option<String>,
    running: bool,
}

struct App {
    query_text: String,
    state: Arc<Mutex<QueryState>>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            query_text: String::new(),
            state: Arc::new(Mutex::new(QueryState::default())),
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

            if !state.result_text.is_empty() {
                let available = ui.available_size();
                let mut text = state.result_text.as_str();
                ui.add_sized(
                    available,
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace),
                );
            }
        });
    }
}

impl App {
    fn run_query(&self, ctx: &egui::Context) {
        let query = self.query_text.clone();
        let state = Arc::clone(&self.state);
        let ctx = ctx.clone();

        {
            let mut s = state.lock().unwrap();
            s.result_text.clear();
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

    let reader =
        StreamReader::try_new(resp.into_reader(), None).map_err(|e| e.to_string())?;

    let schema = reader.schema();
    {
        let header: String = schema
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let mut s = state.lock().unwrap();
        s.result_text.push_str(&header);
        s.result_text.push('\n');
    }
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
            for (i, fmt) in formatters.iter().enumerate() {
                if i > 0 {
                    s.result_text.push(' ');
                }
                s.result_text.push_str(&fmt.value(row).to_string());
            }
            s.result_text.push('\n');
        }
        drop(s);
        ctx.request_repaint();
    }

    Ok(())
}
