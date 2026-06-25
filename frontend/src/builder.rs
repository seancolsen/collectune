//! The query-builder panel: per-section editors for the filter, sorting, and
//! display parts of a query. Each section combines a hand-written Querydown
//! fragment ("custom") with saved presets. A shared layout (see
//! [`App::builder_widget`]) lays out an optional custom input beside a row of
//! right-aligned preset "tabs"; an expanded tab reveals an inline editor below,
//! connected to the tab like a tabbed interface.

use eframe::egui;
use uuid::Uuid;

use crate::App;
use crate::button::Button;
use crate::icons;
use crate::now_playing::menu_item;
use crate::page::DELETE_RED;
use crate::query_def::{BuiltinPreset, QueryDefinition, Section, SectionContent};
use crate::rpc::{self, Preset};

/// Background of an expanded preset tab and its detail panel.
const PRESET_BG: egui::Color32 = egui::Color32::from_rgb(0xF3, 0xE3, 0xFB);

/// Smallest height the builder panel will shrink to, so an empty/"no query"
/// state still has a sane size. Kept just above a single content line so a
/// one-line builder doesn't reserve unused vertical space (which would
/// otherwise eat into the query results area).
const MIN_BUILDER_HEIGHT: f32 = 40.0;
/// Vertical margin of the panel's `Frame::side_top_panel` (`symmetric(8, 2)`),
/// added around the measured content so the panel is exactly tall enough.
const FRAME_V_MARGIN: f32 = 4.0;

/// Minimum width the custom-filter text input must retain beside any preset
/// tabs. Below this, the tabs move to their own row below the input.
const MIN_FILTER_INPUT_WIDTH: f32 = 400.0;

/// Inner padding of a preset tab's name area, chosen so its height matches a
/// one-line text input (whose padding is the same), keeping a collapsed tab
/// aligned with an adjacent input.
const TAB_PADDING: egui::Margin = egui::Margin {
    left: 7,
    right: 7,
    top: 4,
    bottom: 4,
};
/// Corner radius of an expanded tab's (top) corners and the detail panel.
const TAB_RADIUS: u8 = 6;
/// Vertical gap between the tab row and the detail panel. The expanded tab's
/// background is extended down across this gap to bridge into the panel.
const TAB_GAP: f32 = 6.0;
/// How far the expanded tab's background extends past the top of the detail
/// panel. The detail panel is painted in front of this background, so the
/// overlap can run under the panel's rounded top corners with no gap showing
/// between the name area and the detail panel.
const TAB_BG_OVERLAP: f32 = 12.0;
/// Width of the name field in a preset's detail editor.
const NAME_FIELD_WIDTH: f32 = 180.0;

/// The "save as preset" naming dialog: which section is being saved and the
/// Querydown fragment to store.
pub(crate) struct PresetSave {
    pub(crate) section: Section,
    pub(crate) name: String,
    pub(crate) definition: String,
    /// Whether the new preset should apply by default to new queries.
    pub(crate) is_default: bool,
    /// Set on the first frame so the name field grabs focus once.
    pub(crate) take_focus: bool,
}

/// An in-progress edit of a saved preset's definition. The buffers are committed
/// to the preset (and the backend) on save, or discarded on revert. These live
/// keyed by preset id in [`App::preset_edits`], persisting across collapse,
/// builder close, and query navigation.
pub(crate) struct PresetEdit {
    pub(crate) name: String,
    pub(crate) definition: String,
    pub(crate) is_default: bool,
}

/// A choice from a custom block's inline `⋮` menu.
enum CustomChoice {
    Clear,
    Save,
    /// Replace the custom content with the chosen preset (sort/display only).
    Use(Uuid),
}

/// A collapsed preset "tab" (name area) rendered in a builder's tab row.
struct TabInfo {
    /// The preset's id (unused for the built-in tab).
    id: Uuid,
    /// The name shown in the tab.
    name: String,
    /// Whether the preset has unsaved edits (shows the red star marker).
    dirty: bool,
    /// Whether the preset is currently expanded (gets the pink background).
    expanded: bool,
    /// A built-in (Shuffle) tab: no chevron, never expands, carries a Reshuffle
    /// button instead of an editor.
    builtin: bool,
}

/// What a click in the tab row produced.
#[derive(Clone, Copy)]
enum TabClick {
    /// A user preset's name area was clicked (toggle its expansion).
    Toggle(Uuid),
    /// The built-in tab's Reshuffle button was clicked.
    Reshuffle,
}

/// Whether a section's options menu offers independently toggled (checkbox)
/// entries or mutually exclusive (radio) ones. The filter section can combine
/// several presets, so it uses checkboxes; sort and display pick exactly one
/// thing, so they use radio buttons.
#[derive(Clone, Copy)]
enum ToggleKind {
    Checkbox,
    Radio,
}

/// A built-in (parameterized) preset offered in a section's options menu.
/// Currently only Shuffle exists (for sorting the track table), but the menu
/// renders built-ins as their own group so more can be added later.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BuiltinKind {
    Shuffle,
}

impl BuiltinKind {
    /// The display label shown on the menu row.
    fn name(self) -> &'static str {
        match self {
            BuiltinKind::Shuffle => "Shuffle",
        }
    }

    /// The icon shown on the menu row for this built-in.
    fn icon(self) -> icons::MaterialIcon {
        match self {
            BuiltinKind::Shuffle => icons::SHUFFLE,
        }
    }
}

