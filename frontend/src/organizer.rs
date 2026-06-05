//! The sliding "organizer" side panel — a page switcher. It lists the saved
//! queries, lets the user create new ones and refresh the list, and switches
//! the displayed page on click.

use std::cmp::Reverse;
use std::sync::Arc;

use eframe::egui;
use uuid::Uuid;

use crate::{
    ACCENT_BLUE, App, ORGANIZER_DRAG_FRICTION, ORGANIZER_SWIPE_VELOCITY, ORGANIZER_WIDTH, rpc,
};

/// Sliding "organizer" side panel state, including the in-progress drag gesture.
#[derive(Default)]
pub(crate) struct Organizer {
    pub(crate) open: bool,
    pub(crate) dragging: bool,
    pub(crate) drag_dx: f32,
    pub(crate) drag_start_progress: f32,
    pub(crate) dragged_progress: f32,
}

/// One row in the query list, as displayed in the organizer.
struct ListItem {
    id: Uuid,
    name: String,
    unsaved: bool,
}

/// Deferred outcomes of interacting with the query list, applied after the
/// drawer's UI closure releases its borrow of `self`.
#[derive(Default)]
struct ListActions {
    add: bool,
    refresh: bool,
    clicked: Option<Uuid>,
}

impl App {
    pub(crate) fn render_organizer(
        &mut self,
        ctx: &egui::Context,
        progress: f32,
        panel_fill: egui::Color32,
    ) {
        let viewport = ctx.viewport_rect();
        let organizer_offset = progress * ORGANIZER_WIDTH;

        // The scrim covers the full viewport; the organizer Area (Order::Foreground)
        // paints over it wherever the drawer currently is.
        let scrim_alpha = (progress * 120.0).clamp(0.0, 255.0) as u8;
        egui::Area::new(egui::Id::new("organizer_scrim"))
            .order(egui::Order::Middle)
            .fixed_pos(viewport.min)
            .constrain(false)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, resp) =
                    ui.allocate_exact_size(viewport.size(), egui::Sense::click_and_drag());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(scrim_alpha));
                if resp.clicked() {
                    self.organizer.open = false;
                }
                self.handle_organizer_swipe(ctx, &resp, progress);
            });

        let organizer_x = organizer_offset - ORGANIZER_WIDTH;
        let screen_height = viewport.height();
        let items = self.visible_items();
        let current = self.current.query_id();
        let mut actions = ListActions::default();

        egui::Area::new(egui::Id::new("organizer"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(organizer_x, 0.0))
            .constrain(false)
            .interactable(true)
            .show(ctx, |ui| {
                let frame_rect = egui::Rect::from_min_size(
                    egui::pos2(organizer_x, 0.0),
                    egui::vec2(ORGANIZER_WIDTH, screen_height),
                );
                ui.set_min_size(egui::vec2(ORGANIZER_WIDTH, screen_height));
                // Cap the width so inner right-to-left layouts (e.g. the header's
                // "+" button) align to the sidebar's edge rather than the
                // viewport's. Without this the Area also expands its interactive
                // rect across the whole main area, swallowing the scrim's
                // click/swipe-to-close.
                ui.set_max_width(ORGANIZER_WIDTH);
                ui.painter().rect_filled(frame_rect, 0.0, panel_fill);

                // Low-priority swipe target underneath the controls, so taps on
                // the buttons and list rows aren't swallowed by the drag sense.
                let drag = ui.interact(
                    frame_rect,
                    egui::Id::new("organizer_drag"),
                    egui::Sense::drag(),
                );
                self.handle_organizer_swipe(ctx, &drag, progress);

                actions = draw_query_list(ui, &items, current, &mut self.filter);
            });

        if actions.add {
            self.add_query_page();
        }
        if let Some(id) = actions.clicked {
            self.select_page(id);
            self.organizer.open = false;
        }
        if actions.refresh {
            rpc::list_queries(Arc::clone(&self.loaded_queries), ctx.clone());
        }
    }

    /// The pages to show in the organizer: filtered by name, most-recently
    /// created first.
    fn visible_items(&self) -> Vec<ListItem> {
        let filter = self.filter.to_lowercase();
        let mut items: Vec<(i64, ListItem)> = self
            .pages
            .iter()
            .filter(|p| filter.is_empty() || p.live.name.to_lowercase().contains(&filter))
            .map(|p| {
                (
                    p.live.created_at,
                    ListItem {
                        id: p.live.id,
                        name: p.live.name.clone(),
                        unsaved: p.unsaved(),
                    },
                )
            })
            .collect();
        items.sort_by_key(|(created_at, _)| Reverse(*created_at));
        items.into_iter().map(|(_, item)| item).collect()
    }

    pub(crate) fn handle_organizer_swipe(
        &mut self,
        ctx: &egui::Context,
        resp: &egui::Response,
        current_progress: f32,
    ) {
        if resp.drag_started() {
            self.organizer.dragging = true;
            self.organizer.drag_dx = 0.0;
            self.organizer.drag_start_progress = current_progress;
            self.organizer.dragged_progress = current_progress;
        }
        if resp.dragged() {
            self.organizer.drag_dx += resp.drag_delta().x;
            let effective = static_friction(self.organizer.drag_dx, ORGANIZER_DRAG_FRICTION);
            self.organizer.dragged_progress =
                (self.organizer.drag_start_progress + effective / ORGANIZER_WIDTH).clamp(0.0, 1.0);
        }
        if resp.drag_stopped() {
            self.organizer.dragging = false;
            let velocity_x = ctx.input(|i| i.pointer.velocity().x);
            let effective = static_friction(self.organizer.drag_dx, ORGANIZER_DRAG_FRICTION);
            let flick = velocity_x <= -ORGANIZER_SWIPE_VELOCITY;
            let dragged_past_midpoint = effective <= -ORGANIZER_WIDTH / 2.0;
            if flick || dragged_past_midpoint {
                self.organizer.open = false;
            }
            self.organizer.drag_dx = 0.0;
        }
    }
}

