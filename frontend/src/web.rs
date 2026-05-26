#![allow(unsafe_code)]

use wasm_bindgen::prelude::*;

use crate::{App, setup_fonts};

const CANVAS_ID: &str = "the_canvas_id";

#[wasm_bindgen(start)]
pub fn auto_start() {
    console_error_panic_hook::set_once();

    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");
        let canvas = document
            .get_element_by_id(CANVAS_ID)
            .unwrap_or_else(|| panic!("canvas element `{CANVAS_ID}` not found"))
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("element is not a canvas");

        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    setup_fonts(&cc.egui_ctx);
                    Ok(Box::new(App::default()))
                }),
            )
            .await
            .expect("eframe::WebRunner failed");
    });
}
