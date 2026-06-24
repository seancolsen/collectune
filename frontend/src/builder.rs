//! The query-builder panel: per-section editors for the filter, sorting, and
//! display parts of a query. Each section combines hand-written Querydown
//! fragments ("custom") with saved presets, and the panel also owns the modals
//! for naming, renaming, and managing presets.

use eframe::egui;
use uuid::Uuid;

use crate::App;
use crate::button::Button;
use crate::icons;
use crate::now_playing::menu_item;
use crate::page::DELETE_RED;
use crate::query_def::{BuiltinPreset, FilterParts, QueryDefinition, Section, SectionContent};
use crate::rpc::{self, Preset};

/// Background of a preset block.
const PRESET_BG: egui::Color32 = egui::Color32::from_rgb(0xF3, 0xE3, 0xFB);

/// Smallest height the builder panel will shrink to, so an empty/"no query"
/// state still has a sane size.
const MIN_BUILDER_HEIGHT: f32 = 80.0;
/// Vertical margin of the panel's `Frame::side_top_panel` (`symmetric(8, 2)`),
/// added around the measured content so the panel is exactly tall enough.
const FRAME_V_MARGIN: f32 = 4.0;

/// Minimum width the custom-filter text input must retain beside any collapsed
/// preset cards. Below this, the cards move to their own row below the input.
const MIN_FILTER_INPUT_WIDTH: f32 = 400.0;

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

/// The "rename preset" dialog: the preset being renamed and the in-progress name.
pub(crate) struct PresetRename {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    /// Set on the first frame so the name field grabs focus once.
    pub(crate) take_focus: bool,
}

/// An in-progress edit of a saved preset's definition. The buffers are committed
/// to the preset (and the backend) on save, or discarded on revert.
pub(crate) struct PresetEdit {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) definition: String,
    pub(crate) is_default: bool,
}

/// Scope of the manage-presets modal: every preset for the current base
/// table, or just one section's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManageScope {
    All,
    Section(Section),
}

/// A choice from a custom block's inline `⋮` menu.
enum CustomChoice {
    Clear,
    Save,
    /// Replace the custom content with the chosen preset (sort/display only).
    Use(Uuid),
}

/// An action chosen from a preset (or built-in) block's inline `⋮` menu. Not
/// every variant applies to every section; the caller handles the relevant ones.
enum InlineAction {
    /// Discard in-progress edits, restoring the last-saved definition.
    Revert,
    /// Open the rename-preset dialog.
    Rename,
    /// Fold the preset's fragment into the custom filter text (filter only).
    MergeIntoCustom,
    /// Replace the preset reference with its current definition as custom text.
    ConvertToCustom,
    /// Drop the preset (or built-in) from the query.
    Remove,
    /// Swap in a different preset.
    UsePreset(Uuid),
}

