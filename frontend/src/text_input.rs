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

#[cfg(test)]
mod snapshot_tests {
    //! Headless snapshot tests for [`with_menu`], the Filter builder's
    //! custom-input widget. Each test renders one state to a PNG under
    //! `tests/snapshots/text_input_menu/` that the agent eyeballs against the
    //! mockup and (once correct) commits as a regression baseline. Generate or
    //! refresh the PNGs with `UPDATE_SNAPSHOTS=1 cargo test -p frontend`.
    //!
    //! We always render *with* the trigger (even when the text is empty) to
    //! isolate the widget itself, rather than reproducing the builder's
    //! empty-vs-non-empty branching in `filter_custom_input`.

    use std::cell::Cell;
    use std::rc::Rc;

    use eframe::egui;
    use egui_kittest::Harness;

    use super::{button, with_menu};

    /// Logical width of the test container — wide enough to mirror a real Filter
    /// section without forcing a wrap until we want one.
    const CONTAINER_W: f32 = 320.0;
    /// Render at 2× device scale so text is crisp enough to judge spacing by eye.
    const PPP: f32 = 2.0;

    /// Where, if anywhere, the pointer hovers for a given case.
    #[derive(Clone, Copy, PartialEq)]
    enum Hover {
        None,
        Input,
        Trigger,
    }

    /// Renders `text` in the given state and writes a snapshot named
    /// `text_input_menu/<name>`.
    fn snapshot(name: &str, text: &str, focus: bool, hover: Hover) {
        let mut buffer = text.to_owned();
        // Filled each frame so that, after the widget is laid out, we can aim the
        // pointer at it: (input rect, trigger rect).
        let rects: Rc<Cell<Option<(egui::Rect, egui::Rect)>>> = Rc::new(Cell::new(None));
        let rects_in_ui = rects.clone();
        // `build_ui` runs the closure once immediately, before we can configure
        // the context. The widget paints the trigger glyph from the
        // `material-icons` family, which isn't bound until `setup_fonts`, so we
        // bind fonts on that first frame and skip drawing the widget until the
        // next one (font changes only take effect on the following frame).
        let fonts_ready = Cell::new(false);

        let mut harness = Harness::builder()
            .with_size(egui::vec2(CONTAINER_W, 160.0))
            .with_pixels_per_point(PPP)
            .build_ui(move |ui| {
                if !fonts_ready.replace(true) {
                    // Match the running app's environment: bundled fonts + light
                    // visuals. Also stop the text caret blinking so the focused
                    // snapshot is deterministic rather than depending on phase.
                    crate::setup_fonts(ui.ctx());
                    ui.ctx()
                        .global_style_mut(|s| s.visuals.text_cursor.blink = false);
                    return;
                }

                // Mirror `filter_custom_input` + `monospace_edit`: reserve the
                // trigger's width on the right, then a monospace multiline edit
                // that auto-grows with its line count.
                let w = (ui.available_width() - button::SIZE).max(40.0);
                let rows = buffer.lines().count().max(1);
                let edit = egui::TextEdit::multiline(&mut buffer)
                    .desired_width(w)
                    .desired_rows(rows)
                    .font(egui::TextStyle::Monospace);
                let (output, _trigger) = with_menu(ui, edit);
                if focus {
                    output.response.request_focus();
                }
                // Recompute the trigger rect exactly as `with_menu` does, so we
                // can aim the pointer at its center for the trigger-hover case.
                let r = output.response.rect;
                let side = r.height().min(button::SIZE);
                let trigger_rect = egui::Rect::from_min_size(
                    egui::pos2(r.right() - side, r.top()),
                    egui::vec2(side, side),
                );
                rects_in_ui.set(Some((r, trigger_rect)));
            });

        harness.run();
        // Crop the snapshot tightly to the widget rather than the whole container.
        harness.fit_contents();

        if let (true, Some((input_rect, trigger_rect))) = (hover != Hover::None, rects.get()) {
            let pos = match hover {
                Hover::Input => input_rect.center(),
                Hover::Trigger => trigger_rect.center(),
                Hover::None => unreachable!(),
            };
            harness
                .input_mut()
                .events
                .push(egui::Event::PointerMoved(pos));
            harness.run();
        }

        harness.snapshot(format!("text_input_menu/{name}"));
    }

    #[test]
    fn empty() {
        snapshot("empty", "", false, Hover::None);
    }

    #[test]
    fn focused() {
        snapshot("focused", "", true, Hover::None);
    }

    #[test]
    fn hovered() {
        snapshot("hovered", "", false, Hover::Input);
    }

    #[test]
    fn small_text() {
        snapshot("small_text", "Lorem ipsum dolor", false, Hover::None);
    }

    #[test]
    fn small_text_trigger_hovered() {
        snapshot(
            "small_text_trigger_hovered",
            "Lorem ipsum dolor",
            false,
            Hover::Trigger,
        );
    }

    #[test]
    fn wrapping_text() {
        snapshot(
            "wrapping_text",
            "Lorem ipsum dolor sit amet, consectetur adipiscing elit sed do",
            false,
            Hover::None,
        );
    }
}
