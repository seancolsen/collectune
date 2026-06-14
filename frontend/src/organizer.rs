//! The sliding "organizer" side panel — a page switcher. It lists the saved
//! queries, lets the user create new ones and refresh the list, and switches
//! the displayed page on click.

use std::cmp::Reverse;
use std::sync::Arc;

use eframe::egui;
use uuid::Uuid;

use crate::button::Button;
use crate::icons;
use crate::page::{QueryAction, inline_rename_field, query_actions_menu, unsaved_marker_format};
use crate::{
    App, ORGANIZER_DRAG_FRICTION, ORGANIZER_SWIPE_VELOCITY, ORGANIZER_WIDTH, Rename, RenameSurface,
    rpc,
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
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
struct ListActions {
    add: bool,
    refresh: bool,
    clicked: Option<Uuid>,
    rename_request: Option<Uuid>,
    delete_request: Option<Uuid>,
    rename_commit: bool,
    rename_cancel: bool,
}

/// Per-row outcome from `query_list_widget`, folded into `ListActions`.
#[derive(Default)]
struct RowOutcome {
    clicked: bool,
    action: Option<QueryAction>,
    rename_commit: bool,
    rename_cancel: bool,
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

                actions = draw_query_list(ui, &items, current, &mut self.filter, &mut self.rename);
            });

        // Selecting a query closes the modal drawer (it overlays the content);
        // the persistent panel stays open.
        self.apply_list_actions(ctx, &actions, true);
    }

    /// Renders the organizer as a persistent left panel for wide screens. Unlike
    /// the modal drawer it reserves real layout space (the rest of the app lays
    /// out beside it), has no scrim or swipe-to-close, and stays open across
    /// selections — only the explorer button toggles it.
    pub(crate) fn render_persistent_organizer(
        &mut self,
        ui: &mut egui::Ui,
        panel_fill: egui::Color32,
    ) {
        let items = self.visible_items();
        let current = self.current.query_id();
        let mut actions = ListActions::default();

        egui::Panel::left("organizer_panel")
            .exact_size(ORGANIZER_WIDTH)
            .resizable(false)
            .frame(egui::Frame::new().fill(panel_fill))
            .show_animated_inside(ui, self.organizer.open, |ui| {
                actions = draw_query_list(ui, &items, current, &mut self.filter, &mut self.rename);
            });

        self.apply_list_actions(ui.ctx(), &actions, false);
    }

    /// Applies the deferred outcomes of interacting with the query list, shared by
    /// the modal drawer and the persistent panel. `close_on_select` closes the
    /// organizer after navigating to a query (used by the modal drawer only).
    fn apply_list_actions(
        &mut self,
        ctx: &egui::Context,
        actions: &ListActions,
        close_on_select: bool,
    ) {
        // Commit/cancel an in-progress rename before starting a new one, so
        // clicking "Rename" on another row first saves the current edit.
        if actions.rename_commit {
            self.commit_rename();
        }
        if actions.rename_cancel {
            self.cancel_rename();
        }
        if actions.add {
            self.add_query_page();
        }
        if let Some(id) = actions.clicked {
            self.select_page(id);
            if close_on_select {
                self.organizer.open = false;
            }
        }
        if let Some(id) = actions.rename_request {
            self.begin_rename(id, RenameSurface::Sidebar);
        }
        if let Some(id) = actions.delete_request {
            self.request_delete(id);
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
    rename: &mut Option<Rename>,
) -> ListActions {
    let mut actions = ListActions::default();

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.heading("Queries");
        // The list-refresh button sits right after the heading, in a lighter
        // color and with no background.
        if Button::icon(icons::REFRESH)
            .tint(ui.visuals().weak_text_color())
            .show(ui)
            .clicked()
        {
            actions.refresh = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            if Button::icon(icons::ADD).show(ui).clicked() {
                actions.add = true;
            }
        });
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        // Widen the filter so its right edge lines up with the add button's.
        ui.add(
            egui::TextEdit::singleline(filter)
                .hint_text("Filter")
                .desired_width(ORGANIZER_WIDTH - 24.0),
        );
    });

    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // No vertical spacing between rows so adjacent listings butt together
            // and leave no dead (unclickable) gap.
            ui.spacing_mut().item_spacing.y = 0.0;
            for item in items {
                let out = query_list_widget(ui, item, current == Some(item.id), rename);
                if out.clicked {
                    actions.clicked = Some(item.id);
                }
                if out.rename_commit {
                    actions.rename_commit = true;
                }
                if out.rename_cancel {
                    actions.rename_cancel = true;
                }
                match out.action {
                    Some(QueryAction::Rename) => actions.rename_request = Some(item.id),
                    Some(QueryAction::Delete) => actions.delete_request = Some(item.id),
                    None => {}
                }
            }
        });

    actions
}