/// Options selected from a section's toolbar (gear) options menu.
enum OptionsChoice {
    Reset,
    SaveAsPreset,
    UsePreset(Uuid),
    UseShuffle,
    Manage,
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
                // Feed this frame's natural content height back for the next
                // frame. Use a tolerance so sub-pixel jitter doesn't trigger a
                // permanent repaint loop (which would peg the CPU).
                let measured = output.content_size.y;
                if self
                    .builder_content_height
                    .is_none_or(|h| (h - measured).abs() > 0.5)
                {
                    self.builder_content_height = Some(measured);
                    ui.ctx().request_repaint();
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
    /// presets, the latter shown as collapsible cards beside (or below) the input.
    // One linear pass over the section's blocks and menus; splitting it up
    // would just scatter the collected actions.
    #[allow(clippy::too_many_lines)]
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
        let mut save_as = None;
        let mut toggle_expand = None;
        let mut inline = None;

        let presets = def.filter.presets.clone();
        // Drop a stale expansion (e.g. the preset was removed elsewhere).
        if let Some(eid) = self.expanded_filter_preset
            && !presets.contains(&eid)
        {
            self.expanded_filter_preset = None;
            self.preset_edit = None;
        }
        let expanded = self.expanded_filter_preset;
        let expanded_dirty = self.preset_edit_dirty();

        if presets.is_empty() {
            // No presets: the input consumes the full width.
            ui.horizontal_top(|ui| {
                custom_choice =
                    filter_custom_input(ui, &mut def.filter.custom, base_chosen, focus, run, 0.0);
            });
        } else {
            let names: Vec<(Uuid, String)> = presets
                .iter()
                .map(|id| (*id, self.preset_name(*id)))
                .collect();
            let total_cards: f32 = names
                .iter()
                .map(|(_, n)| measure_collapsed_card_width(ui, n))
                .sum::<f32>()
                + ui.spacing().item_spacing.x * names.len() as f32;
            let side_by_side = ui.available_width() - total_cards >= MIN_FILTER_INPUT_WIDTH;
            if side_by_side {
                // Input on the left (filling the remainder), cards on the right.
                ui.horizontal_top(|ui| {
                    custom_choice = filter_custom_input(
                        ui,
                        &mut def.filter.custom,
                        base_chosen,
                        focus,
                        run,
                        total_cards,
                    );
                    for (id, name) in &names {
                        let dirty = expanded == Some(*id) && expanded_dirty;
                        if collapsed_filter_card(ui, *id, name, dirty, expanded == Some(*id)) {
                            toggle_expand = Some(*id);
                        }
                    }
                });
            } else {
                // Not enough room: input full-width, cards wrapped below it.
                ui.horizontal_top(|ui| {
                    custom_choice = filter_custom_input(
                        ui,
                        &mut def.filter.custom,
                        base_chosen,
                        focus,
                        run,
                        0.0,
                    );
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for (id, name) in &names {
                        let dirty = expanded == Some(*id) && expanded_dirty;
                        if collapsed_filter_card(ui, *id, name, dirty, expanded == Some(*id)) {
                            toggle_expand = Some(*id);
                        }
                    }
                });
            }
        }

        // The expanded preset's editor, full width below the row of cards.
        if let Some(eid) = self.expanded_filter_preset {
            ui.add_space(6.0);
            inline = self
                .preset_editor(ui, eid, None, run, |ui, dirty| {
                    let mut choice = None;
                    if menu_item(ui, icons::REVERT, "Revert changes", dirty, None).clicked() {
                        choice = Some(InlineAction::Revert);
                    }
                    if menu_item(ui, icons::RENAME, "Rename preset", true, None).clicked() {
                        choice = Some(InlineAction::Rename);
                    }
                    if menu_item(ui, icons::CONVERT, "Merge into custom", true, None).clicked() {
                        choice = Some(InlineAction::MergeIntoCustom);
                    }
                    if menu_item(ui, icons::CLOSE, "Remove from query", true, None).clicked() {
                        choice = Some(InlineAction::Remove);
                    }
                    choice
                })
                .map(|a| (eid, a));
        }

        // The toolbar (gear) options menu.
        let addable: Vec<(Uuid, String)> = self
            .presets_for(&def.base, Section::Filter)
            .into_iter()
            .filter(|(id, _)| !def.filter.presets.contains(id))
            .collect();
        let toolbar = options_menu(trigger, |ui| {
            let mut choice = None;
            if menu_item(ui, icons::RESET, "Reset to default", true, None).clicked() {
                choice = Some(OptionsChoice::Reset);
            }
            if let Some(id) = preset_submenu(ui, "Add Preset", &addable) {
                choice = Some(OptionsChoice::UsePreset(id));
            }
            choice
        });

        // Apply collected actions.
        match custom_choice {
            Some(CustomChoice::Clear) => def.filter.custom.clear(),
            Some(CustomChoice::Save) => save_as = Some(def.filter.custom.clone()),
            _ => {}
        }
        if let Some(id) = toggle_expand {
            if self.expanded_filter_preset == Some(id) {
                self.expanded_filter_preset = None;
                self.preset_edit = None;
            } else {
                self.expanded_filter_preset = Some(id);
                self.begin_preset_edit(id);
            }
        }
        if let Some((id, action)) = inline {
            match action {
                InlineAction::Revert => self.begin_preset_edit(id),
                InlineAction::Rename => self.begin_preset_rename(id),
                InlineAction::MergeIntoCustom => {
                    if let Some(preset) = self.presets.iter().find(|p| p.id == id) {
                        if !def.filter.custom.trim().is_empty() {
                            def.filter.custom.push('\n');
                        }
                        def.filter.custom.push_str(&preset.definition);
                    }
                    def.filter.presets.retain(|p| *p != id);
                    self.expanded_filter_preset = None;
                    self.preset_edit = None;
                    *run = true;
                }
                InlineAction::Remove => {
                    def.filter.presets.retain(|p| *p != id);
                    self.expanded_filter_preset = None;
                    self.preset_edit = None;
                    *run = true;
                }
                InlineAction::ConvertToCustom | InlineAction::UsePreset(_) => {}
            }
        }
        match toolbar {
            Some(OptionsChoice::Reset) => {
                def.filter = FilterParts::default();
                self.expanded_filter_preset = None;
                self.preset_edit = None;
                *run = true;
            }
            Some(OptionsChoice::UsePreset(id)) => {
                def.filter.presets.push(id);
                *run = true;
            }
            _ => {}
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
    /// (always-expanded, inline-editable) preset, or the built-in Shuffle preset.
    // One linear pass over the section's blocks and menus; splitting it up
    // would just scatter the collected actions.
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
        let noun = section.noun();
        let heading = format!("PRESET {}", noun.to_uppercase());
        let focus = std::mem::take(&mut self.builder_focus);

        let mut custom_choice = None;
        let mut save_as = None;
        let mut inline = None;
        // Assigned by every match arm below.
        let toolbar;
        let mut reshuffle = false;

        let content = match section {
            Section::Sort => &mut def.sort,
            Section::Display => &mut def.display,
            Section::Filter => unreachable!("filter uses filter_builder_ui"),
        };

        match content {
            SectionContent::Custom(text) => {
                let has_text = !text.trim().is_empty();
                let inline_presets = available.clone();
                ui.horizontal_top(|ui| {
                    let reserve = crate::button::SIZE + ui.spacing().item_spacing.x;
                    let w = (ui.available_width() - reserve).max(40.0);
                    code_editor(ui, text, w, focus, run);
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
                        if let Some(id) = preset_submenu(ui, "Replace with preset", &inline_presets)
                        {
                            choice = Some(CustomChoice::Use(id));
                        }
                        choice
                    });
                });
                toolbar = options_menu(trigger, |ui| {
                    let mut choice = None;
                    if show_shuffle
                        && menu_item(ui, icons::SHUFFLE, "Shuffle", true, None).clicked()
                    {
                        choice = Some(OptionsChoice::UseShuffle);
                    }
                    if menu_item(ui, icons::RESET, "Reset to default", true, None).clicked() {
                        choice = Some(OptionsChoice::Reset);
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
                        choice = Some(OptionsChoice::SaveAsPreset);
                    }
                    if let Some(id) = preset_submenu(ui, "Replace with preset", &available) {
                        choice = Some(OptionsChoice::UsePreset(id));
                    }
                    if menu_item(
                        ui,
                        icons::MANAGE_PRESETS,
                        &format!("Manage {} presets", noun.to_lowercase()),
                        true,
                        None,
                    )
                    .clicked()
                    {
                        choice = Some(OptionsChoice::Manage);
                    }
                    choice
                });
            }
            SectionContent::Preset(id) => {
                let id = *id;
                let inline_presets = available.clone();
                inline = self
                    .preset_editor(ui, id, Some(&heading), run, |ui, dirty| {
                        let mut choice = None;
                        if menu_item(ui, icons::REVERT, "Revert changes", dirty, None).clicked() {
                            choice = Some(InlineAction::Revert);
                        }
                        if menu_item(ui, icons::RENAME, "Rename preset", true, None).clicked() {
                            choice = Some(InlineAction::Rename);
                        }
                        if menu_item(ui, icons::CONVERT, "Convert to custom", true, None).clicked()
                        {
                            choice = Some(InlineAction::ConvertToCustom);
                        }
                        if menu_item(ui, icons::CLOSE, "Remove from query", true, None).clicked() {
                            choice = Some(InlineAction::Remove);
                        }
                        if let Some(pid) =
                            preset_submenu(ui, "Use a different preset", &inline_presets)
                        {
                            choice = Some(InlineAction::UsePreset(pid));
                        }
                        choice
                    })
                    .map(|a| (id, a));
                toolbar = options_menu(trigger, |ui| {
                    single_preset_toolbar(ui, show_shuffle, noun, &available)
                });
            }
            SectionContent::Builtin(builtin) => {
                // A built-in preset gets its own block. Its name sits beside a
                // "Reshuffle" button that regenerates the seed in place, plus an
                // inline `⋮` menu. The generated Querydown is shown below.
                let inline_presets = available.clone();
                section_frame(ui, PRESET_BG, |ui| {
                    small_heading(ui, &heading);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(builtin.name()).strong());
                        let label = format!("{}  Reshuffle", icons::SHUFFLE.codepoint);
                        if ui.button(label).clicked() {
                            reshuffle = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            inline = dots_menu(ui, |ui| {
                                let mut choice = None;
                                if menu_item(ui, icons::CONVERT, "Convert to custom", true, None)
                                    .clicked()
                                {
                                    choice = Some(InlineAction::ConvertToCustom);
                                }
                                if menu_item(ui, icons::CLOSE, "Remove from query", true, None)
                                    .clicked()
                                {
                                    choice = Some(InlineAction::Remove);
                                }
                                if let Some(pid) =
                                    preset_submenu(ui, "Use a different preset", &inline_presets)
                                {
                                    choice = Some(InlineAction::UsePreset(pid));
                                }
                                choice
                            })
                            .map(|a| (Uuid::nil(), a));
                        });
                    });
                    ui.label(egui::RichText::new(builtin.querydown()).monospace());
                });
                toolbar = options_menu(trigger, |ui| {
                    single_preset_toolbar(ui, show_shuffle, noun, &available)
                });
            }
        }

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
                self.preset_edit = None;
                *run = true;
            }
            None => {}
        }
        if let Some((id, action)) = inline {
            match action {
                InlineAction::Revert => self.begin_preset_edit(id),
                InlineAction::Rename => self.begin_preset_rename(id),
                InlineAction::ConvertToCustom => {
                    let text = match content {
                        SectionContent::Builtin(builtin) => builtin.querydown(),
                        _ => self
                            .presets
                            .iter()
                            .find(|p| p.id == id)
                            .map(|p| p.definition.clone())
                            .unwrap_or_default(),
                    };
                    *content = SectionContent::Custom(text);
                    self.preset_edit = None;
                }
                InlineAction::Remove => {
                    *content = SectionContent::default();
                    self.preset_edit = None;
                    *run = true;
                }
                InlineAction::UsePreset(pid) => {
                    *content = SectionContent::Preset(pid);
                    self.preset_edit = None;
                    *run = true;
                }
                InlineAction::MergeIntoCustom => {}
            }
        }
        match toolbar {
            Some(OptionsChoice::Reset) => {
                if matches!(
                    content,
                    SectionContent::Preset(_) | SectionContent::Builtin(_)
                ) {
                    *run = true;
                }
                *content = SectionContent::default();
                self.preset_edit = None;
            }
            Some(OptionsChoice::SaveAsPreset) => {
                if let SectionContent::Custom(text) = content {
                    save_as = Some(text.clone());
                }
            }
            Some(OptionsChoice::UsePreset(id)) => {
                *content = SectionContent::Preset(id);
                self.preset_edit = None;
                *run = true;
            }
            Some(OptionsChoice::UseShuffle) => {
                *content = SectionContent::Builtin(BuiltinPreset::shuffle());
                self.preset_edit = None;
                *run = true;
            }
            Some(OptionsChoice::Manage) => {
                self.manage_presets = Some(ManageScope::Section(section));
            }
            None => {}
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

    /// Renders the inline editor for the preset `id` (an always-expanded
    /// sort/display preset, or an expanded filter preset): an editable
    /// definition, a save button (enabled only while the buffer differs from the
    /// saved preset), the `⋮` menu (its items supplied by `menu`, which receives
    /// the dirty flag), and the "Apply by default" checkbox. Saving commits and
    /// triggers a re-run. Returns the menu's chosen value.
    fn preset_editor<T>(
        &mut self,
        ui: &mut egui::Ui,
        id: Uuid,
        header: Option<&str>,
        run: &mut bool,
        menu: impl FnOnce(&mut egui::Ui, bool) -> Option<T>,
    ) -> Option<T> {
        // Seed (or re-seed) the edit buffer when it doesn't already target `id`,
        // so typing in a stable buffer is never wiped mid-edit.
        if self.preset_edit.as_ref().is_none_or(|e| e.id != id) {
            self.begin_preset_edit(id);
        }
        let name = self.preset_name(id);
        let dirty = self.preset_edit_dirty();
        let mut edit = self.preset_edit.take()?;
        let mut chosen = None;
        let mut save = false;
        section_frame(ui, PRESET_BG, |ui| {
            if let Some(heading) = header {
                small_heading(ui, heading);
                ui.label(egui::RichText::new(&name).strong());
            }
            ui.horizontal_top(|ui| {
                let reserve = (crate::button::SIZE + ui.spacing().item_spacing.x) * 2.0;
                let w = (ui.available_width() - reserve).max(40.0);
                sized_multiline(ui, &mut edit.definition, w);
                save = Button::icon(icons::SAVE)
                    .enabled(dirty && !edit.name.trim().is_empty())
                    .show(ui)
                    .clicked();
                chosen = dots_menu(ui, |ui| menu(ui, dirty));
            });
            ui.checkbox(&mut edit.is_default, "Apply by default");
        });
        self.preset_edit = Some(edit);
        if save {
            self.commit_preset_edit();
            *run = true;
        }
        chosen
    }

    /// The display name of a preset, or a placeholder if it no longer exists.
    fn preset_name(&self, id: Uuid) -> String {
        self.presets
            .iter()
            .find(|p| p.id == id)
            .map_or_else(|| "(missing preset)".to_string(), |p| p.name.clone())
    }

    /// Whether the in-progress preset edit differs from its saved version (so the
    /// save button enables and the unsaved marker shows).
    fn preset_edit_dirty(&self) -> bool {
        let Some(edit) = &self.preset_edit else {
            return false;
        };
        match self.presets.iter().find(|p| p.id == edit.id) {
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

    /// Starts an inline edit of a preset, seeding the buffers from its current
    /// name and definition.
    fn begin_preset_edit(&mut self, id: Uuid) {
        if let Some(preset) = self.presets.iter().find(|p| p.id == id) {
            self.preset_edit = Some(PresetEdit {
                id,
                name: preset.name.clone(),
                definition: preset.definition.clone(),
                is_default: preset.is_default,
            });
        }
    }

    /// Opens the rename dialog for a preset, seeding it with the current name.
    fn begin_preset_rename(&mut self, id: Uuid) {
        self.preset_rename = Some(PresetRename {
            id,
            name: self.preset_name(id),
            take_focus: true,
        });
    }

    /// Commits the in-progress preset edit locally and to the backend.
    fn commit_preset_edit(&mut self) {
        let Some(edit) = self.preset_edit.take() else {
            return;
        };
        let name = edit.name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(preset) = self.presets.iter_mut().find(|p| p.id == edit.id) {
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

    /// Commits a preset rename locally and to the backend, keeping any active
    /// edit buffer's name in sync so it doesn't read as freshly unsaved.
    fn commit_preset_rename(&mut self, state: &PresetRename) {
        let name = state.name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let mut committed = false;
        if let Some(preset) = self.presets.iter_mut().find(|p| p.id == state.id) {
            preset.name.clone_from(&name);
            preset.modified_at = rpc::now_epoch();
            rpc::update_preset(
                preset.id,
                &preset.name,
                &preset.definition,
                preset.is_default,
                preset.modified_at,
            );
            committed = true;
        }
        if committed
            && let Some(edit) = self.preset_edit.as_mut()
            && edit.id == state.id
        {
            edit.name = name;
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
        if self.preset_edit.as_ref().is_some_and(|e| e.id == id) {
            self.preset_edit = None;
        }
        if self.expanded_filter_preset == Some(id) {
            self.expanded_filter_preset = None;
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

    /// The rename dialog shown by a preset block's "Rename preset" menu item.
    pub(crate) fn render_preset_rename_modal(&mut self, ctx: &egui::Context) {
        let Some(state) = self.preset_rename.as_mut() else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("preset_rename")).show(ctx, |ui| {
            ui.set_max_width(280.0);
            ui.heading("Rename preset");
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
            if let Some(state) = self.preset_rename.take() {
                self.commit_preset_rename(&state);
            }
        } else if cancel {
            self.preset_rename = None;
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

    /// The manage-presets modal: lists the current base table's presets in the
    /// requested scope, with per-preset delete.
    pub(crate) fn render_manage_presets_modal(&mut self, ctx: &egui::Context) {
        let Some(scope) = self.manage_presets else {
            return;
        };
        let base_table = self
            .current_page()
            .map_or(String::new(), |p| p.live.definition.base.clone());
        let listed: Vec<(Uuid, String, Section, bool)> = self
            .presets
            .iter()
            .filter(|p| p.base_table == base_table)
            .filter(|p| match scope {
                ManageScope::All => true,
                ManageScope::Section(section) => p.section == section,
            })
            .map(|p| (p.id, p.name.clone(), p.section, p.is_default))
            .collect();

        let mut delete = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("manage_presets")).show(ctx, |ui| {
            ui.set_width(280.0);
            let heading = match scope {
                ManageScope::All => "Manage presets".to_string(),
                ManageScope::Section(section) => {
                    format!("Manage {} presets", section.noun().to_lowercase())
                }
            };
            ui.heading(heading);
            ui.add_space(8.0);
            if listed.is_empty() {
                ui.weak("No presets yet.");
            }
            for (id, name, section, is_default) in &listed {
                ui.horizontal(|ui| {
                    ui.label(name);
                    if scope == ManageScope::All {
                        ui.weak(section.noun().to_lowercase());
                    }
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
            self.manage_presets = None;
        }
    }
}

/// The sort/display toolbar options menu shared by the preset and built-in
/// states (the custom state has its own, with "Save as preset"). Returns the
/// chosen option.
fn single_preset_toolbar(
    ui: &mut egui::Ui,
    show_shuffle: bool,
    noun: &str,
    available: &[(Uuid, String)],
) -> Option<OptionsChoice> {
    let mut choice = None;
    if show_shuffle && menu_item(ui, icons::SHUFFLE, "Shuffle", true, None).clicked() {
        choice = Some(OptionsChoice::UseShuffle);
    }
    if menu_item(ui, icons::RESET, "Reset to default", true, None).clicked() {
        choice = Some(OptionsChoice::Reset);
    }
    if let Some(id) = preset_submenu(ui, "Replace with preset", available) {
        choice = Some(OptionsChoice::UsePreset(id));
    }
    if menu_item(
        ui,
        icons::MANAGE_PRESETS,
        &format!("Manage {} presets", noun.to_lowercase()),
        true,
        None,
    )
    .clicked()
    {
        choice = Some(OptionsChoice::Manage);
    }
    choice
}

/// The custom-filter input with a trailing `⋮` menu (shown only when the input
/// is non-empty). `reserve_for_cards` is width set aside to the right for any
/// collapsed preset cards rendered after it. Returns the menu's chosen value.
fn filter_custom_input(
    ui: &mut egui::Ui,
    custom: &mut String,
    base_chosen: bool,
    focus: bool,
    run: &mut bool,
    reserve_for_cards: f32,
) -> Option<CustomChoice> {
    let has_text = !custom.trim().is_empty();
    // The trigger now sits inside the input rather than after it, so shrink the
    // editor's content width by the trigger's footprint to keep the whole input
    // within the available width.
    let spacing = ui.spacing().item_spacing.x;
    let trigger_reserve = if has_text {
        crate::button::SIZE + spacing
    } else {
        0.0
    };
    let w = (ui.available_width() - reserve_for_cards - trigger_reserve).max(40.0);

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

/// A collapsed filter preset card: a small "PRESET" heading (with an unsaved
/// marker when `dirty`) above a disclosure arrow and the preset name. The whole
/// card is clickable; returns `true` when clicked (to toggle its expansion).
fn collapsed_filter_card(
    ui: &mut egui::Ui,
    id: Uuid,
    name: &str,
    dirty: bool,
    expanded: bool,
) -> bool {
    ui.push_id(id, |ui| {
        let inner = egui::Frame::new()
            .fill(PRESET_BG)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!("{}  PRESET", icons::PRESET.codepoint))
                                .small()
                                .weak(),
                        );
                        if dirty {
                            ui.label(
                                egui::RichText::new(icons::UNSAVED.codepoint)
                                    .small()
                                    .color(DELETE_RED),
                            );
                        }
                    });
                    let arrow = if expanded {
                        icons::EXPAND_OPEN
                    } else {
                        icons::EXPAND_CLOSED
                    };
                    ui.label(format!("{}  {name}", arrow.codepoint));
                });
            });
        inner.response.interact(egui::Sense::click()).clicked()
    })
    .inner
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

/// A tinted rounded container used for preset blocks.
fn section_frame<R>(
    ui: &mut egui::Ui,
    fill: egui::Color32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(fill)
        .corner_radius(6.0)
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

/// An over-estimate of a collapsed preset card's rendered width, used to decide
/// whether the cards fit beside the custom filter input. Slightly generous so
/// the cards never overflow the available width.
fn measure_collapsed_card_width(ui: &egui::Ui, name: &str) -> f32 {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    // Each content line carries an icon glyph plus a gap (~22px).
    let heading_line = galley_width(ui, "PRESET", small) + 22.0;
    let name_line = galley_width(ui, name, body) + 22.0;
    // + inner margins (8 each side) + a little slack.
    heading_line.max(name_line) + 16.0 + 4.0
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
