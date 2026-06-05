use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::egui::emath::TSTransform;
use uuid::Uuid;

mod audio;
mod columns;
mod compile;
mod field_layout;
mod http;
mod lineage;
mod menu_bar;
mod now_playing;
mod organizer;
mod page;
mod results;
mod rpc;
#[cfg(target_arch = "wasm32")]
mod web;
mod welcome;

use audio::AudioPlayer;
use columns::ColumnMetadata;
use field_layout::FieldLayout;
use now_playing::CurrentTrack;
use organizer::Organizer;
use page::{CurrentPage, QueryPage};

pub(crate) const ORGANIZER_WIDTH: f32 = 200.0;
const ORGANIZER_ANIM_TIME: f32 = 0.1;
/// Leftward pointer velocity (px/s) that counts as a swipe-to-close flick,
/// even if the cumulative drag distance is small.
pub(crate) const ORGANIZER_SWIPE_VELOCITY: f32 = 400.0;
/// Static-friction scale for the drawer drag. Small finger movements (well
/// below this) produce ~no drawer motion, so vertical scroll gestures inside
/// the drawer aren't mistaken for a close-swipe. Past a few times this value,
/// the drawer tracks the finger 1:1 (offset by a constant amount).
pub(crate) const ORGANIZER_DRAG_FRICTION: f32 = 16.0;

pub(crate) const ACCENT_BLUE: egui::Color32 = egui::Color32::from_rgb(0x2E, 0x7C, 0xF6);

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
    /// Resolved display metadata for each result column, positionally aligned with each
    /// row's cells. Empty until the query is (re)compiled.
    pub(crate) columns: Vec<ColumnMetadata>,
    pub(crate) error: Option<String>,
    pub(crate) running: bool,
    pub(crate) track_id_column: Option<usize>,
    pub(crate) lineage_done: bool,
    pub(crate) needs_revalidation: bool,
}

/// Which surface initiated an in-progress rename. Both surfaces edit the same
/// query name, but only the initiating one renders the inline field, so the two
/// can't fight over focus.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameSurface {
    /// The query's row in the organizer sidebar.
    Sidebar,
    /// The query name shown in the top menu bar of the query page.
    Page,
}

/// An in-progress inline rename of a query.
pub(crate) struct Rename {
    pub(crate) id: Uuid,
    pub(crate) buffer: String,
    pub(crate) surface: RenameSurface,
    /// Set on the first frame so the field grabs focus and selects its text once.
    pub(crate) take_focus: bool,
}

/// A delete awaiting confirmation in the modal dialog.
pub(crate) struct PendingDelete {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) unsaved: bool,
}

// Several independent one-shot startup/UI flags; grouping them into a sub-struct
// wouldn't make any of them clearer.
#[allow(clippy::struct_excessive_bools)]
pub struct App {
    /// All open query pages. The organizer is a switcher over these.
    pub(crate) pages: Vec<QueryPage>,
    /// The currently displayed page.
    pub(crate) current: CurrentPage,
    /// Whether the one-time, on-open auto-selection of the most-recent query has
    /// happened yet. Keeps later list refreshes from hijacking the current page.
    pub(crate) auto_selected_initial: bool,
    /// Organizer name filter (in-memory only; issues no requests).
    pub(crate) filter: String,
    /// Inbox for an in-flight `query.list`; drained into `pages` on the next frame.
    pub(crate) loaded_queries: Arc<Mutex<Option<Vec<rpc::Query>>>>,
    pub(crate) queries_fetch_started: bool,
    pub(crate) selection: HashSet<usize>,
    pub(crate) selection_anchor: Option<usize>,
    pub(crate) organizer: Organizer,
    /// The in-progress inline rename, if any.
    pub(crate) rename: Option<Rename>,
    /// The query whose deletion is awaiting confirmation in the modal, if any.
    pub(crate) pending_delete: Option<PendingDelete>,
    pub(crate) config_open: bool,
    pub(crate) current_track: Arc<Mutex<Option<CurrentTrack>>>,
    pub(crate) audio: Box<dyn AudioPlayer>,
    pub(crate) pending_scroll_to_row: Option<usize>,
    /// Database schema JSON, fetched once at startup and used to compile Querydown.
    pub(crate) schema: Arc<Mutex<Option<String>>>,
    pub(crate) schema_fetch_started: bool,
    /// Memoized result-row field layout, reused across rows and frames until the column
    /// set or available width changes.
    pub(crate) field_layout_cache: Option<(field_layout::LayoutKey, Rc<FieldLayout>)>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            current: CurrentPage::default(),
            auto_selected_initial: false,
            filter: String::new(),
            loaded_queries: Arc::new(Mutex::new(None)),
            queries_fetch_started: false,
            selection: HashSet::new(),
            selection_anchor: None,
            organizer: Organizer::default(),
            rename: None,
            pending_delete: None,
            config_open: false,
            current_track: Arc::new(Mutex::new(None)),
            audio: audio::new_player(),
            pending_scroll_to_row: None,
            schema: Arc::new(Mutex::new(None)),
            schema_fetch_started: false,
            field_layout_cache: None,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.bootstrap(ctx);
        self.drain_loaded_queries();

