pub trait AudioPlayer {
    fn load(&mut self, track_id: &str);
    fn play(&mut self);
    fn pause(&mut self);
    fn is_playing(&self) -> bool;
    fn position(&self) -> f64;
    fn duration(&self) -> Option<f64>;
}

#[cfg(target_arch = "wasm32")]
pub fn new_player() -> Box<dyn AudioPlayer> {
    Box::new(wasm_impl::WebAudioPlayer::new())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn new_player() -> Box<dyn AudioPlayer> {
    Box::<native_impl::NullAudioPlayer>::default()
}

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    #![allow(unsafe_code)]

    use wasm_bindgen::JsCast;
    use web_sys::HtmlAudioElement;

    use super::AudioPlayer;

    pub struct WebAudioPlayer {
        audio: HtmlAudioElement,
        loaded_id: Option<String>,
    }

    impl WebAudioPlayer {
        pub fn new() -> Self {
            let document = web_sys::window()
                .expect("no window")
                .document()
                .expect("no document");
            let element = document
                .create_element("audio")
                .expect("create audio element");
            let audio: HtmlAudioElement = element.unchecked_into();
            audio.set_preload("auto");
            let _ = audio.style().set_property("display", "none");
            if let Some(body) = document.body() {
                let _ = body.append_child(&audio);
            }
            Self {
                audio,
                loaded_id: None,
            }
        }
    }

    impl AudioPlayer for WebAudioPlayer {
        fn load(&mut self, track_id: &str) {
            if self.loaded_id.as_deref() == Some(track_id) {
                return;
            }
            let src = format!("/api/tracks/{track_id}/stream");
            self.audio.set_src(&src);
            self.audio.load();
            self.loaded_id = Some(track_id.to_string());
        }

        fn play(&mut self) {
            let _ = self.audio.play();
        }

        fn pause(&mut self) {
            let _ = self.audio.pause();
        }

        fn is_playing(&self) -> bool {
            !self.audio.paused() && !self.audio.ended()
        }

        fn position(&self) -> f64 {
            let t = self.audio.current_time();
            if t.is_finite() { t } else { 0.0 }
        }

        fn duration(&self) -> Option<f64> {
            let d = self.audio.duration();
            if d.is_finite() && d > 0.0 { Some(d) } else { None }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native_impl {
    use super::AudioPlayer;

    #[derive(Default)]
    pub struct NullAudioPlayer {
        loaded_id: Option<String>,
        playing: bool,
    }

    impl AudioPlayer for NullAudioPlayer {
        fn load(&mut self, track_id: &str) {
            self.loaded_id = Some(track_id.to_string());
            self.playing = false;
        }

        fn play(&mut self) {
            if self.loaded_id.is_some() {
                self.playing = true;
            }
        }

        fn pause(&mut self) {
            self.playing = false;
        }

        fn is_playing(&self) -> bool {
            self.playing
        }

        fn position(&self) -> f64 {
            0.0
        }

        fn duration(&self) -> Option<f64> {
            None
        }
    }
}