/// The entry a user clicked in a section's toolbar (gear) options menu. The
/// caller interprets a click per section: the filter toggles the entry, while
/// sort/display select it exclusively.
enum MenuEntry {
    /// The always-present "Custom …" entry.
    Custom,
    /// A built-in preset.
    Builtin(BuiltinKind),
    /// A user-defined preset, by id.
    Preset(Uuid),
}

impl App {
    /// The top panel below the menu bar holding the open builder section.
    pub(crate) fn render_builder_panel(&mut self, ui: &mut egui::Ui) {
        let full_mode = self
            .current_page()
            .is_some_and(|p| p.live.definition.is_full());
        let section = self.builder_section;
        // In sectioned mode the panel shows the open builder section; bail when
        // none is open. (Full mode has no section — it shows the full editor.)
        if !full_mode && section.is_none() {
            return;
        }
        // egui panels fix their height before rendering and clip any overflow;
        // they don't size to content. So we drive the height from the previous
        // frame's measured content height (captured below via the ScrollArea's
        // `content_size`), clamped to a sane minimum and to 60% of the window.
        let max_h = ui.ctx().content_rect().height() * 0.5;
        let content_h = self
            .builder_content_height
            .map_or(max_h, |h| h + FRAME_V_MARGIN);
        let height = content_h.clamp(MIN_BUILDER_HEIGHT, max_h);
        // Edit a local copy of the definition so rendering can freely borrow
        // other parts of `self` (presets, modal state); written back below.
        let mut def = self.current_page().map(|p| p.live.definition.clone());
        let mut run = false;
        // The options popup hangs off the active section's toolbar button, whose
        // menu trigger the menu bar captured into `section_menu_anchor` earlier
        // this frame.
        let trigger = self.section_menu_anchor.clone();
        egui::Panel::top("query_builder")
            .exact_size(height)
            .show_inside(ui, |ui| {
                let output = egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(def) = def.as_mut() else {
                        ui.weak("No query selected.");
                        return;
                    };
                    ui.add_space(6.0);
                    if full_mode {
                        full_builder_ui(ui, def, &mut run);
                    } else if let Some(section) = section {
                        let trigger = trigger.as_ref();
                        match section {
                            Section::Filter => self.filter_builder_ui(ui, def, trigger, &mut run),
                            Section::Sort | Section::Display => {
                                self.single_builder_ui(ui, section, def, trigger, &mut run);
                            }
                        }
                    }
                    ui.add_space(6.0);
                });
                // Feed this frame's natural content height back. The panel's
                // height for *this* pass was derived from the previous frame's
                // measurement (egui panels fix their height before rendering and
                // clip overflow — they don't size to content). When the content
                // height changes — most visibly when expanding a preset — that
                // stale height would clip the taller content for one painted
                // frame, a jarring flash. So rather than `request_repaint` (which
                // paints this wrong-height pass, then draws a corrected one), we
                // `request_discard`: it throws this pass's output away *before*
                // it is painted and re-runs immediately with the corrected
                // height, so the intermediate frame is never shown. The 0.5px
                // tolerance limits this to genuine changes, so we never exhaust
                // egui's multi-pass budget in steady state.
                let measured = output.content_size.y;
                if self
                    .builder_content_height
                    .is_none_or(|h| (h - measured).abs() > 0.5)
                {
                    self.builder_content_height = Some(measured);
                    ui.ctx().request_discard("builder height changed");
                }
            });
        if let Some(def) = def
            && let Some(page) = self.current_page_mut()
        {
            page.live.definition = def;
        }
        if run {
            self.run_query(ui.ctx());
        }
    }

    /// The filter builder: a custom block combined (via AND) with any number of
    /// presets, the latter shown as right-aligned tabs beside (or below) the
    /// input.
    fn filter_builder_ui(
        &mut self,
        ui: &mut egui::Ui,
        def: &mut QueryDefinition,
        trigger: Option<&egui::Response>,
        run: &mut bool,
    ) {
        let base_chosen = def.is_runnable();
        let focus = std::mem::take(&mut self.builder_focus);
        let mut custom_choice = None;
        let mut custom_run = false;
        let mut save_as = None;

        let presets = def.filter.presets.clone();
        // Drop a stale expansion (e.g. the preset was removed elsewhere).
        if let Some(eid) = self.expanded_preset
            && !presets.contains(&eid)
        {
            self.expanded_preset = None;
        }
        let expanded = self.expanded_preset;
        let tabs: Vec<TabInfo> = presets
            .iter()
            .map(|id| TabInfo {
                id: *id,
                name: self.preset_name(*id),
                dirty: self.preset_dirty(*id),
                expanded: expanded == Some(*id),
                builtin: false,
            })
            .collect();
        let expanded_id = expanded.filter(|e| presets.contains(e));

        let click = self.builder_widget(
            ui,
            true,
            |ui| {
                custom_choice = filter_custom_input(
                    ui,
                    &mut def.filter.custom,
                    base_chosen,
                    focus,
                    &mut custom_run,
                );
            },
            &tabs,
            expanded_id,
            run,
        );
        *run |= custom_run;

        // The toolbar (gear) options menu: a flat checkbox list. "Custom filter"
        // is always checked (and disabled, so it can't be removed); each user
        // preset's checkbox toggles its membership in the filter. The filter
        // section has no built-in presets.
        let preset_rows: Vec<(Uuid, String, bool)> = self
            .presets_for(&def.base, Section::Filter)
            .into_iter()
            .map(|(id, name)| {
                let checked = def.filter.presets.contains(&id);
                (id, name, checked)
            })
            .collect();
        let custom_label = format!("Custom {}", Section::Filter.label().to_lowercase());
        let menu_choice = section_options_menu(
            trigger,
            ToggleKind::Checkbox,
            &custom_label,
            true,
            false,
            &[],
            &preset_rows,
        );

        // Apply collected actions.
        match custom_choice {
            Some(CustomChoice::Clear) => def.filter.custom.clear(),
            Some(CustomChoice::Save) => save_as = Some(def.filter.custom.clone()),
            Some(CustomChoice::Use(_)) | None => {}
        }
        if let Some(TabClick::Toggle(id)) = click {
            self.toggle_expand(id);
        }
        match menu_choice {
            Some(MenuEntry::Preset(id)) => {
                // Toggle the preset's membership in the filter.
                if def.filter.presets.contains(&id) {
                    def.filter.presets.retain(|p| *p != id);
                    if self.expanded_preset == Some(id) {
                        self.expanded_preset = None;
                    }
                } else {
                    def.filter.presets.push(id);
                }
                *run = true;
            }
            // "Custom filter" is disabled and the filter has no built-ins, so
            // neither is ever returned.
            Some(MenuEntry::Custom | MenuEntry::Builtin(_)) | None => {}
        }
        if let Some(definition) = save_as {
            self.preset_save = Some(PresetSave {
                section: Section::Filter,
                name: String::new(),
                definition,
                is_default: false,
                take_focus: true,
            });
        }
    }

    /// The sort/display builder: the section is either one custom block, one
    /// (collapsible, inline-editable) preset, or the built-in Shuffle preset.
    #[allow(clippy::too_many_lines)]
    fn single_builder_ui(
        &mut self,
        ui: &mut egui::Ui,
        section: Section,
        def: &mut QueryDefinition,
        trigger: Option<&egui::Response>,
        run: &mut bool,
    ) {
        let base_chosen = def.is_runnable();
        let available = self.presets_for(&def.base, section);
        // The built-in Shuffle preset is offered only for sorting the track table.
        let show_shuffle = section == Section::Sort && def.base.eq_ignore_ascii_case("track");
        let focus = std::mem::take(&mut self.builder_focus);

        let mut custom_choice = None;
        let mut custom_run = false;
        let mut save_as = None;
        let mut reshuffle = false;

        // Keep the expansion valid only for the section's current preset.
        let current_preset = match section {
            Section::Sort => &def.sort,
            Section::Display => &def.display,
            Section::Filter => unreachable!("filter uses filter_builder_ui"),
        };
        let preset_id = match current_preset {
            SectionContent::Preset(id) => Some(*id),
            _ => None,
        };
        if self.expanded_preset.is_some() && self.expanded_preset != preset_id {
            self.expanded_preset = None;
        }

        let content = match section {
            Section::Sort => &mut def.sort,
            Section::Display => &mut def.display,
            Section::Filter => unreachable!("filter uses filter_builder_ui"),
        };

        match content {
            SectionContent::Custom(text) => {
                let inline_presets = available.clone();
                self.builder_widget(
                    ui,
                    true,
                    |ui| {
                        let has_text = !text.trim().is_empty();
                        let reserve = crate::button::SIZE + ui.spacing().item_spacing.x;
                        let w = (ui.available_width() - reserve).max(40.0);
                        code_editor(ui, text, w, focus, &mut custom_run);
                        custom_choice = dots_menu(ui, |ui| {
                            let mut choice = None;
                            if menu_item(ui, icons::CLEAR, "Clear", has_text, None).clicked() {
                                choice = Some(CustomChoice::Clear);
                            }
                            if menu_item(
                                ui,
                                icons::SAVE,
                                "Save as preset",
                                has_text && base_chosen,
                                None,
                            )
                            .clicked()
                            {
                                choice = Some(CustomChoice::Save);
                            }
                            if let Some(id) =
                                preset_submenu(ui, "Replace with preset", &inline_presets)
                            {
                                choice = Some(CustomChoice::Use(id));
                            }
                            choice
                        });
                    },
                    &[],
                    None,
                    run,
                );
            }
            SectionContent::Preset(id) => {
                let id = *id;
                let expanded = self.expanded_preset == Some(id);
                let tabs = [TabInfo {
                    id,
                    name: self.preset_name(id),
                    dirty: self.preset_dirty(id),
                    expanded,
                    builtin: false,
                }];
                let expanded_id = expanded.then_some(id);
                let click = self.builder_widget(ui, false, |_ui| {}, &tabs, expanded_id, run);
                if let Some(TabClick::Toggle(_)) = click {
                    self.toggle_expand(id);
                }
            }
            SectionContent::Builtin(builtin) => {
                let tabs = [TabInfo {
                    id: Uuid::nil(),
                    name: builtin.name().to_string(),
                    dirty: false,
                    expanded: false,
                    builtin: true,
                }];
                let click = self.builder_widget(ui, false, |_ui| {}, &tabs, None, run);
                if let Some(TabClick::Reshuffle) = click {
                    reshuffle = true;
                }
            }
        }
        *run |= custom_run;

        // The toolbar (gear) options menu: a flat radio list. The "Custom …" entry
        // is selected when the section holds custom text; below a separator come any
        // built-in presets (their own group), then the user-defined presets.
        // Exactly one entry is selected, matching the section's single content.
        let custom_selected = matches!(content, SectionContent::Custom(_));
        let builtins: Vec<(BuiltinKind, bool)> = if show_shuffle {
            let on = matches!(
                content,
                SectionContent::Builtin(BuiltinPreset::Shuffle { .. })
            );
            vec![(BuiltinKind::Shuffle, on)]
        } else {
            Vec::new()
        };
        let preset_rows: Vec<(Uuid, String, bool)> = available
            .iter()
            .map(|(id, name)| (*id, name.clone(), *content == SectionContent::Preset(*id)))
            .collect();
        let custom_label = format!("Custom {}", section.label().to_lowercase());
        let menu_choice = section_options_menu(
            trigger,
            ToggleKind::Radio,
            &custom_label,
            custom_selected,
            true,
            &builtins,
            &preset_rows,
        );

        // Apply collected actions (re-deriving `content` borrows as needed).
        if reshuffle && let SectionContent::Builtin(builtin) = content {
            builtin.reshuffle();
            *run = true;
        }
        match custom_choice {
            Some(CustomChoice::Clear) => {
                if let SectionContent::Custom(text) = content {
                    text.clear();
                }
            }
            Some(CustomChoice::Save) => {
                if let SectionContent::Custom(text) = content {
                    save_as = Some(text.clone());
                }
            }
            Some(CustomChoice::Use(id)) => {
                *content = SectionContent::Preset(id);
                *run = true;
            }
            None => {}
        }
        match menu_choice {
            Some(MenuEntry::Custom) => {
                // Switch to the custom editor, seeding it with the current
                // preset's or built-in's resolved Querydown so nothing is lost.
                // A no-op when already custom; the resolved text is equivalent, so
                // no re-run is needed.
                let text = match content {
                    SectionContent::Custom(_) => None,
                    SectionContent::Builtin(builtin) => Some(builtin.querydown()),
                    SectionContent::Preset(id) => Some(
                        self.presets
                            .iter()
                            .find(|p| p.id == *id)
                            .map(|p| p.definition.clone())
                            .unwrap_or_default(),
                    ),
                };
                if let Some(text) = text {
                    *content = SectionContent::Custom(text);
                    self.expanded_preset = None;
                }
            }
            Some(MenuEntry::Builtin(BuiltinKind::Shuffle))
                if !matches!(
                    content,
                    SectionContent::Builtin(BuiltinPreset::Shuffle { .. })
                ) =>
            {
                *content = SectionContent::Builtin(BuiltinPreset::shuffle());
                self.expanded_preset = None;
                *run = true;
            }
            Some(MenuEntry::Preset(id)) if *content != SectionContent::Preset(id) => {
                *content = SectionContent::Preset(id);
                self.expanded_preset = None;
                *run = true;
            }
            // The chosen radio is already selected (its guard failed), or nothing
            // was clicked: nothing to do.
            Some(_) | None => {}
        }
        if let Some(definition) = save_as {
            self.preset_save = Some(PresetSave {
                section,
                name: String::new(),
                definition,
                is_default: false,
                take_focus: true,
            });
        }
    }

    /// The shared builder layout. Lays out an optional custom input beside a row
    /// of right-aligned preset `tabs`, wrapping the tabs to their own
    /// right-aligned row below the input when the input would otherwise get too
    /// narrow. When a preset is `expanded_id`, its detail editor is drawn full
    /// width below the row, with the expanded tab's background extended down to
    /// bridge into it (a tabbed look). `custom` draws the custom input (only when
    /// `has_custom`). Returns any tab-row click.
    fn builder_widget(
        &mut self,
        ui: &mut egui::Ui,
        has_custom: bool,
        mut custom: impl FnMut(&mut egui::Ui),
        tabs: &[TabInfo],
        expanded_id: Option<Uuid>,
        run: &mut bool,
    ) -> Option<TabClick> {
        // Reserve a background shape *before* any content so it paints behind the
        // tabs and the detail panel; filled in below once the expanded tab's rect
        // and the row height are known.
        let bg_idx = ui.painter().add(egui::Shape::Noop);

        let mut click = None;
        let mut expanded_rect = None;

        let row_resp = if tabs.is_empty() {
            ui.horizontal_top(|ui| {
                if has_custom {
                    custom(ui);
                }
            })
            .response
        } else {
            let spacing = ui.spacing().item_spacing.x;
            let tabs_total: f32 = tabs.iter().map(|t| tab_est_width(ui, t)).sum::<f32>()
                + spacing * tabs.len() as f32;
            let side_by_side =
                !has_custom || ui.available_width() - tabs_total >= MIN_FILTER_INPUT_WIDTH;
            if side_by_side {
                // Tabs hug the right edge; the input fills the remaining width on
                // the left (right-to-left layout, tabs added in reverse so they
                // read left-to-right).
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    for t in tabs.iter().rev() {
                        let (c, r) = draw_tab(ui, t);
                        click = click.or(c);
                        expanded_rect = expanded_rect.or(r);
                    }
                    if has_custom {
                        custom(ui);
                    }
                })
                .response
            } else {
                let top = ui.horizontal_top(|ui| custom(ui)).response;
                ui.add_space(TAB_GAP);
                let bottom = ui
                    .with_layout(
                        egui::Layout::right_to_left(egui::Align::Min).with_main_wrap(true),
                        |ui| {
                            for t in tabs.iter().rev() {
                                let (c, r) = draw_tab(ui, t);
                                click = click.or(c);
                                expanded_rect = expanded_rect.or(r);
                            }
                        },
                    )
                    .response;
                top.union(bottom)
            }
        };

        if let Some(eid) = expanded_id {
            // Bridge the expanded tab's background down to the detail panel,
            // filling the full row height (so the tab grows with a multiline
            // input) plus the gap, then extending under the detail panel (which
            // paints in front) far enough to sit beneath its rounded top corners
            // so no seam shows. This is purely painted, so it never shifts the
            // detail panel's position.
            if let Some(trect) = expanded_rect {
                let bg = egui::Rect::from_min_max(
                    trect.min,
                    egui::pos2(trect.max.x, row_resp.rect.max.y + TAB_GAP + TAB_BG_OVERLAP),
                );
                ui.painter().set(bg_idx, tab_bg_shape(bg));
            }
            ui.add_space(TAB_GAP);
            self.preset_editor(ui, eid, run);
        }

        click
    }

    /// Renders the inline detail editor for the expanded preset `id`: an editable
    /// name and definition, a revert button (enabled while dirty), a save button
    /// (enabled while dirty and named), and the "Apply by default" checkbox.
    /// Saving commits and triggers a re-run; reverting restores the saved version.
    fn preset_editor(&mut self, ui: &mut egui::Ui, id: Uuid, run: &mut bool) {
        // Seed the edit buffer the first time this preset is expanded; thereafter
        // the persisted buffer (in `preset_edits`) is reused so unsaved changes
        // are never wiped.
        if !self.preset_edits.contains_key(&id) {
            self.begin_preset_edit(id);
        }
        let dirty = self.preset_dirty(id);
        let Some(mut edit) = self.preset_edits.remove(&id) else {
            return;
        };
        let mut save = false;
        let mut revert = false;
        section_frame(ui, PRESET_BG, |ui| {
            ui.horizontal(|ui| {
                crate::text_input::add(
                    ui,
                    egui::TextEdit::singleline(&mut edit.name)
                        .hint_text("Preset name")
                        .desired_width(NAME_FIELD_WIDTH),
                );
                revert = Button::icon(icons::REVERT)
                    .enabled(dirty)
                    .show(ui)
                    .clicked();
                save = Button::icon(icons::SAVE)
                    .enabled(dirty && !edit.name.trim().is_empty())
                    .show(ui)
                    .clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut edit.is_default, "Apply by default");
                });
            });
            let w = ui.available_width();
            sized_multiline(ui, &mut edit.definition, w);
        });
        self.preset_edits.insert(id, edit);
        if revert {
            // Re-seed from the saved preset, discarding the in-progress edit.
            self.begin_preset_edit(id);
        }
        if save {
            self.commit_preset_edit(id);
            *run = true;
        }
    }

    /// Toggles the expansion of preset `id` (collapsing any other expanded one).
    fn toggle_expand(&mut self, id: Uuid) {
        if self.expanded_preset == Some(id) {
            self.expanded_preset = None;
        } else {
            self.expanded_preset = Some(id);
        }
    }

    /// The display name of a preset, or a placeholder if it no longer exists.
    fn preset_name(&self, id: Uuid) -> String {
        self.presets
            .iter()
            .find(|p| p.id == id)
            .map_or_else(|| "(missing preset)".to_string(), |p| p.name.clone())
    }

    /// Whether the preset `id` has an in-progress edit that differs from its
    /// saved version (so the save/revert buttons enable and the star shows).
    fn preset_dirty(&self, id: Uuid) -> bool {
        let Some(edit) = self.preset_edits.get(&id) else {
            return false;
        };
        match self.presets.iter().find(|p| p.id == id) {
            Some(p) => {
                p.name != edit.name
                    || p.definition != edit.definition
                    || p.is_default != edit.is_default
            }
            None => true,
        }
    }

    /// Saved presets matching a base table and section, as `(id, name)`.
    fn presets_for(&self, base_table: &str, section: Section) -> Vec<(Uuid, String)> {
        self.presets
            .iter()
            .filter(|p| p.section == section && p.base_table == base_table)
            .map(|p| (p.id, p.name.clone()))
            .collect()
    }

    /// Starts (or restarts) an inline edit of a preset, seeding the buffer from
    /// its current name and definition.
    fn begin_preset_edit(&mut self, id: Uuid) {
        if let Some(preset) = self.presets.iter().find(|p| p.id == id) {
            self.preset_edits.insert(
                id,
                PresetEdit {
                    name: preset.name.clone(),
                    definition: preset.definition.clone(),
                    is_default: preset.is_default,
                },
            );
        }
    }

    /// Commits the in-progress edit of preset `id` locally and to the backend,
    /// then drops the edit buffer (it now matches the saved preset).
    fn commit_preset_edit(&mut self, id: Uuid) {
        let Some(edit) = self.preset_edits.remove(&id) else {
            return;
        };
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(preset) = self.presets.iter_mut().find(|p| p.id == id) {
            preset.name = name;
            preset.definition = edit.definition;
            preset.is_default = edit.is_default;
            preset.modified_at = rpc::now_epoch();
            rpc::update_preset(
                preset.id,
                &preset.name,
                &preset.definition,
                preset.is_default,
                preset.modified_at,
            );
        }
    }

    /// Deletes a preset locally and on the backend, and drops references to it
    /// from every open page's live definition so those queries keep
    /// assembling. (Affected queries show as unsaved until re-saved.)
    fn delete_preset(&mut self, id: Uuid) {
        self.presets.retain(|p| p.id != id);
        rpc::delete_preset(id);
        for page in &mut self.pages {
            let def = &mut page.live.definition;
            def.filter.presets.retain(|p| *p != id);
            if def.sort == SectionContent::Preset(id) {
                def.sort = SectionContent::default();
            }
            if def.display == SectionContent::Preset(id) {
                def.display = SectionContent::default();
            }
        }
        self.preset_edits.remove(&id);
        if self.expanded_preset == Some(id) {
            self.expanded_preset = None;
        }
    }

    /// The naming dialog shown by "Save as preset". Confirming creates the
    /// preset and swaps the saved fragment for a reference to it.
    pub(crate) fn render_preset_save_modal(&mut self, ctx: &egui::Context) {
        let Some(state) = self.preset_save.as_mut() else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("preset_save")).show(ctx, |ui| {
            ui.set_max_width(280.0);
            ui.heading(format!(
                "Save {} preset",
                state.section.noun().to_lowercase()
            ));
            ui.add_space(8.0);
            let field = crate::text_input::add(
                ui,
                egui::TextEdit::singleline(&mut state.name)
                    .hint_text("Preset name")
                    .desired_width(f32::INFINITY),
            );
            if state.take_focus {
                field.request_focus();
                state.take_focus = false;
            }
            let name_ok = !state.name.trim().is_empty();
            if name_ok && field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save = true;
            }
            ui.add_space(8.0);
            ui.checkbox(&mut state.is_default, "Apply by default");
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(name_ok, egui::Button::new("Save")).clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if modal.should_close() {
            cancel = true;
        }
        if save {
            if let Some(state) = self.preset_save.take() {
                self.create_preset(state);
            }
        } else if cancel {
            self.preset_save = None;
        }
    }

    /// Creates a preset from the naming dialog's state and replaces the source
    /// fragment in the current query with a reference to it.
    fn create_preset(&mut self, state: PresetSave) {
        let Some(base_table) = self
            .current_page()
            .map(|p| p.live.definition.base.clone())
            .filter(|b| !b.trim().is_empty())
        else {
            return;
        };
        let now = rpc::now_epoch();
        let preset = Preset {
            id: Uuid::new_v4(),
            name: state.name.trim().to_string(),
            base_table,
            section: state.section,
            definition: state.definition,
            is_default: state.is_default,
            created_at: now,
            modified_at: now,
        };
        rpc::add_preset(&preset);
        if let Some(page) = self.current_page_mut() {
            let def = &mut page.live.definition;
            match state.section {
                Section::Filter => {
                    def.filter.custom.clear();
                    def.filter.presets.push(preset.id);
                }
                Section::Sort => def.sort = SectionContent::Preset(preset.id),
                Section::Display => def.display = SectionContent::Preset(preset.id),
            }
        }
        self.presets.push(preset);
    }

    /// The manage-presets modal: lists every preset for the current base table,
    /// with per-preset delete.
    pub(crate) fn render_manage_presets_modal(&mut self, ctx: &egui::Context) {
        if !self.manage_presets {
            return;
        }
        let base_table = self
            .current_page()
            .map_or(String::new(), |p| p.live.definition.base.clone());
        let listed: Vec<(Uuid, String, Section, bool)> = self
            .presets
            .iter()
            .filter(|p| p.base_table == base_table)
            .map(|p| (p.id, p.name.clone(), p.section, p.is_default))
            .collect();

        let mut delete = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("manage_presets")).show(ctx, |ui| {
            ui.set_width(280.0);
            ui.heading("Manage presets");
            ui.add_space(8.0);
            if listed.is_empty() {
                ui.weak("No presets yet.");
            }
            for (id, name, section, is_default) in &listed {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.weak(section.noun().to_lowercase());
                    if *is_default {
                        ui.weak("· default");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if Button::icon(icons::DELETE)
                            .tint(DELETE_RED)
                            .show(ui)
                            .clicked()
                        {
                            delete = Some(*id);
                        }
                    });
                });
            }
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        if let Some(id) = delete {
            self.delete_preset(id);
        }
        if close || modal.should_close() {
            self.manage_presets = false;
        }
    }
}