        let panel_fill = ctx.style().visuals.panel_fill;
        let progress = self.organizer_progress(ctx);
        let organizer_offset = progress * ORGANIZER_WIDTH;

        self.ensure_current_results(ctx);

        // Top panels: each page type renders its own bar (including the explorer
        // button). Top/bottom panels must be added before the central panel.
        match self.current {
            CurrentPage::Query(_) => {
                self.render_menu_bar(ctx);
                if self.config_open {
                    self.render_config_panel(ctx);
                }
            }
            CurrentPage::Welcome => self.render_welcome_bar(ctx),
        }
        self.render_now_playing(ctx);
        self.maybe_revalidate_current_track_index();

        // Central panel.
        match self.current {
            CurrentPage::Query(_) => self.render_results(ctx),
            CurrentPage::Welcome => welcome::render_welcome_center(ctx),
        }

        ctx.set_transform_layer(
            egui::LayerId::background(),
            TSTransform::from_translation(egui::vec2(organizer_offset, 0.0)),
        );

        // Render while dragging even at progress == 0 so the widget that owns
        // the in-flight drag stays mounted and `drag_stopped` fires on release.
        if progress > 0.0 || self.organizer.dragging {
            self.render_organizer(ctx, progress, panel_fill);
        }

        // The delete-confirmation modal floats above everything else.
        self.render_delete_confirm(ctx);
    }
}

impl App {
    /// Kicks off the one-time startup fetches (schema + saved query list).
    fn bootstrap(&mut self, ctx: &egui::Context) {
        if !self.schema_fetch_started {
            self.schema_fetch_started = true;
            http::fetch_schema(Arc::clone(&self.schema), ctx.clone());
        }
        if !self.queries_fetch_started {
            self.queries_fetch_started = true;
            rpc::list_queries(Arc::clone(&self.loaded_queries), ctx.clone());
        }
    }

    /// If a `query.list` response has arrived, rebuild `pages` from it. This wipes
    /// out never-saved ephemeral pages and any unsaved edits, but carries over the
    /// cached results (and their fetched flag) for queries that still exist, so a
    /// list refresh doesn't re-run the current page's query.
    fn drain_loaded_queries(&mut self) {
        let Some(list) = self.loaded_queries.lock().unwrap().take() else {
            return;
        };
        let mut prior: HashMap<Uuid, (Arc<Mutex<QueryState>>, bool)> = self
            .pages
            .drain(..)
            .map(|p| (p.live.id, (p.results, p.results_fetched)))
            .collect();
        self.pages = list
            .into_iter()
            .map(|q| {
                let mut page = QueryPage::persisted(q);
                if let Some((results, fetched)) = prior.remove(&page.live.id) {
                    page.results = results;
                    page.results_fetched = fetched;
                }
                page
            })
            .collect();

        if !self.auto_selected_initial {
            // On first load, open the most-recently-created query (or the welcome
            // page if there are none yet).
            self.auto_selected_initial = true;
            self.current = self
                .pages
                .iter()
                .max_by_key(|p| p.live.created_at)
                .map_or(CurrentPage::Welcome, |p| CurrentPage::Query(p.live.id));
        } else if let CurrentPage::Query(cur) = self.current
            && !self.pages.iter().any(|p| p.live.id == cur)
        {
            self.current = CurrentPage::Welcome;
        }
        self.selection.clear();
        self.selection_anchor = None;
    }

    fn organizer_progress(&self, ctx: &egui::Context) -> f32 {
        let (anim_target, anim_time) = if self.organizer.dragging {
            (self.organizer.dragged_progress, 0.0)
        } else if self.organizer.open {
            (1.0, ORGANIZER_ANIM_TIME)
        } else {
            (0.0, ORGANIZER_ANIM_TIME)
        };
        ctx.animate_value_with_time(egui::Id::new("organizer_anim"), anim_target, anim_time)
    }

