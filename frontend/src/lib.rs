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
mod text_input;
#[cfg(target_arch = "wasm32")]
mod web;
mod welcome;

use audio::AudioPlayer;
use builder::{PresetEdit, PresetSave};
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
pub(crate) const HOVER_BLUE: egui::Color32 = egui::Color32::from_rgb(0x77, 0xA5, 0xCE);

/// Margin kept on each side between the "View SQL" modal and the viewport edges,
/// so the modal shrinks to fit small windows instead of touching the edges.
const VIEW_SQL_VIEWPORT_MARGIN: f32 = 24.0;
/// Height reserved below the "View SQL" modal's scroll area for its footer — the
/// button row plus the gap above it — so the footer stays pinned just under the
/// SQL at a constant height while the scroll area absorbs any viewport shrinkage.
const VIEW_SQL_FOOTER_HEIGHT: f32 = 40.0;

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
    /// In-progress inline edits of presets, keyed by preset id. An entry exists
    /// once a preset has been expanded for editing, and persists thereafter (even
    /// when the preset is collapsed, the builder is closed, or the user navigates
    /// to a different query) so unsaved changes are never silently dropped. A
    /// preset is "dirty" when its entry differs from the saved preset.
    pub(crate) preset_edits: HashMap<Uuid, PresetEdit>,
    /// Which preset is currently expanded for editing in the builder, if any.
    /// Only one expands at a time. This expanded state is ephemeral to the open
    /// builder area: it is cleared whenever a builder section is opened, closed,
    /// or switched.
    pub(crate) expanded_preset: Option<Uuid>,
    /// Set when a builder section is (re)opened, so the builder focuses the right
    /// input once. Consumed on the next builder frame.
    pub(crate) builder_focus: bool,
    /// Whether the manage-presets modal is open.
    pub(crate) manage_presets: bool,
    /// The contents of the "View SQL" modal when open: either the pretty-printed
    /// SQL the current query would send to the query API, or a compile-error
    /// message to show in its place. `None` when the modal is closed.
    pub(crate) view_sql: Option<String>,
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
            preset_edits: HashMap::new(),
            expanded_preset: None,
            builder_focus: false,
            manage_presets: false,
            view_sql: None,
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
        self.render_view_sql_modal(&ctx);
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

    /// Compiles the current page's live query into the `DuckDB` SQL that would be
    /// sent to the query API, returning the SQL or a compile-error message. This
    /// mirrors the compilation [`run_query`](Self::run_query) performs, but stops
    /// at the SQL string and runs nothing.
    fn current_query_sql(&self) -> Result<String, String> {
        let definition = self
            .current_page()
            .map(|p| p.live.definition.clone())
            .ok_or_else(|| "No query is open.".to_string())?;
        let presets = self.effective_presets();
        let schema = self.schema.lock().unwrap();
        match (definition.assemble(&presets), schema.as_deref()) {
            (Err(e), _) => Err(e),
            (_, None) => Err("Schema not loaded yet. Please try again in a moment.".to_string()),
            (Ok(source), Some(schema_json)) => {
                compile::querydown_to_duckdb(&source, schema_json).map(|c| c.sql)
            }
        }
    }

    /// Opens the "View SQL" modal, populating it with the current query's compiled
    /// SQL (pretty-formatted) or, if compilation fails, the error message.
    pub(crate) fn open_view_sql(&mut self) {
        self.view_sql = Some(match self.current_query_sql() {
            Ok(sql) => format_sql(&sql),
            Err(e) => e,
        });
    }

    /// Renders the "View SQL" modal when open: a scrollable, selectable view of the
    /// compiled SQL with a button to copy it to the clipboard. Closing (button,
    /// backdrop click, or Esc) dismisses it. The modal sizes itself to the viewport
    /// so it stays usable on small windows.
    pub(crate) fn render_view_sql_modal(&mut self, ctx: &egui::Context) {
        let Some(sql) = self.view_sql.as_ref() else {
            return;
        };
        let sql = sql.clone();
        let mut close = false;
        // Size the modal to the viewport so it stays usable on small (e.g. mobile)
        // windows. Width caps at a comfortable reading width but shrinks to fit
        // narrow screens. Height is bounded by `set_max_height` below so the modal
        // shrinks to fit short screens; the scroll area then absorbs all of that
        // shrinkage while the heading and footer keep their natural height.
        let screen = ctx.content_rect();
        let width = (screen.width() - 2.0 * VIEW_SQL_VIEWPORT_MARGIN).clamp(120.0, 560.0);
        let max_height = (screen.height() - 2.0 * VIEW_SQL_VIEWPORT_MARGIN).max(120.0);
        let modal = egui::Modal::new(egui::Id::new("view_sql")).show(ctx, |ui| {
            ui.set_width(width);
            ui.set_max_height(max_height);
            ui.heading("SQL");
            ui.add_space(8.0);
            // Fill the space between the heading and the fixed-height footer with the
            // scrollable SQL. Deriving its height from the remaining `available_height`
            // (which already accounts for the heading and the `set_max_height` bound)
            // keeps the footer pinned just below the SQL and the whole modal inside the
            // viewport, regardless of the heading's font metrics or the screen size.
            let scroll_height = (ui.available_height() - VIEW_SQL_FOOTER_HEIGHT).clamp(48.0, 480.0);
            // Vertical-only scrolling with wrapped text: long SQL lines wrap to the
            // modal width rather than scrolling sideways (better on small/mobile
            // viewports), and dropping the horizontal scrollbar lets the scroll area
            // shrink to the content so the footer hugs the SQL instead of floating
            // below a reserved scrollbar.
            egui::ScrollArea::vertical()
                .max_height(scroll_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(&sql).monospace())
                            .selectable(true)
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                });
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(sql.clone());
                }
            });
        });
        if close || modal.should_close() {
            self.view_sql = None;
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
        let presets = self.effective_presets();
        let compiled = {
            let schema = self.schema.lock().unwrap();
            match (definition.assemble(&presets), schema.as_deref()) {
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

/// Pretty-formats `DuckDB` `sql` for display in the "View SQL" modal, using
/// `polyglot-sql`. Statements are joined with blank lines (the formatter returns
/// one per statement). Falls back to the raw SQL if it can't be parsed/formatted,
/// so the user always sees something they can copy.
fn format_sql(sql: &str) -> String {
    match polyglot_sql::format(sql, polyglot_sql::DialectType::DuckDB) {
        Ok(statements) if !statements.is_empty() => statements.join(";\n\n"),
        _ => sql.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_sql;

    #[test]
    fn format_sql_pretty_prints() {
        // A single-line query gains line breaks once pretty-formatted.
        let out = format_sql("select a, b from t where a > 1");
        assert!(out.contains('\n'), "expected multi-line output, got: {out}");
        assert!(out.to_uppercase().contains("SELECT"));
    }
}

#[cfg(test)]
mod view_sql_snapshot_tests {
    //! Snapshot tests for the "View SQL" modal's responsive sizing, driving the
    //! real [`App::render_view_sql_modal`] so the snapshots track the actual modal
    //! code. The invariant: the footer (button row) keeps a constant height and
    //! stays hugging the SQL, while the modal as a whole shrinks to fit short
    //! viewports. `tall` shows the whole (wrapped) query; `short` shows it clipped
    //! into a modal squeezed by the viewport. Generate or refresh with
    //! `UPDATE_SNAPSHOTS=1 cargo test -p frontend`.

    use std::cell::Cell;

    use eframe::egui;

    use crate::App;

    /// A long, wide block of SQL: tall enough to overflow the scroll area and with a
    /// line long enough to exercise wrapping at a narrow modal width.
    const SAMPLE_SQL: &str = "\
WITH \"cte0\" AS (\n  \
  SELECT\n    \
    \"credit\".\"track\" AS \"pk\"\n  \
  FROM \"credit\"\n  \
  JOIN \"artist\"\n    \
    ON \"credit\".\"artist\" = \"artist\".\"id\"\n  \
  WHERE\n    \
    COALESCE(CONTAINS(LOWER(STRIP_ACCENTS(\"artist\".\"name\")), LOWER(STRIP_ACCENTS('x'))), FALSE)\n  \
  GROUP BY\n    \
    \"credit\".\"track\"\n\
), \"cte1\" AS (\n  \
  SELECT\n    \
    \"play\".\"track\" AS \"pk\",\n    \
    COUNT(*) AS \"v1\"\n  \
  FROM \"play\"\n  \
  GROUP BY\n    \
    \"play\".\"track\"\n\
)\nSELECT\n  \
  \"track\".\"id\" AS \"id\",\n  \
  \"track\".\"title\" AS \"title\"\nFROM \"track\"\n\
LEFT JOIN \"cte0\" ON \"cte0\".\"pk\" = \"track\".\"id\"\n\
LEFT JOIN \"cte1\" ON \"cte1\".\"pk\" = \"track\".\"id\"\n\
ORDER BY\n  \"track\".\"title\"";

    /// Renders the modal at `size` (logical points) into `view_sql_modal/<name>`.
    fn snapshot(name: &str, size: egui::Vec2) {
        let mut app = App {
            view_sql: Some(SAMPLE_SQL.to_owned()),
            ..Default::default()
        };
        // The closure runs once before we can bind fonts; do that on the first frame
        // and paint the modal from the second (font changes take effect the following
        // frame), mirroring the filter-builder snapshots. The modal is a `ctx`-level
        // floating area, so we drive it via `ui.ctx()` rather than the passed `ui`.
        let fonts_ready = Cell::new(false);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(size)
            .with_pixels_per_point(2.0)
            .build_ui(move |ui| {
                if !fonts_ready.replace(true) {
                    crate::setup_fonts(ui.ctx());
                    return;
                }
                app.render_view_sql_modal(ui.ctx());
            });
        harness.run();
        harness.snapshot(format!("view_sql_modal/{name}"));
    }

    #[test]
    fn tall() {
        // Tall enough that the whole (wrapped) query fits: the footer hugs the SQL.
        snapshot("tall", egui::vec2(480.0, 620.0));
    }

    #[test]
    fn short() {
        // Short viewport: the modal shrinks, the SQL clips, the footer stays put.
        snapshot("short", egui::vec2(480.0, 260.0));
    }
}
