//! The query-builder panel: per-section editors for the filter, sorting, and
//! display parts of a query. Each section combines hand-written Querydown
//! fragments ("custom", cyan) with saved presets (yellow), and the panel also
//! owns the modals for naming and managing presets.

use eframe::egui;
use uuid::Uuid;

use crate::App;
use crate::button::Button;
use crate::icons;
use crate::now_playing::menu_item;
use crate::page::DELETE_RED;
use crate::query_def::{FilterParts, QueryDefinition, Section, SectionContent};
use crate::rpc::{self, Preset};

/// Background of a custom (hand-written) block.
const CUSTOM_BG: egui::Color32 = egui::Color32::from_rgb(0xDD, 0xF3, 0xF8);
/// Background of a preset block.
const PRESET_BG: egui::Color32 = egui::Color32::from_rgb(0xFB, 0xF4, 0xC9);

/// Smallest height the builder panel will shrink to, so an empty/"no query"
/// state still has a sane size.
const MIN_BUILDER_HEIGHT: f32 = 80.0;
/// Vertical margin of the panel's `Frame::side_top_panel` (`symmetric(8, 2)`),
/// added around the measured content so the panel is exactly tall enough.
const FRAME_V_MARGIN: f32 = 4.0;

/// The "save as preset" naming dialog: which section is being saved and the
/// Querydown fragment to store.
pub(crate) struct PresetSave {
    pub(crate) section: Section,
    pub(crate) name: String,
    pub(crate) definition: String,
    /// Set on the first frame so the name field grabs focus once.
    pub(crate) take_focus: bool,
}

/// An in-progress edit of a saved preset. The buffers are committed to the
/// preset (and the backend) on save, or discarded on revert.
pub(crate) struct PresetEdit {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) definition: String,
}

/// Scope of the manage-presets modal: every preset for the current base
/// table, or just one section's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManageScope {
    All,
    Section(Section),
}

/// An action chosen from a filter preset block's `⋮` menu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PresetBlockAction {
    Remove,
    Edit,
    MergeIntoCustom,
}