/// Pixel size of the section icon shown on each options-menu row.
const MENU_ROW_ICON_SIZE: f32 = 16.0;

/// One row of a section's options menu: a native [`egui::Checkbox`] or
/// [`egui::RadioButton`] (per `kind`) whose label is the section `icon` followed
/// by `label`. Using egui's own widgets keeps these rows' checkbox/radio styling
/// consistent with the form checkboxes elsewhere (e.g. "Apply by default").
/// `checked` is the row's current state; `enabled` gates interaction. Returns the
/// click response.
fn toggle_menu_item(
    ui: &mut egui::Ui,
    kind: ToggleKind,
    icon: icons::MaterialIcon,
    label: &str,
    checked: bool,
    enabled: bool,
) -> egui::Response {
    let atoms = (
        egui::RichText::new(icon.codepoint)
            .family(icons::family())
            .size(MENU_ROW_ICON_SIZE),
        egui::RichText::new(label),
    );
    match kind {
        ToggleKind::Checkbox => {
            let mut checked = checked;
            ui.add_enabled(enabled, egui::Checkbox::new(&mut checked, atoms))
        }
        ToggleKind::Radio => ui.add_enabled(enabled, egui::RadioButton::new(checked, atoms)),
    }
}

/// A small, all-caps category heading inside a section's options menu (the
/// `text` is already upper-case).
fn menu_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).small().weak());
}

