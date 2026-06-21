use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::egui::emath::TSTransform;
use uuid::Uuid;

mod audio;
mod builder;
mod button;
mod columns;
mod compile;
mod field_layout;
mod format;
mod http;
mod icons;
mod lineage;
mod menu_bar;
mod now_playing;
mod organizer;
mod page;
mod query_def;
mod results;
mod rpc;
mod schema;
#[cfg(target_arch = "wasm32")]
mod web;
mod welcome;

use audio::AudioPlayer;
use builder::{ManageScope, PresetEdit, PresetSave};
use columns::ColumnMetadata;
use field_layout::FieldLayout;
use now_playing::CurrentTrack;
use organizer::Organizer;
use page::{CurrentPage, QueryPage};
use query_def::{QueryDefinition, Section, SectionContent};

pub(crate) const ORGANIZER_WIDTH: f32 = 200.0;
const ORGANIZER_ANIM_TIME: f32 = 0.1;
/// At or above this viewport width the organizer becomes a persistent left panel
/// (reserving its own space) instead of a modal drawer that overlays the content.
pub(crate) const PERSISTENT_ORGANIZER_MIN_WIDTH: f32 = 500.0;
/// Leftward pointer velocity (px/s) that counts as a swipe-to-close flick,
/// even if the cumulative drag distance is small.
pub(crate) const ORGANIZER_SWIPE_VELOCITY: f32 = 400.0;
/// Static-friction scale for the drawer drag. Small finger movements (well
/// below this) produce ~no drawer motion, so vertical scroll gestures inside
/// the drawer aren't mistaken for a close-swipe. Past a few times this value,
/// the drawer tracks the finger 1:1 (offset by a constant amount).
pub(crate) const ORGANIZER_DRAG_FRICTION: f32 = 16.0;

pub(crate) const ACCENT_BLUE: egui::Color32 = egui::Color32::from_rgb(0xBC, 0xD0, 0xEA);