impl App {
    /// The top panel below the menu bar holding the open builder section.
    pub(crate) fn render_builder_panel(&mut self, ui: &mut egui::Ui) {
        let Some(section) = self.builder_section else {
            return;
        };
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
        egui::Panel::top("query_builder")
            .exact_size(height)
            .show_inside(ui, |ui| {
                let output = egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(def) = def.as_mut() else {
                        ui.weak("No query selected.");
                        return;
                    };
                    ui.add_space(6.0);
                    match section {
                        Section::Filter => self.filter_builder_ui(ui, def, &mut run),
                        Section::Sort | Section::Display => {
                            self.single_builder_ui(ui, section, def, &mut run);
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

    /// The filter builder: one custom block combined (via AND) with any number
    /// of presets.
    // One linear pass over the section's blocks and menus; splitting it up
    // would just scatter the collected actions.
    #[allow(clippy::too_many_lines)]
    fn filter_builder_ui(&mut self, ui: &mut egui::Ui, def: &mut QueryDefinition, run: &mut bool) {
        let base_chosen = def.is_runnable();
        let mut save_as = None;
        let mut block_action = None;

        section_frame(ui, CUSTOM_BG, |ui| {
            let custom = &mut def.filter.custom;
            ui.horizontal(|ui| {
                small_heading(ui, "CUSTOM FILTER");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(choice) = dots_menu(ui, |ui| {
                        let mut choice = None;
                        let has_text = !custom.trim().is_empty();
                        if menu_item(ui, icons::CLEAR, "Clear", has_text, None).clicked() {
                            choice = Some(false);
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
                            choice = Some(true);
                        }
                        choice
                    }) {
                        if choice {
                            save_as = Some(custom.clone());
                        } else {
                            custom.clear();
                        }
                    }
                });
            });
            code_editor(ui, custom, 2, run);
        });

        for id in def.filter.presets.clone() {
            ui.add_space(6.0);
            if let Some(action) = self.preset_block(ui, "PRESET FILTER", id, true) {
                block_action = Some((id, action));
            }
        }

        match block_action {
            Some((id, PresetBlockAction::Remove)) => def.filter.presets.retain(|p| *p != id),
            Some((id, PresetBlockAction::Edit)) => self.begin_preset_edit(id),
            Some((id, PresetBlockAction::MergeIntoCustom)) => {
                if let Some(preset) = self.presets.iter().find(|p| p.id == id) {
                    if !def.filter.custom.trim().is_empty() {
                        def.filter.custom.push('\n');
                    }
                    def.filter.custom.push_str(&preset.definition);
                }
                def.filter.presets.retain(|p| *p != id);
            }
            None => {}
        }

        let addable: Vec<(Uuid, String)> = self
            .presets_for(&def.base, Section::Filter)
            .into_iter()
            .filter(|(id, _)| !def.filter.presets.contains(id))
            .collect();
        let filter_choice = options_menu(ui, Section::Filter, |ui| {
            let mut choice = None;
            if menu_item(ui, icons::RESET, "Reset to default", true, None).clicked() {
                choice = Some(OptionsChoice::Reset);
            }
            if let Some(id) = preset_submenu(ui, "Add Preset", &addable) {
                choice = Some(OptionsChoice::UsePreset(id));
            }
            if menu_item(ui, icons::MANAGE_PRESETS, "Manage all presets", true, None).clicked() {
                choice = Some(OptionsChoice::Manage);
            }
            choice
        });
        match filter_choice {
            Some(OptionsChoice::Reset) => def.filter = FilterParts::default(),
            Some(OptionsChoice::UsePreset(id)) => def.filter.presets.push(id),
            Some(OptionsChoice::Manage) => self.manage_presets = Some(ManageScope::All),
            _ => {}
        }

        if let Some(definition) = save_as {
            self.preset_save = Some(PresetSave {
                section: Section::Filter,
                name: String::new(),
                definition,
                take_focus: true,
            });
        }
    }

    /// The sort/display builder: the section is either one custom block or one
    /// preset, switched via the options menu.
    // One linear pass over the section's blocks and menus; splitting it up
    // would just scatter the collected actions.
    #[allow(clippy::too_many_lines)]
    fn single_builder_ui(
        &mut self,
        ui: &mut egui::Ui,
        section: Section,
        def: &mut QueryDefinition,
        run: &mut bool,
    ) {
        let base_chosen = def.is_runnable();
        let available = self.presets_for(&def.base, section);
        let noun = section.noun();
        let content = match section {
            Section::Sort => &mut def.sort,
            Section::Display => &mut def.display,
            Section::Filter => unreachable!("filter uses filter_builder_ui"),
        };

        let mut save_as = None;
        let mut edit_preset = None;
        let choice = match content {
            SectionContent::Custom(text) => {
                section_frame(ui, CUSTOM_BG, |ui| {
                    small_heading(ui, &format!("CUSTOM {}", noun.to_uppercase()));
                    code_editor(ui, text, 5, run);
                });
                let has_text = !text.trim().is_empty();
                options_menu(ui, section, |ui| {
                    let mut choice = None;
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
                })
            }
            SectionContent::Preset(id) => {
                let id = *id;
                self.preset_block(ui, &format!("PRESET {}", noun.to_uppercase()), id, false);
                options_menu(ui, section, |ui| {
                    let mut choice = None;
                    if menu_item(ui, icons::RESET, "Reset to default", true, None).clicked() {
                        choice = Some(OptionsChoice::Reset);
                    }
                    if menu_item(ui, icons::CONVERT, "Convert to custom", true, None).clicked() {
                        choice = Some(OptionsChoice::ConvertToCustom);
                    }
                    if menu_item(ui, icons::BUILDER, "Edit preset", true, None).clicked() {
                        choice = Some(OptionsChoice::EditPreset(id));
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
                })
            }
        };

        match choice {
            Some(OptionsChoice::Reset) => *content = SectionContent::default(),
            Some(OptionsChoice::SaveAsPreset) => {
                if let SectionContent::Custom(text) = content {
                    save_as = Some(text.clone());
                }
            }
            Some(OptionsChoice::UsePreset(id)) => *content = SectionContent::Preset(id),
            Some(OptionsChoice::ConvertToCustom) => {
                if let SectionContent::Preset(id) = content {
                    let text = self
                        .presets
                        .iter()
                        .find(|p| p.id == *id)
                        .map(|p| p.definition.clone())
                        .unwrap_or_default();
                    *content = SectionContent::Custom(text);
                }
            }
            Some(OptionsChoice::EditPreset(id)) => edit_preset = Some(id),
            Some(OptionsChoice::Manage) => {
                self.manage_presets = Some(ManageScope::Section(section));
            }
            None => {}
        }
        if let Some(id) = edit_preset {
            self.begin_preset_edit(id);
        }
        if let Some(definition) = save_as {
            self.preset_save = Some(PresetSave {
                section,
                name: String::new(),
                definition,
                take_focus: true,
            });
        }
    }

    /// One referenced preset as a collapsible yellow block (or its inline edit
    /// UI while it's being edited). `with_menu` adds the `⋮` menu used by
    /// filter preset blocks; returns the action chosen from it, if any.
    fn preset_block(
        &mut self,
        ui: &mut egui::Ui,
        heading: &str,
        id: Uuid,
        with_menu: bool,
    ) -> Option<PresetBlockAction> {
        let mut action = None;
        let editing = self.preset_edit.as_ref().is_some_and(|e| e.id == id);
        section_frame(ui, PRESET_BG, |ui| {
            if editing {
                let mut save = false;
                let mut revert = false;
                {
                    let edit = self.preset_edit.as_mut().unwrap();
                    ui.horizontal(|ui| {
                        small_heading(ui, heading);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            revert = Button::icon(icons::RESET).show(ui).clicked();
                            save = Button::icon(icons::SAVE)
                                .enabled(!edit.name.trim().is_empty())
                                .show(ui)
                                .clicked();
                            ui.add_sized(
                                egui::vec2(ui.available_width(), 20.0),
                                egui::TextEdit::singleline(&mut edit.name),
                            );
                        });
                    });
                    ui.add(
                        egui::TextEdit::multiline(&mut edit.definition)
                            .desired_width(f32::INFINITY)
                            .desired_rows(4)
                            .font(egui::TextStyle::Monospace),
                    );
                }
                if save {
                    self.commit_preset_edit();
                } else if revert {
                    self.preset_edit = None;
                }
            } else {
                ui.horizontal(|ui| {
                    small_heading(ui, heading);
                    if with_menu {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            action = dots_menu(ui, |ui| {
                                let mut choice = None;
                                if menu_item(ui, icons::CLOSE, "Remove", true, None).clicked() {
                                    choice = Some(PresetBlockAction::Remove);
                                }
                                if menu_item(ui, icons::BUILDER, "Edit preset", true, None)
                                    .clicked()
                                {
                                    choice = Some(PresetBlockAction::Edit);
                                }
                                if menu_item(ui, icons::CONVERT, "Merge into custom", true, None)
                                    .clicked()
                                {
                                    choice = Some(PresetBlockAction::MergeIntoCustom);
                                }
                                choice
                            });
                        });
                    }
                });
                let preset = self.presets.iter().find(|p| p.id == id);
                let name = preset.map_or("(missing preset)", |p| p.name.as_str());
                egui::CollapsingHeader::new(name)
                    .id_salt(("preset_block", id))
                    .show(ui, |ui| {
                        let definition = preset.map_or("", |p| p.definition.as_str());
                        ui.label(egui::RichText::new(definition).monospace());
                    });
            }
        });
        action
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
            });
        }
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
            preset.modified_at = rpc::now_epoch();
            rpc::update_preset(
                preset.id,
                &preset.name,
                &preset.definition,
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
        if self.preset_edit.as_ref().is_some_and(|e| e.id == id) {
            self.preset_edit = None;
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
            let field = ui.add(
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
        let listed: Vec<(Uuid, String, Section)> = self
            .presets
            .iter()
            .filter(|p| p.base_table == base_table)
            .filter(|p| match scope {
                ManageScope::All => true,
                ManageScope::Section(section) => p.section == section,
            })
            .map(|p| (p.id, p.name.clone(), p.section))
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
            for (id, name, section) in &listed {
                ui.horizontal(|ui| {
                    ui.label(name);
                    if scope == ManageScope::All {
                        ui.weak(section.noun().to_lowercase());
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

/// Options selected from a section's gear-options menu.
enum OptionsChoice {
    Reset,
    SaveAsPreset,
    UsePreset(Uuid),
    ConvertToCustom,
    EditPreset(Uuid),
    Manage,
}

/// A tinted rounded container used for custom (cyan) and preset (yellow) blocks.
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

/// A monospace Querydown fragment editor. Ctrl+Enter requests a query run.
fn code_editor(ui: &mut egui::Ui, text: &mut String, rows: usize, run: &mut bool) {
    let resp = ui.add(
        egui::TextEdit::multiline(text)
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .font(egui::TextStyle::Monospace),
    );
    if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl) {
        *run = true;
    }
}

/// A `⋮` button opening a popup menu; returns the menu's chosen value, if any.
fn dots_menu<T>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> Option<T>) -> Option<T> {
    let dots = Button::icon(icons::MORE).show(ui);
    egui::Popup::menu(&dots)
        .align(egui::RectAlign::BOTTOM_END)
        .show(|ui| {
            ui.set_width(170.0);
            content(ui)
        })
        .and_then(|inner| inner.inner)
}

/// The right-aligned "⚙ <Section> options ▾" button below a builder section,
/// opening the given menu. Returns the menu's chosen value, if any.
fn options_menu<T>(
    ui: &mut egui::Ui,
    section: Section,
    content: impl FnOnce(&mut egui::Ui) -> Option<T>,
) -> Option<T> {
    let mut choice = None;
    ui.add_space(4.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        let label = format!("{} options", section.noun());
        let resp = Button::icon(icons::OPTIONS)
            .label(label)
            .caret(true)
            .show(ui);
        choice = egui::Popup::menu(&resp)
            .align(egui::RectAlign::BOTTOM_END)
            .show(|ui| {
                ui.set_width(190.0);
                content(ui)
            })
            .and_then(|inner| inner.inner);
    });
    choice
}

/// A "… ▶" submenu listing presets by name. Returns the clicked preset id.
/// The preset glyph is inlined in the label text; Material Symbols are
/// registered as a fallback on the proportional family, so it renders inline.
fn preset_submenu(ui: &mut egui::Ui, label: &str, presets: &[(Uuid, String)]) -> Option<Uuid> {
    let label = format!("{}  {label}", icons::PRESET.codepoint);
    let (_, inner) = egui::containers::menu::SubMenuButton::new(label).ui(ui, |ui| {
        ui.set_min_width(150.0);
        let mut choice = None;
        if presets.is_empty() {
            ui.weak("No presets");
        }
        for (id, name) in presets {
            if ui.button(name).clicked() {
                choice = Some(*id);
            }
        }
        choice
    });
    inner.and_then(|i| i.inner)
}