/// A builder section's toolbar (gear) options menu, shared by Filter, Sort, and
/// Display. It renders a flat list whose rows each carry a checkbox (`kind` =
/// [`ToggleKind::Checkbox`]) or radio button ([`ToggleKind::Radio`]): first the
/// always-present "Custom …" entry, then any built-in presets (under a "BUILT IN
/// PRESETS" heading), then the user-defined presets (under a "USER DEFINED
/// PRESETS" heading). Each `(_, checked)` / `(id, name, checked)` carries the
/// row's current state. Returns the entry the user clicked; clicking any row also
/// closes the menu.
fn section_options_menu(
    trigger: Option<&egui::Response>,
    kind: ToggleKind,
    custom_label: &str,
    custom_checked: bool,
    custom_enabled: bool,
    builtins: &[(BuiltinKind, bool)],
    presets: &[(Uuid, String, bool)],
) -> Option<MenuEntry> {
    options_menu(trigger, |ui| {
        let mut choice = None;
        if toggle_menu_item(
            ui,
            kind,
            icons::CUSTOM,
            custom_label,
            custom_checked,
            custom_enabled,
        )
        .clicked()
        {
            choice = Some(MenuEntry::Custom);
        }
        if !builtins.is_empty() {
            ui.separator();
            menu_heading(ui, "BUILT IN PRESETS");
            for (builtin, checked) in builtins {
                if toggle_menu_item(ui, kind, builtin.icon(), builtin.name(), *checked, true)
                    .clicked()
                {
                    choice = Some(MenuEntry::Builtin(*builtin));
                }
            }
        }
        if !presets.is_empty() {
            ui.separator();
            menu_heading(ui, "USER DEFINED PRESETS");
            for (id, name, checked) in presets {
                if toggle_menu_item(ui, kind, icons::PRESET, name, *checked, true).clicked() {
                    choice = Some(MenuEntry::Preset(*id));
                }
            }
        }
        choice
    })
}