/// A single query listing in the organizer. Spans the sidebar's full width with
/// no dead click area, and reuses the query-result row's hover/selected styling
/// (see `results::row_widget`) so selection looks consistent across the app.
///
/// When the row is being renamed it shows an inline edit field instead of the
/// name; otherwise it shows a `⋮` actions button (only while the row is hovered)
/// and opens the Rename/Delete menu on `⋮`-click or right-click.
fn query_list_widget(
    ui: &mut egui::Ui,
    item: &ListItem,
    selected: bool,
    rename: &mut Option<Rename>,
) -> RowOutcome {
    let mut outcome = RowOutcome::default();
    let row_height = ui.text_style_height(&egui::TextStyle::Body);
    let height = row_height + 12.0;
    let desired = egui::vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    // Capture colors up front (Color32 is Copy) so later `&mut ui` calls (interact,
    // child UIs) don't clash with an outstanding `ui.visuals()` borrow.
    let (sel_fill, hover_fill, sep_color, text_color, weak_text) = {
        let v = ui.visuals();
        (
            v.selection.bg_fill,
            // Only a slight darkening on hover, matching the result rows.
            crate::results::darken(v.panel_fill, crate::results::ROW_HOVER_DARKEN),
            v.widgets.noninteractive.bg_stroke.color,
            v.text_color(),
            v.weak_text_color(),
        )
    };

    if selected {
        let fill = if response.hovered() {
            crate::results::darken(sel_fill, 20)
        } else {
            sel_fill
        };
        ui.painter().rect_filled(rect, 0.0, fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, hover_fill);
    }

    // Thin separator at the bottom of the row, matching the result rows.
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, sep_color),
    );

    // If this row is being renamed in the sidebar, show the inline field instead
    // of the name (and skip the actions button / navigation for this row).
    let editing = rename
        .as_mut()
        .filter(|r| r.surface == RenameSurface::Sidebar && r.id == item.id);
    if let Some(state) = editing {
        let field_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 12.0, rect.top() + 4.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 4.0),
        );
        let builder = egui::UiBuilder::new()
            .max_rect(field_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center));
        let res = ui
            .scope_builder(builder, |ui| {
                inline_rename_field(
                    ui,
                    &mut state.buffer,
                    &mut state.take_focus,
                    egui::Id::new(("sidebar-rename", item.id)),
                    field_rect.width(),
                )
            })
            .inner;
        outcome.rename_commit = res.commit;
        outcome.rename_cancel = res.cancel;
        return outcome;
    }

    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let name_x = rect.left() + 12.0;
    let right_limit = rect.right() - 8.0;

    // Lay out the superscript "unsaved" marker first (if any) so we know how much
    // room to reserve for it — the name truncates with an ellipsis *before* it.
    let marker_galley = item.unsaved.then(|| {
        let mut job = egui::text::LayoutJob::default();
        job.append(icons::UNSAVED.codepoint, 0.0, unsaved_marker_format());
        ui.painter().layout_job(job)
    });
    let marker_gap = 2.0;
    let reserved = marker_galley
        .as_ref()
        .map_or(0.0, |g| g.size().x + marker_gap);
    let name_avail = (right_limit - name_x - reserved).max(0.0);

    let mut name_job = egui::text::LayoutJob::single_section(
        item.name.clone(),
        egui::TextFormat {
            font_id,
            color: text_color,
            ..Default::default()
        },
    );
    name_job.wrap = egui::text::TextWrapping::truncate_at_width(name_avail);
    let name_galley = ui.painter().layout_job(name_job);
    let name_w = name_galley.size().x;
    let name_top = rect.center().y - name_galley.size().y / 2.0;
    ui.painter()
        .galley(egui::pos2(name_x, name_top), name_galley, text_color);
    if let Some(marker) = marker_galley {
        // Top-aligned (small font) so it reads as a raised superscript asterisk.
        ui.painter().galley(
            egui::pos2(name_x + name_w + marker_gap, name_top),
            marker,
            text_color,
        );
    }

    outcome.action = row_actions_menu(
        ui,
        item,
        &response,
        rect,
        response.hovered(),
        text_color,
        weak_text,
    );
    outcome.clicked = response.clicked();
    outcome
}

/// Draws the row's "⋮" actions button and wires up both its click-menu and the
/// row's right-click context menu, returning the chosen action. The button is
/// interacted every frame (a stable anchor keeps its popup open even once the row
/// stops being hovered) but its glyph is only painted when `show_button` is set.
/// It's interacted after the row so it sits on top and steals clicks from the
/// row's navigation.
fn row_actions_menu(
    ui: &mut egui::Ui,
    item: &ListItem,
    row: &egui::Response,
    rect: egui::Rect,
    show_button: bool,
    text_color: egui::Color32,
    weak_text: egui::Color32,
) -> Option<QueryAction> {
    let btn_size = 24.0;
    let btn_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - btn_size / 2.0 - 2.0, rect.center().y),
        egui::vec2(btn_size, btn_size),
    );
    let btn_resp = ui.interact(
        btn_rect,
        egui::Id::new(("query-menu", item.id)),
        egui::Sense::click(),
    );
    if show_button || btn_resp.hovered() {
        let icon_font = icons::font_id(18.0);
        let icon_color = if btn_resp.hovered() {
            text_color
        } else {
            weak_text
        };
        ui.painter().text(
            btn_rect.center(),
            egui::Align2::CENTER_CENTER,
            icons::MORE.codepoint,
            icon_font,
            icon_color,
        );
    }

    // Opened by clicking "⋮" (anchored under the button) or by right-clicking
    // anywhere on the row (at the pointer).
    let mut action = None;
    if let Some(inner) = egui::Popup::menu(&btn_resp)
        .align(egui::RectAlign::BOTTOM_END)
        .show(query_actions_menu)
        && inner.inner.is_some()
    {
        action = inner.inner;
    }
    if let Some(inner) = egui::Popup::context_menu(row).show(query_actions_menu)
        && inner.inner.is_some()
    {
        action = inner.inner;
    }
    action
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
