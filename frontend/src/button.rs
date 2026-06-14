//! The app's one toolbar button. Every clickable icon/label control routes
//! through here so they all share a size and look: frameless (no background) by
//! default, an optional icon to the left of an optional label, a subtle blue
//! hover outline painted *inside* the allocated rect (so it never shifts
//! layout), and a blue "active" background for toggled buttons.

use eframe::egui;

use crate::ACCENT_BLUE;
use crate::icons::{self, MaterialIcon};

/// Light-blue background of an active (toggled-on) button.
const ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(0xBB, 0xD9, 0xFB);
/// The fixed height (and icon-only width) shared by every button.
const SIZE: f32 = 26.0;
/// Icon glyph size.
const ICON_SIZE: f32 = 16.0;
/// Label font size.
const LABEL_SIZE: f32 = 13.0;
/// Horizontal padding inside a labelled button.
const PAD_X: f32 = 6.0;
/// Gap between the icon and the label.
const ICON_GAP: f32 = 5.0;
/// Corner radius of the hover outline and active fill.
const RADIUS: f32 = 4.0;

/// A toolbar button. Build it with the chaining setters, then `show` it.
// Several independent style flags; folding them into enums wouldn't read better.
#[allow(clippy::struct_excessive_bools)]
#[must_use]
pub(crate) struct Button {
    icon: Option<MaterialIcon>,
    label: Option<String>,
    caret: bool,
    active: bool,
    enabled: bool,
    spin: bool,
    tint: Option<egui::Color32>,
}

impl Button {
    /// A button with neither an icon nor a label (set at least one).
    pub(crate) fn new() -> Self {
        Self {
            icon: None,
            label: None,
            caret: false,
            active: false,
            enabled: true,
            spin: false,
            tint: None,
        }
    }

    /// A button showing just `icon`.
    pub(crate) fn icon(icon: MaterialIcon) -> Self {
        Self::new().with_icon(icon)
    }

    pub(crate) fn with_icon(mut self, icon: MaterialIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub(crate) fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Appends a dropdown caret ("▾") after the content.
    pub(crate) fn caret(mut self, caret: bool) -> Self {
        self.caret = caret;
        self
    }

    /// Gives the button the blue toggled-on background.
    pub(crate) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Continuously rotates the (icon-only) glyph to act as a loading spinner.
    pub(crate) fn spin(mut self, spin: bool) -> Self {
        self.spin = spin;
        self
    }

    /// Overrides the content color (e.g. red for destructive actions).
    pub(crate) fn tint(mut self, tint: egui::Color32) -> Self {
        self.tint = Some(tint);
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui) -> egui::Response {
        // A spinning button reads as "busy" rather than "disabled", so keep its
        // glyph at full strength even while it's not clickable.
        let color = if self.enabled || self.spin {
            self.tint.unwrap_or_else(|| ui.visuals().text_color())
        } else {
            ui.visuals().weak_text_color()
        };

        // Lay out icon + label + caret together so the content can be centered
        // (icon-only) or left-padded (labelled) as one unit.
        let mut job = egui::text::LayoutJob::default();
        if let Some(icon) = self.icon {
            job.append(
                icon.codepoint,
                0.0,
                egui::TextFormat {
                    font_id: icons::font_id(ICON_SIZE),
                    color,
                    ..Default::default()
                },
            );
        }
        if let Some(label) = &self.label {
            let leading = if self.icon.is_some() { ICON_GAP } else { 0.0 };
            job.append(
                label,
                leading,
                egui::TextFormat {
                    font_id: egui::FontId::proportional(LABEL_SIZE),
                    color,
                    ..Default::default()
                },
            );
        }
        if self.caret {
            job.append(
                icons::EXPAND.codepoint,
                4.0,
                egui::TextFormat {
                    font_id: icons::font_id(10.0),
                    color,
                    ..Default::default()
                },
            );
        }
        let galley = ui.painter().layout_job(job);

        // Labelled buttons grow with their content; icon-only buttons are a fixed
        // square so they all line up uniformly.
        let width = if self.label.is_some() {
            galley.size().x + PAD_X * 2.0
        } else {
            SIZE
        };
        let sense = if self.enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, SIZE), sense);

        if ui.is_rect_visible(rect) {
            if self.active {
                ui.painter().rect_filled(rect, RADIUS, ACTIVE_BG);
            }
            // Hover outline, painted just inside the rect so it costs no layout
            // space; transparent otherwise so hovering never shifts anything.
            let stroke_color = if self.enabled && resp.hovered() {
                ACCENT_BLUE
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_stroke(
                rect.shrink(0.5),
                RADIUS,
                egui::Stroke::new(1.0, stroke_color),
                egui::StrokeKind::Inside,
            );

            if self.spin {
                ui.ctx().request_repaint();
                let angle = ui.input(|i| i.time) as f32 * std::f32::consts::TAU;
                ui.painter().add(
                    egui::epaint::TextShape::new(rect.center(), galley, color)
                        .with_angle_and_anchor(angle, egui::Align2::CENTER_CENTER),
                );
            } else {
                let pos = rect.center() - galley.size() / 2.0;
                ui.painter().galley(pos, galley, color);
            }
        }
        resp
    }
}