/// The custom-filter input with a trailing `⋮` menu (shown only when the input
/// is non-empty), sized to fill the available width. Returns the menu's chosen
/// value.
fn filter_custom_input(
    ui: &mut egui::Ui,
    custom: &mut String,
    base_chosen: bool,
    focus: bool,
    run: &mut bool,
) -> Option<CustomChoice> {
    let has_text = !custom.trim().is_empty();
    // The trigger sits inside the input (its space is reserved by `with_menu` as
    // extra right padding), so shrink the editor's content width by exactly the
    // trigger's footprint — and nothing more — to keep the whole input the same
    // outer width as the empty (trigger-less) state. Reserving any extra here
    // would leave a stray gap beside the input.
    let trigger_reserve = if has_text { crate::button::SIZE } else { 0.0 };
    let w = (ui.available_width() - trigger_reserve).max(40.0);

    // With no text there's nothing the menu can act on, so show a plain input.
    if !has_text {
        code_editor(ui, custom, w, focus, run);
        return None;
    }

    let (output, trigger) = crate::text_input::with_menu(ui, monospace_edit(custom, w));
    drive_code_editor(ui, output, custom, focus, run);
    egui::Popup::menu(&trigger)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.set_width(190.0);
            let mut choice = None;
            if menu_item(ui, icons::CLEAR, "Clear", true, None).clicked() {
                choice = Some(CustomChoice::Clear);
            }
            if menu_item(ui, icons::SAVE, "Save as preset", base_chosen, None).clicked() {
                choice = Some(CustomChoice::Save);
            }
            choice
        })
        .and_then(|inner| inner.inner)
}

