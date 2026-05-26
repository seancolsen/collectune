#[cfg(not(target_arch = "wasm32"))]
mod native {
    use clap::Parser;
    use frontend::{App, setup_fonts};

    #[derive(Parser)]
    struct Cli {
        /// UI scale factor (e.g. 1.5, 2)
        #[arg(long, short)]
        scale: Option<f32>,
    }

    pub fn run() -> eframe::Result {
        let cli = Cli::parse();
        let options = eframe::NativeOptions::default();
        eframe::run_native(
            "Collectune",
            options,
            Box::new(move |cc| {
                setup_fonts(&cc.egui_ctx);
                if let Some(s) = cli.scale {
                    cc.egui_ctx.set_pixels_per_point(s);
                }
                Ok(Box::new(App::default()))
            }),
        )
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    native::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
