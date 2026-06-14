//! The app's two toolbar controls. [`Button`] is the icon button every
//! clickable icon routes through, so they share a size and look: a frameless
//! fixed square with a subtle blue hover outline painted *inside* its rect (so
//! hovering never shifts layout). [`SplitButton`] adds a label and an active
//! (toggled-on) blue background, plus an optional embedded "⋮" menu trigger.

use eframe::egui;

use crate::ACCENT_BLUE;
use crate::icons::{self, MaterialIcon};

/// Light-blue background of an active (toggled-on) [`SplitButton`].
const ACTIVE_BG: egui::Color32 = egui::Color32::from_rgb(0xBB, 0xD9, 0xFB);
/// The fixed height (and icon-only width) shared by every button.
pub(crate) const SIZE: f32 = 26.0;
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

/// A frameless, fixed-square icon button. Build it with the chaining setters,
/// then `show` it.
#[must_use]
pub(crate) struct Button {
    icon: MaterialIcon,
    enabled: bool,
    spin: bool,
    tint: Option<egui::Color32>,
}

impl Button {
    /// A button showing `icon`.
    pub(crate) fn icon(icon: MaterialIcon) -> Self {
        Self {
            icon,
            enabled: true,
            spin: false,
            tint: None,
        }
    }

    pub(crate) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Continuously rotates the glyph to act as a loading spinner.
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

        let galley = ui
            .painter()
            .layout_job(egui::text::LayoutJob::single_section(
                self.icon.codepoint.to_owned(),
                egui::TextFormat {
                    font_id: icons::font_id(ICON_SIZE),
                    color,
                    ..Default::default()
                },
            ));

        let sense = if self.enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(SIZE, SIZE), sense);

        if ui.is_rect_visible(rect) {
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

            let pos = rect.center() - galley.size() / 2.0;
            if self.spin {
                ui.ctx().request_repaint();
                let angle = ui.input(|i| i.time) as f32 * std::f32::consts::TAU;
                ui.painter().add(
                    egui::epaint::TextShape::new(pos, galley, color)
                        .with_angle_and_anchor(angle, egui::Align2::CENTER_CENTER),
                );
            } else {
                ui.painter().galley(pos, galley, color);
            }
        }
        resp
    }
}

/// Width of the embedded "⋮" menu-trigger region shown on the right of an
/// active [`SplitButton`].
const MENU_TRIGGER_WIDTH: f32 = 22.0;
/// Background of the menu-trigger region: a lighter tint of [`ACTIVE_BG`] that
/// sets the trigger off from the main area.
const MENU_TRIGGER_BG: egui::Color32 = egui::Color32::from_rgb(0xD7, 0xEA, 0xFD);

/// A labelled toggle [`Button`] that, while `active`, grows an embedded "⋮" menu
/// trigger on its right, set off from the label by a lighter background. The main
/// area and the trigger are independent click targets sharing one button surface: the
/// main area toggles like an ordinary button, while the trigger is meant to
/// anchor a popup menu (see [`SplitButtonResponse::menu`]). The trigger appears
/// only while the button is active, matching designs where a thing's options are
/// reachable only once that thing is open.
#[must_use]
pub(crate) struct SplitButton {
    icon: MaterialIcon,
    label: String,
    active: bool,
}

/// The two independent responses produced by showing a [`SplitButton`].
pub(crate) struct SplitButtonResponse {
    /// The main (icon + label) region; its `clicked()` toggles the button.
    pub(crate) main: egui::Response,
    /// The embedded "⋮" menu trigger, present only while the button is active.
    /// Anchor a popup menu to it (e.g. `egui::Popup::menu(&menu)`).
    pub(crate) menu: Option<egui::Response>,
}

impl SplitButton {
    pub(crate) fn new(icon: MaterialIcon, label: impl Into<String>) -> Self {
        Self {
            icon,
            label: label.into(),
            active: false,
        }
    }

    /// Gives the button the blue toggled-on background and reveals its menu
    /// trigger.
    pub(crate) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub(crate) fn show(self, ui: &mut egui::Ui) -> SplitButtonResponse {
        let text_color = ui.visuals().text_color();
        let weak_color = ui.visuals().weak_text_color();

        // Icon + label laid out as one unit, centered in the main region.
        let mut job = egui::text::LayoutJob::default();
        job.append(
            self.icon.codepoint,
            0.0,
            egui::TextFormat {
                font_id: icons::font_id(ICON_SIZE),
                color: text_color,
                ..Default::default()
            },
        );
        job.append(
            &self.label,
            ICON_GAP,
            egui::TextFormat {
                font_id: egui::FontId::proportional(LABEL_SIZE),
                color: text_color,
                ..Default::default()
            },
        );
        let galley = ui.painter().layout_job(job);

        let main_w = galley.size().x + PAD_X * 2.0;
        let total_w = if self.active {
            main_w + MENU_TRIGGER_WIDTH
        } else {
            main_w
        };

        // The two regions need stable, distinct ids so each keeps its own
        // interaction state (and the trigger anchors a persistent popup).
        let id = egui::Id::new(("split_button", &self.label));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(total_w, SIZE), egui::Sense::hover());

        let main_rect = egui::Rect::from_min_size(rect.min, egui::vec2(main_w, SIZE));
        let main = ui.interact(main_rect, id.with("main"), egui::Sense::click());
        let menu = self.active.then(|| {
            let menu_rect = egui::Rect::from_min_max(main_rect.right_top(), rect.max);
            ui.interact(menu_rect, id.with("menu"), egui::Sense::click())
        });

        if ui.is_rect_visible(rect) {
            if self.active {
                ui.painter().rect_filled(rect, RADIUS, ACTIVE_BG);
                // The trigger region sits over the right end of the fill in a
                // lighter tint, sharing the button's rounded right corners.
                if let Some(menu) = &menu {
                    let radius = RADIUS as u8;
                    ui.painter().rect_filled(
                        menu.rect,
                        egui::CornerRadius {
                            nw: 0,
                            ne: radius,
                            sw: 0,
                            se: radius,
                        },
                        MENU_TRIGGER_BG,
                    );
                }
            }
            // Hover outline painted just inside the rect (so hovering never shifts
            // layout); transparent unless either region is hovered. An active
            // button already reads as highlighted, so it gets no hover outline.
            let hovered = !self.active
                && (main.hovered() || menu.as_ref().is_some_and(egui::Response::hovered));
            let stroke_color = if hovered {
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

            let pos = main_rect.center() - galley.size() / 2.0;
            ui.painter().galley(pos, galley, text_color);

            if let Some(menu) = &menu {
                // The dots read as secondary until the trigger itself is hovered.
                let dots_color = if menu.hovered() {
                    text_color
                } else {
                    weak_color
                };
                ui.painter().text(
                    menu.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icons::MORE.codepoint,
                    icons::font_id(ICON_SIZE),
                    dots_color,
                );
            }
        }

        SplitButtonResponse { main, menu }
    }
}