/// Draws a collapsed preset "tab" (its name area). For a user preset this is a
/// single line — chevron, preset icon, name, and (when dirty) a red star — at
/// normal font size, padded to a one-line input's height, with no background
/// (the background is painted by the caller only while expanded). The whole tab
/// is clickable. For the built-in tab it's the Shuffle name beside a Reshuffle
/// button, with no chevron and no expansion. Returns any click plus, when this
/// tab is the expanded one, its rect (so the caller can paint its background).
fn draw_tab(ui: &mut egui::Ui, t: &TabInfo) -> (Option<TabClick>, Option<egui::Rect>) {
    if t.builtin {
        let mut click = None;
        egui::Frame::new().inner_margin(TAB_PADDING).show(ui, |ui| {
            ui.style_mut().interaction.selectable_labels = false;
            ui.horizontal(|ui| {
                // Preset icon, then the preset's name ("Shuffle"), then the
                // Reshuffle button on the right.
                ui.label(format!("{}  {}", icons::PRESET.codepoint, t.name));
                if ui
                    .button(format!("{}  Reshuffle", icons::SHUFFLE.codepoint))
                    .clicked()
                {
                    click = Some(TabClick::Reshuffle);
                }
            });
        });
        return (click, None);
    }
    let inner = egui::Frame::new().inner_margin(TAB_PADDING).show(ui, |ui| {
        // The name area is a click target, not text — don't let its labels show
        // the I-beam cursor or become text-selectable.
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            let arrow = if t.expanded {
                icons::EXPAND_OPEN
            } else {
                icons::EXPAND_CLOSED
            };
            ui.label(format!(
                "{}  {}  {}",
                arrow.codepoint,
                icons::PRESET.codepoint,
                t.name
            ));
            if t.dirty {
                ui.label(egui::RichText::new(icons::UNSAVED.codepoint).color(DELETE_RED));
            }
        });
    });
    let rect = inner.response.rect;
    let clicked = inner
        .response
        .interact(egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::Default)
        .clicked();
    let click = clicked.then_some(TabClick::Toggle(t.id));
    let expanded_rect = t.expanded.then_some(rect);
    (click, expanded_rect)
}