/// Renders the "Queries" header (+ add button), the filter/refresh row, and the
/// query list. Returns the deferred actions the caller should apply.
fn draw_query_list(
    ui: &mut egui::Ui,
    items: &[ListItem],
    current: Option<Uuid>,
    filter: &mut String,
) -> ListActions {
    let mut actions = ListActions::default();

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.heading("Queries");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            if ui
                .button(egui::RichText::new(egui_phosphor::bold::PLUS).size(16.0))
                .clicked()
            {
                actions.add = true;
            }
        });
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text("Filter")
                .desired_width(ORGANIZER_WIDTH - 72.0),
        );
        if ui
            .button(egui::RichText::new(egui_phosphor::bold::ARROWS_CLOCKWISE).size(14.0))
            .clicked()
        {
            actions.refresh = true;
        }
    });

    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // No vertical spacing between rows so adjacent listings butt together
            // and leave no dead (unclickable) gap.
            ui.spacing_mut().item_spacing.y = 0.0;
            for item in items {
                if query_list_widget(ui, item, current == Some(item.id)).clicked() {
                    actions.clicked = Some(item.id);
                }
            }
        });

    actions
}

/// A single query listing in the organizer. Spans the sidebar's full width with
/// no dead click area, and reuses the query-result row's hover/selected styling
/// (see `results::row_widget`) so selection looks consistent across the app.
fn query_list_widget(ui: &mut egui::Ui, item: &ListItem, selected: bool) -> egui::Response {
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    let height = row_height + 12.0;
    let desired = egui::vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    let visuals = ui.visuals();
    if selected {
        let base = visuals.selection.bg_fill;
        let fill = if response.hovered() {
            egui::Color32::from_rgba_unmultiplied(
                base.r().saturating_sub(20),
                base.g().saturating_sub(20),
                base.b().saturating_sub(20),
                base.a(),
            )
        } else {
            base
        };
        ui.painter().rect_filled(rect, 0.0, fill);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, visuals.widgets.hovered.weak_bg_fill);
    }

    // Thin separator at the bottom of the row, matching the result rows.
    let sep_color = visuals.widgets.noninteractive.bg_stroke.color;
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, sep_color),
    );

    // Unsaved-changes dot, drawn in the left gutter.
    if item.unsaved {
        ui.painter().circle_filled(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            3.0,
            ACCENT_BLUE,
        );
    }

    let font_id = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &item.name,
        font_id,
        visuals.text_color(),
    );

    response
}

/// Models a static-friction-like resistance to drag motion: near-zero response
/// for small `dx`, smoothly approaching 1:1 response (offset by `friction`) for
/// `|dx|` much larger than `friction`.
fn static_friction(dx: f32, friction: f32) -> f32 {
    if friction <= 0.0 {
        return dx;
    }
    dx - friction * (dx / friction).tanh()
}
