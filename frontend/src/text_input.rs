//! The app's shared low-level text input. Every text field routes its
//! `egui::TextEdit` through [`show`]/[`add`] so they share one look: a little
//! inner padding, a light-gray resting border (a shade darker than the app's
//! gray panel background) that doesn't react to hover, and a same-thickness
//! light-blue border while focused.
//!
//! [`with_menu`] builds on that: an input that embeds an options-menu trigger
//! pinned to its top-right corner (see the filter builder's custom input).

use eframe::egui;

use crate::HOVER_BLUE;
use crate::button;
use crate::icons;

/// Inner padding between an input's border and its text — a touch roomier than
/// egui's default `(4, 2)` so the text breathes.
const PADDING: egui::Margin = egui::Margin {
    left: 7,
    right: 7,
    top: 4,
    bottom: 4,
};
/// Corner radius of the input's border (matches the app's buttons).
const RADIUS: f32 = 4.0;
/// Border thickness, shared by the resting and focused states.
const BORDER_WIDTH: f32 = 1.0;
/// Resting (and hover) border: a light gray a shade darker than the app's gray
/// panel fill, so the input reads as a distinct well.
const BORDER: egui::Color32 = egui::Color32::from_gray(0xC8);

/// Icon size of the embedded options-menu trigger glyph.
const TRIGGER_ICON_SIZE: f32 = 16.0;
/// The trigger glyph's resting color; it darkens to black on hover.
const TRIGGER_COLOR: egui::Color32 = egui::Color32::from_gray(0x55);

/// Shows `edit` as a standard app text input, returning its full output so the
/// caller can drive the cursor/selection. The styling is centralized here.
pub(crate) fn show(
    ui: &mut egui::Ui,
    edit: egui::TextEdit<'_>,
) -> egui::widgets::text_edit::TextEditOutput {
    show_padded(ui, edit, 0)
}

/// [`show`] returning only the response, for callers that don't touch the
/// cursor.
pub(crate) fn add(ui: &mut egui::Ui, edit: egui::TextEdit<'_>) -> egui::Response {
    show(ui, edit).response.response
}

/// Core of [`show`]. `extra_right` is added to the right padding so text never
/// slides under a trailing element (e.g. [`with_menu`]'s trigger).
fn show_padded(
    ui: &mut egui::Ui,
    edit: egui::TextEdit<'_>,
    extra_right: i8,
) -> egui::widgets::text_edit::TextEditOutput {
    let mut margin = PADDING;
    margin.right += extra_right;
    // A custom frame (fill + padding only) replaces egui's default frame so we
    // can paint the border ourselves: egui draws a focused input's border with
    // `selection.stroke`, which also tints selected text, so we can't recolor
    // it there without harming legibility.
    let frame = egui::Frame::new()
        .fill(ui.visuals().text_edit_bg_color())
        .corner_radius(RADIUS)
        .inner_margin(margin);
    let output = edit.frame(frame).show(ui);
    let color = if output.response.has_focus() {
        HOVER_BLUE
    } else {
        BORDER
    };
    ui.painter().rect_stroke(
        output.response.rect,
        RADIUS,
        egui::Stroke::new(BORDER_WIDTH, color),
        egui::StrokeKind::Inside,
    );
    output
}

/// A text input with an options-menu trigger embedded at its top-right. The
/// input reserves padding on the right so its text never slides under the
/// trigger, and the trigger stays pinned to the top-right corner no matter how
/// tall or wide the input grows. The trigger has no border (it's dark gray,
/// darkening to black on hover). Returns the input's output together with the
/// trigger's response — anchor a popup menu to the latter
/// (`egui::Popup::menu(&trigger)`).
pub(crate) fn with_menu(
    ui: &mut egui::Ui,
    edit: egui::TextEdit<'_>,
) -> (egui::widgets::text_edit::TextEditOutput, egui::Response) {
    let output = show_padded(ui, edit, button::SIZE as i8);
    let rect = output.response.rect;

    // Pin the trigger to the top-right corner. A short input shrinks the square
    // so the glyph still lines up with the single text row; a tall (multiline)
    // input keeps it at the top rather than centering it vertically.
    let side = rect.height().min(button::SIZE);
    let trigger_rect = egui::Rect::from_min_size(
        egui::pos2(rect.right() - side, rect.top()),
        egui::vec2(side, side),
    );
    // A later, distinct id puts the trigger on top of the text edit for hit
    // testing, so clicking it opens the menu instead of placing a text cursor.
    let trigger = ui.interact(
        trigger_rect,
        output.response.id.with("options_menu"),
        egui::Sense::click(),
    );
    if trigger.hovered() {
        // The trigger sits over the text edit, which would otherwise show its
        // I-beam cursor here; restore the plain cursor our buttons use so the
        // trigger reads as a button.
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
    let color = if trigger.hovered() {
        egui::Color32::BLACK
    } else {
        TRIGGER_COLOR
    };
    ui.painter().text(
        trigger_rect.center(),
        egui::Align2::CENTER_CENTER,
        icons::MORE.codepoint,
        icons::font_id(TRIGGER_ICON_SIZE),
        color,
    );

    (output, trigger)
}