/// The background shape for an expanded tab: a pink fill with rounded top corners
/// (the bottom merges into the detail panel below).
fn tab_bg_shape(rect: egui::Rect) -> egui::Shape {
    egui::Shape::rect_filled(
        rect,
        egui::CornerRadius {
            nw: TAB_RADIUS,
            ne: TAB_RADIUS,
            sw: 0,
            se: 0,
        },
        PRESET_BG,
    )
}

/// An over-estimate of a collapsed preset tab's rendered width, used to decide
/// whether the tabs fit beside the custom filter input. Slightly generous so the
/// tabs never overflow the available width.
fn tab_est_width(ui: &egui::Ui, t: &TabInfo) -> f32 {
    let body = egui::TextStyle::Body.resolve(ui.style());
    // The name, plus the chevron and preset icon glyphs with their gaps.
    let mut w = galley_width(ui, &t.name, body) + 48.0;
    if t.dirty {
        w += 18.0;
    }
    // + inner margins (7 each side) + a little slack.
    w + 14.0 + 4.0
}

/// The full-querydown editor: a single large code editor bound to the query's
/// full text. Full mode has no presets or section options, so this is just one
/// custom block. Ctrl+Enter runs the query.
fn full_builder_ui(ui: &mut egui::Ui, def: &mut QueryDefinition, run: &mut bool) {
    let Some(text) = def.full.as_mut() else {
        return;
    };
    small_heading(ui, "FULL QUERY");
    code_editor(ui, text, f32::INFINITY, false, run);
}