    /// Auto-fetches the current page's results the first time it's shown, so
    /// navigating to a query with a cold cache loads it without an explicit run.
    fn ensure_current_results(&mut self, ctx: &egui::Context) {
        if self.schema.lock().unwrap().is_none() {
            return;
        }
        let needs = self.current_page().is_some_and(|page| {
            !page.results_fetched
                && !page.live.definition.trim().is_empty()
                && !page.results.lock().unwrap().running
        });
        if needs {
            self.run_query(ctx);
        }
    }

    pub(crate) fn current_page(&self) -> Option<&QueryPage> {
        let id = self.current.query_id()?;
        self.pages.iter().find(|p| p.live.id == id)
    }

    pub(crate) fn current_page_mut(&mut self) -> Option<&mut QueryPage> {
        let id = self.current.query_id()?;
        self.pages.iter_mut().find(|p| p.live.id == id)
    }

    pub(crate) fn page_results(&self, id: Uuid) -> Option<Arc<Mutex<QueryState>>> {
        self.pages
            .iter()
            .find(|p| p.live.id == id)
            .map(|p| Arc::clone(&p.results))
    }

    /// Creates a new ephemeral query (not yet persisted) and selects it.
    pub(crate) fn add_query_page(&mut self) {
        let now = rpc::now_epoch();
        let query = rpc::Query {
            id: Uuid::new_v4(),
            name: rpc::now_name(),
            created_at: now,
            modified_at: now,
            last_play: now,
            definition: String::new(),
        };
        let id = query.id;
        self.pages.push(QueryPage::ephemeral(query));
        self.select_page(id);
    }

    pub(crate) fn select_page(&mut self, id: Uuid) {
        self.current = CurrentPage::Query(id);
        self.selection.clear();
        self.selection_anchor = None;
    }

    /// Starts an inline rename of `id`, seeding the edit buffer with the current
    /// name. `surface` records where the rename was triggered so only that
    /// surface renders the field.
    pub(crate) fn begin_rename(&mut self, id: Uuid, surface: RenameSurface) {
        let Some(page) = self.pages.iter().find(|p| p.live.id == id) else {
            return;
        };
        self.rename = Some(Rename {
            id,
            buffer: page.live.name.clone(),
            surface,
            take_focus: true,
        });
    }

    /// Commits the in-progress rename. An empty/whitespace-only name is rejected
    /// and treated as a cancel. For a persisted query the new name is pushed to
    /// the backend immediately and mirrored into the saved snapshot, so the
    /// rename doesn't register as an unsaved (blue-dot) change.
    pub(crate) fn commit_rename(&mut self) {
        let Some(state) = self.rename.take() else {
            return;
        };
        let name = state.buffer.trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some(page) = self.pages.iter_mut().find(|p| p.live.id == state.id) else {
            return;
        };
        if page.live.name == name {
            return;
        }
        page.live.name.clone_from(&name);
        if let Some(saved) = page.saved.as_mut() {
            saved.name.clone_from(&name);
            rpc::rename_query(state.id, &name);
        }
    }

    /// Abandons the in-progress rename, restoring the original name.
    pub(crate) fn cancel_rename(&mut self) {
        self.rename = None;
    }

    /// Opens the delete-confirmation modal for `id`.
    pub(crate) fn request_delete(&mut self, id: Uuid) {
        if let Some(page) = self.pages.iter().find(|p| p.live.id == id) {
            self.pending_delete = Some(PendingDelete {
                id,
                name: page.live.name.clone(),
                unsaved: page.unsaved(),
            });
        }
    }

    /// Deletes a query: drops its page, deletes it on the backend if it was
    /// persisted, and — if it was the open page — navigates to the top-listed
    /// (most-recently-created) remaining query, or the welcome page if none.
    pub(crate) fn delete_query(&mut self, id: Uuid) {
        let was_persisted = self
            .pages
            .iter()
            .find(|p| p.live.id == id)
            .is_some_and(QueryPage::is_persisted);
        self.pages.retain(|p| p.live.id != id);
        if was_persisted {
            rpc::delete_query(id);
        }
        if self.rename.as_ref().is_some_and(|r| r.id == id) {
            self.rename = None;
        }
        if self.current.query_id() == Some(id) {
            self.current = self
                .pages
                .iter()
                .max_by_key(|p| p.live.created_at)
                .map_or(CurrentPage::Welcome, |p| CurrentPage::Query(p.live.id));
            self.selection.clear();
            self.selection_anchor = None;
        }
    }