pub fn setup_fonts(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::light());
    let mut fonts = egui::FontDefinitions::default();

    // Bundle our own faces so the UI doesn't depend on system-installed fonts:
    // Noto Sans for proportional text, Noto Sans Mono for monospace. Insert each
    // at the front of its family so it's the primary face while keeping egui's
    // default fallbacks (emoji/CJK coverage) behind it.
    fonts.font_data.insert(
        "noto-sans".into(),
        egui::FontData::from_static(include_bytes!("../fonts/NotoSans-Regular.ttf")).into(),
    );
    fonts.font_data.insert(
        "noto-sans-mono".into(),
        egui::FontData::from_static(include_bytes!("../fonts/NotoSansMono-Regular.ttf")).into(),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "noto-sans".into());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "noto-sans-mono".into());

    ctx.set_fonts(fonts);

    // Register the Material Symbols outline font (adds it as a named family and
    // as a low-priority fallback on the proportional family).
    egui_material_icons::initialize(ctx);
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
    /// Which query-builder section (filter/sort/display) is open, if any. Applies
    /// to sectioned mode only.
    pub(crate) builder_section: Option<Section>,
    /// Whether the full-querydown editor panel is open. Applies to full-querydown
    /// mode only (the "Querydown" toolbar toggle).
    pub(crate) full_editor_open: bool,
    /// The active section button's embedded "⋮" menu trigger, captured each
    /// frame by the menu bar so the builder panel (rendered just after) can
    /// anchor that section's options popup to the toolbar button.
    pub(crate) section_menu_anchor: Option<egui::Response>,
    /// Last measured natural height of the query-builder content, used to size
    /// the builder panel to fit its contents (see `render_builder_panel`).
    /// `None` until the first frame has measured it.
    pub(crate) builder_content_height: Option<f32>,
    /// All saved presets (every table and section), fetched at startup and kept
    /// in sync locally as the user adds/edits/deletes them.
    pub(crate) presets: Vec<rpc::Preset>,
    /// Inbox for an in-flight `preset.list`; drained into `presets` on the next frame.
    pub(crate) loaded_presets: Arc<Mutex<Option<Vec<rpc::Preset>>>>,
    pub(crate) presets_fetch_started: bool,
    /// The in-progress "save as preset" naming dialog, if any.
    pub(crate) preset_save: Option<PresetSave>,
    /// The in-progress inline preset edit, if any.
    pub(crate) preset_edit: Option<PresetEdit>,
    /// Scope of the open manage-presets modal, if any.
    pub(crate) manage_presets: Option<ManageScope>,
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
            builder_section: None,
            full_editor_open: false,
            section_menu_anchor: None,
            builder_content_height: None,
            presets: Vec::new(),
            loaded_presets: Arc::new(Mutex::new(None)),
            presets_fetch_started: false,
            preset_save: None,
            preset_edit: None,
            manage_presets: None,
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.bootstrap(&ctx);
        self.drain_loaded_queries();
        self.drain_loaded_presets();

        let panel_fill = ui.style().visuals.panel_fill;
        let persistent = ctx.viewport_rect().width() >= PERSISTENT_ORGANIZER_MIN_WIDTH;

        self.ensure_current_results(&ctx);

        // On wide screens the organizer is a persistent left panel that reserves
        // its own space, so it must be added before the top/central panels for
        // them to lay out in the remaining area.
        if persistent {
            self.render_persistent_organizer(ui, panel_fill);
        }

        // Top panels: each page type renders its own bar (including the explorer
        // button). Top/bottom panels must be added before the central panel.
        match self.current {
            CurrentPage::Query(_) => {
                self.render_menu_bar(ui);
                // In full mode the panel is the full-query editor (gated by its
                // own toggle); in sectioned mode it's the open builder section.
                let full_mode = self
                    .current_page()
                    .is_some_and(|p| p.live.definition.is_full());
                let show_builder = if full_mode {
                    self.full_editor_open
                } else {
                    self.builder_section.is_some()
                };
                if show_builder {
                    self.render_builder_panel(ui);
                }
            }
            CurrentPage::Welcome => self.render_welcome_bar(ui),
        }
        self.render_now_playing(ui);
        self.maybe_revalidate_current_track_index();

        // Central panel.
        match self.current {
            CurrentPage::Query(_) => self.render_results(ui),
            CurrentPage::Welcome => welcome::render_welcome_center(ui),
        }

        if persistent {
            // The persistent panel reserves real layout space, so the content
            // mustn't also be slid aside by the modal drawer's transform.
            ctx.set_transform_layer(egui::LayerId::background(), TSTransform::IDENTITY);
        } else {
            // On narrow screens the organizer is a modal drawer that slides over
            // the content (with a dimming scrim and swipe-to-close), pushing the
            // background layer aside as it opens.
            let progress = self.organizer_progress(&ctx);
            let organizer_offset = progress * ORGANIZER_WIDTH;
            ctx.set_transform_layer(
                egui::LayerId::background(),
                TSTransform::from_translation(egui::vec2(organizer_offset, 0.0)),
            );

            // Render while dragging even at progress == 0 so the widget that owns
            // the in-flight drag stays mounted and `drag_stopped` fires on release.
            if progress > 0.0 || self.organizer.dragging {
                self.render_organizer(&ctx, progress, panel_fill);
            }
        }

        // Modals float above everything else.
        self.render_delete_confirm(&ctx);
        self.render_preset_save_modal(&ctx);
        self.render_manage_presets_modal(&ctx);
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
        if !self.presets_fetch_started {
            self.presets_fetch_started = true;
            rpc::list_presets(Arc::clone(&self.loaded_presets), ctx.clone());
        }
    }

    /// If a `preset.list` response has arrived, replace the local preset list.
    fn drain_loaded_presets(&mut self) {
        if let Some(list) = self.loaded_presets.lock().unwrap().take() {
            self.presets = list;
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
                && page.live.definition.is_runnable()
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
            definition: self.definition_for_base("track".to_string()),
        };
        let id = query.id;
        self.pages.push(QueryPage::ephemeral(query));
        self.select_page(id);
    }

    /// Creates a new ephemeral query copied from `id` and selects it. The copy's
    /// definition is taken from the source's `live` version, so any unsaved edits
    /// are carried into the duplicate; its name is computed like a freshly created
    /// query's (see [`rpc::now_name`]).
    pub(crate) fn duplicate_query(&mut self, id: Uuid) {
        let Some(source) = self.pages.iter().find(|p| p.live.id == id) else {
            return;
        };
        let now = rpc::now_epoch();
        let query = rpc::Query {
            id: Uuid::new_v4(),
            name: rpc::now_name(),
            created_at: now,
            modified_at: now,
            last_play: now,
            definition: source.live.definition.clone(),
        };
        let new_id = query.id;
        self.pages.push(QueryPage::ephemeral(query));
        self.select_page(new_id);
    }

    /// A fresh definition for `base`, seeded with every default preset scoped to
    /// that base table. The filter section accepts any number of default presets;
    /// sort and display take only one, so the first default of each (presets are
    /// listed by name) wins.
    pub(crate) fn definition_for_base(&self, base: String) -> QueryDefinition {
        let mut def = QueryDefinition {
            base,
            ..Default::default()
        };
        for preset in &self.presets {
            if !preset.is_default || preset.base_table != def.base {
                continue;
            }
            match preset.section {
                Section::Filter => def.filter.presets.push(preset.id),
                Section::Sort if def.sort == SectionContent::default() => {
                    def.sort = SectionContent::Preset(preset.id);
                }
                Section::Display if def.display == SectionContent::default() => {
                    def.display = SectionContent::Preset(preset.id);
                }
                Section::Sort | Section::Display => {}
            }
        }
        def
    }

    /// Converts the current page's sectioned query into full-querydown mode by
    /// concatenating its resolved parts into one query, and opens the full-query
    /// editor so the result is immediately visible. A no-op if the query is
    /// already in full mode.
    pub(crate) fn convert_current_to_full(&mut self) {
        let full = match self.current_page() {
            Some(page) if !page.live.definition.is_full() => {
                page.live.definition.to_full_query(&self.presets)
            }
            _ => return,
        };
        if let Some(page) = self.current_page_mut() {
            page.live.definition.full = Some(full);
        }
        self.full_editor_open = true;
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

    /// Discards a query's unsaved edits, restoring its `live` version from the
    /// last-saved snapshot. A no-op for a never-saved query (nothing to revert
    /// to). Also cancels any in-progress rename of the query, since reverting
    /// restores its saved name.
    pub(crate) fn revert_query(&mut self, id: Uuid) {
        let Some(page) = self.pages.iter_mut().find(|p| p.live.id == id) else {
            return;
        };
        let Some(saved) = page.saved.clone() else {
            return;
        };
        page.live = saved;
        if self.rename.as_ref().is_some_and(|r| r.id == id) {
            self.rename = None;
        }
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

        // Resolve the four query parts into per-section Querydown source, then
        // compile it into DuckDB SQL before running it.
        let compiled = {
            let schema = self.schema.lock().unwrap();
            match (definition.assemble(&self.presets), schema.as_deref()) {
                (Err(e), _) => Err(e),
                (_, None) => {
                    Err("Schema not loaded yet. Please try again in a moment.".to_string())
                }
                (Ok(source), Some(schema_json)) => {
                    compile::querydown_to_duckdb(&source, schema_json)
                }
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