/// A tinted rounded container used for the preset detail panel.
fn section_frame<R>(
    ui: &mut egui::Ui,
    fill: egui::Color32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(TAB_RADIUS)
        .inner_margin(egui::Margin::same(8))
        .show(ui, add)
        .inner
}

/// The small all-caps heading at the top of a builder block.
fn small_heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// A monospace Querydown editor that auto-sizes to its line count (at least one
/// line). `width` is the desired width (use `f32::INFINITY` for full width).
/// When `focus`, it grabs focus once with the caret at the end. Ctrl+Enter
/// requests a query run.
fn code_editor(ui: &mut egui::Ui, text: &mut String, width: f32, focus: bool, run: &mut bool) {
    let output = crate::text_input::show(ui, monospace_edit(text, width));
    drive_code_editor(ui, output, text, focus, run);
}

/// Builds the auto-sizing monospace `TextEdit` shared by every Querydown editor:
/// it grows to its line count (at least one line) and fills `width`.
fn monospace_edit(text: &mut String, width: f32) -> egui::TextEdit<'_> {
    let rows = text.lines().count().max(1);
    egui::TextEdit::multiline(text)
        .desired_width(width)
        .desired_rows(rows)
        .font(egui::TextStyle::Monospace)
}

/// Applies a code editor's behavior to a shown editor's `output`: optional
/// one-time focus with the caret at the end, and Ctrl+Enter to request a run.
fn drive_code_editor(
    ui: &mut egui::Ui,
    mut output: egui::widgets::text_edit::TextEditOutput,
    text: &str,
    focus: bool,
    run: &mut bool,
) {
    if focus {
        output.response.request_focus();
        let end = text.chars().count();
        output
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(end),
                egui::text::CCursor::new(end),
            )));
        output.state.store(ui.ctx(), output.response.id);
    }
    if output.response.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl)
    {
        *run = true;
    }
}

/// A monospace multiline editor that auto-sizes to its line count (at least one
/// line), for preset definitions. `width` is the desired width.
fn sized_multiline(ui: &mut egui::Ui, text: &mut String, width: f32) -> egui::Response {
    crate::text_input::add(ui, monospace_edit(text, width))
}

/// The width of `text` laid out without wrapping in `font`.
fn galley_width(ui: &egui::Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE)
        .size()
        .x
}

/// A `⋮` button opening a popup menu; returns the menu's chosen value, if any.
fn dots_menu<T>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> Option<T>) -> Option<T> {
    let dots = Button::icon(icons::MORE).show(ui);
    egui::Popup::menu(&dots)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.set_width(190.0);
            content(ui)
        })
        .and_then(|inner| inner.inner)
}

/// A builder section's toolbar options menu, hung off the section's toolbar
/// button via its embedded "⋮" menu `trigger`. Returns the menu's chosen value,
/// if any. A `None` trigger (the section button isn't showing one this frame)
/// yields no menu.
fn options_menu<T>(
    trigger: Option<&egui::Response>,
    content: impl FnOnce(&mut egui::Ui) -> Option<T>,
) -> Option<T> {
    let trigger = trigger?;
    egui::Popup::menu(trigger)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.set_width(190.0);
            content(ui)
        })
        .and_then(|inner| inner.inner)
}

/// A "… ▶" submenu listing user-defined presets by name, each with the
/// preset/approval icon. Returns the clicked preset id. The submenu-button glyph
/// is inlined in the label text; Material Symbols are registered as a fallback on
/// the proportional family, so it renders inline.
fn preset_submenu(ui: &mut egui::Ui, label: &str, presets: &[(Uuid, String)]) -> Option<Uuid> {
    let label = format!("{}  {label}", icons::PRESET.codepoint);
    let (_, inner) = egui::containers::menu::SubMenuButton::new(label).ui(ui, |ui| {
        // A fixed (narrow) width: the icon-bearing `menu_item` rows each claim the
        // full available width, so without an upper bound the submenu stretches.
        ui.set_width(150.0);
        let mut choice = None;
        if presets.is_empty() {
            ui.weak("No presets");
        }
        for (id, name) in presets {
            if menu_item(ui, icons::PRESET, name, true, None).clicked() {
                choice = Some(*id);
            }
        }
        choice
    });
    inner.and_then(|i| i.inner)
}