    /// Renders the delete-confirmation modal when a delete is pending. Confirming
    /// performs the delete; cancelling (button, backdrop click, or Esc) dismisses.
    pub(crate) fn render_delete_confirm(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_delete.as_ref() else {
            return;
        };
        let id = pending.id;
        let name = pending.name.clone();
        let unsaved = pending.unsaved;
        let mut confirm = false;
        let mut cancel = false;

        let modal = egui::Modal::new(egui::Id::new("delete_query_confirm")).show(ctx, |ui| {
            ui.set_max_width(280.0);
            ui.heading("Delete query");
            ui.add_space(8.0);
            ui.label(format!("Delete \u{201c}{name}\u{201d}?"));
            if unsaved {
                ui.add_space(4.0);
                ui.colored_label(ACCENT_BLUE, "This query has unsaved changes.");
            }
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Delete").color(egui::Color32::WHITE),
                        )
                        .fill(page::DELETE_RED),
                    )
                    .clicked()
                {
                    confirm = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });

        if modal.should_close() {
            cancel = true;
        }
        if confirm {
            self.pending_delete = None;
            self.delete_query(id);
        } else if cancel {
            self.pending_delete = None;
        }
    }

    /// Compiles and runs the current page's live query, replacing its results.
    pub(crate) fn run_query(&mut self, ctx: &egui::Context) {
        let Some((results, definition)) = self
            .current_page()
            .map(|p| (Arc::clone(&p.results), p.live.definition.clone()))
        else {
            return;
        };
        let ctx = ctx.clone();

        self.selection.clear();
        self.selection_anchor = None;
        if let Some(page) = self.current_page_mut() {
            page.results_fetched = true;
        }

        {
            let mut s = results.lock().unwrap();
            s.rows.clear();
            s.columns.clear();
            s.error = None;
            s.running = true;
            s.track_id_column = None;
            s.lineage_done = false;
            s.needs_revalidation = true;
        }

        // Compile the user's Querydown into DuckDB SQL before running it.
        let compiled = {
            let schema = self.schema.lock().unwrap();
            match schema.as_deref() {
                Some(schema_json) => compile::querydown_to_duckdb(&definition, schema_json),
                None => Err("Schema not loaded yet. Please try again in a moment.".to_string()),
            }
        };
        let sql = match compiled {
            Ok(compiled) => {
                let mut s = results.lock().unwrap();
                s.columns = compiled.columns;
                compiled.sql
            }
            Err(e) => {
                let mut s = results.lock().unwrap();
                s.error = Some(e);
                s.running = false;
                drop(s);
                ctx.request_repaint();
                return;
            }
        };

        lineage::detect_track_column(sql.clone(), Arc::clone(&results), ctx.clone());
        http::run_query(sql, &results, &ctx);
    }

    /// Persists the current page's live query. Inserts it if it's new, otherwise
    /// updates its definition; either way bumps `modified_at`.
    pub(crate) fn save_current(&mut self) {
        let Some(page) = self.current_page_mut() else {
            return;
        };
        page.live.modified_at = rpc::now_epoch();
        let snapshot = page.live.clone();
        let was_persisted = page.saved.is_some();
        page.saved = Some(snapshot.clone());
        if was_persisted {
            rpc::update_definition(snapshot.id, &snapshot.definition, snapshot.modified_at);
        } else {
            rpc::add_query(&snapshot);
        }
    }

    /// Plays a track that was located on `source_page`, recording the play
    /// against that query's `last_play`.
    pub(crate) fn play_track(
        &mut self,
        source_page: Uuid,
        index: usize,
        id: &str,
        ctx: &egui::Context,
    ) {
        {
            let mut ct = self.current_track.lock().unwrap();
            *ct = Some(CurrentTrack {
                source_page,
                id: id.to_string(),
                row_index: Some(index),
                title: None,
                artist_names: Vec::new(),
            });
        }
        self.audio.load(id);
        self.audio.play();
        http::fetch_track_metadata(id, &self.current_track, ctx);

        // Record the play on the originating query. Updating both live and saved
        // equally keeps `last_play` out of the unsaved comparison.
        let now = rpc::now_epoch();
        if let Some(page) = self.pages.iter_mut().find(|p| p.live.id == source_page) {
            page.live.last_play = now;
            if let Some(saved) = page.saved.as_mut() {
                saved.last_play = now;
            }
            if page.is_persisted() {
                rpc::record_play(source_page, now);
            }
        }
    }
}
