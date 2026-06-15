//! The central panel: renders the current query page's result rows in a configurable
//! multi-column layout, and handles row selection and double-click-to-play.

use std::rc::Rc;

use eframe::egui;
use egui::emath::GuiRounding;
use egui::text::{LayoutJob, TextWrapping};

use crate::columns::{ColumnMetadata, FontColor, FontSize, TextAlign};
use crate::field_layout::{ColSize, FieldLayout, LayoutKey, Placement, compute_field_layout};
use crate::{ACCENT_BLUE, App, QueryState};

/// Vertical padding above and below a row's content.
const ROW_PAD_Y: f32 = 6.0;
/// Horizontal padding on the left and right of a row's content.
const TEXT_PAD_X: f32 = 8.0;
/// Horizontal gap between adjacent columns on a line.
const COL_GAP: f32 = 16.0;
/// How much darker an un-selected row gets on hover (per RGB channel). Small, so
/// the hover effect is only slightly darker than an un-hovered row.
pub(crate) const ROW_HOVER_DARKEN: u8 = 10;

/// Returns `color` darkened by `amount` on each RGB channel (alpha unchanged).
pub(crate) fn darken(color: egui::Color32, amount: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        color.r().saturating_sub(amount),
        color.g().saturating_sub(amount),
        color.b().saturating_sub(amount),
        color.a(),
    )
}

impl App {
    /// Returns the memoized field layout for the given columns and width, recomputing
    /// only when the column set or available width changes.
    fn field_layout(&mut self, cols: &[ColSize], avail: f32) -> Rc<FieldLayout> {
        let key = LayoutKey::new(cols, avail, COL_GAP);
        if let Some((cached_key, layout)) = &self.field_layout_cache
            && *cached_key == key
        {
            return Rc::clone(layout);
        }
        let layout = Rc::new(compute_field_layout(cols, avail, COL_GAP));
        self.field_layout_cache = Some((key, Rc::clone(&layout)));
        layout
    }

    /// Computes the row-independent layout (visible columns, widths, line positions,
    /// fonts/colors, and the shared row height) for the current results.
    fn row_metrics(&mut self, ui: &egui::Ui, state: &QueryState) -> ResultMetrics {
        // One metadata entry per result column (defaults fill any gap), keeping only the
        // visible ones paired with their original cell index.
        let col_count = state.rows.first().map_or(0, Vec::len);
        let visible: Vec<(usize, ColumnMetadata)> = (0..col_count)
            .map(|i| (i, state.columns.get(i).cloned().unwrap_or_default()))
            .filter(|(_, meta)| !meta.hide)
            .collect();
        let col_sizes: Vec<ColSize> = visible
            .iter()
            .map(|(_, m)| ColSize {
                min: m.min_width,
                max: m.max_width,
            })
            .collect();

        let avail = (ui.available_width() - TEXT_PAD_X * 2.0).max(0.0);
        let layout = self.field_layout(&col_sizes, avail);

        // Per-line height = the tallest column on that line; the row's content height is
        // the sum, so every row shares one fixed height (the layout is the same for all
        // rows). Cumulative line tops drive vertical placement.
        let body_h = ui.text_style_height(&egui::TextStyle::Body);
        let small_h = ui.text_style_height(&egui::TextStyle::Small);
        let mut line_heights = vec![0.0_f32; layout.line_count];
        for (vis_idx, p) in layout.placements.iter().enumerate() {
            let h = match visible[vis_idx].1.font_size {
                FontSize::Small => small_h,
                FontSize::Normal => body_h,
            };
            line_heights[p.line] = line_heights[p.line].max(h);
        }
        let mut line_tops = vec![0.0_f32; layout.line_count];
        let mut acc = 0.0;
        for (i, h) in line_heights.iter().enumerate() {
            line_tops[i] = acc;
            acc += h;
        }
        let content_h = if layout.line_count == 0 { body_h } else { acc };

        ResultMetrics {
            visible,
            layout,
            line_tops,
            line_heights,
            body_font: egui::TextStyle::Body.resolve(ui.style()),
            small_font: egui::TextStyle::Small.resolve(ui.style()),
            text_color: ui.visuals().text_color(),
            weak_color: ui.visuals().weak_text_color(),
            row_height: content_h + ROW_PAD_Y * 2.0,
        }
    }

    pub(crate) fn render_results(&mut self, ui: &mut egui::Ui) {
        let Some(current_id) = self.current.query_id() else {
            return;
        };
        let Some(results) = self.page_results(current_id) else {
            egui::CentralPanel::default().show_inside(ui, |_ui| {});
            return;
        };

        let ctx = ui.ctx().clone();
        egui::CentralPanel::default().show_inside(ui, |ui| {
            let state = results.lock().unwrap();

            if let Some(err) = &state.error {
                ui.colored_label(egui::Color32::RED, err);
            }

            if state.rows.is_empty() {
                return;
            }

            let mut clicked: Option<(usize, egui::Modifiers)> = None;
            let mut double_clicked: Option<(usize, String)> = None;

            let pending_locate = self
                .pending_scroll_to_row
                .take()
                .filter(|i| *i < state.rows.len());
            if let Some(idx) = pending_locate {
                self.selection.clear();
                self.selection.insert(idx);
                self.selection_anchor = Some(idx);
            }

            let metrics = self.row_metrics(ui, &state);
            let row_height = metrics.row_height;
            let row_layout = RowLayout {
                visible: &metrics.visible,
                placements: &metrics.layout.placements,
                line_tops: &metrics.line_tops,
                line_heights: &metrics.line_heights,
                body_font: metrics.body_font.clone(),
                small_font: metrics.small_font.clone(),
                text_color: metrics.text_color,
                weak_color: metrics.weak_color,
                row_height,
            };

            // Only highlight the now-playing row when it belongs to this page.
            let current_row = {
                let ct = self.current_track.lock().unwrap();
                ct.as_ref()
                    .filter(|c| c.source_page == current_id)
                    .and_then(|c| c.row_index)
            };
            let rows = &state.rows;
            let selection = &self.selection;
            let track_id_column = state.track_id_column;

            let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if let Some(idx) = pending_locate {
                let viewport_h = ui.available_height();
                let target = (idx as f32 * row_height) - (viewport_h - row_height).max(0.0) * 0.5;
                scroll_area = scroll_area.vertical_scroll_offset(target.max(0.0));
            }
            scroll_area.show_rows(ui, row_height, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for index in range {
                    let cells = &rows[index];
                    let track_id = track_id_column.and_then(|i| cells.get(i).map(String::as_str));
                    let is_current = current_row == Some(index);
                    let resp = draw_row(
                        ui,
                        &row_layout,
                        cells,
                        selection.contains(&index),
                        is_current,
                    );
                    if resp.double_clicked() {
                        if let Some(id) = track_id {
                            double_clicked = Some((index, id.to_string()));
                        }
                    } else if resp.clicked() {
                        let mods = ui.input(|i| i.modifiers);
                        clicked = Some((index, mods));
                    }
                }
            });
            drop(state);

            if let Some((index, mods)) = clicked {
                self.handle_row_click(index, mods);
            }
            if let Some((index, id)) = double_clicked {
                self.play_track(current_id, index, &id, &ctx);
            }
        });
    }

    pub(crate) fn handle_row_click(&mut self, index: usize, modifiers: egui::Modifiers) {
        if modifiers.shift {
            let anchor = self.selection_anchor.unwrap_or(index);
            let (lo, hi) = if anchor <= index {
                (anchor, index)
            } else {
                (index, anchor)
            };
            self.selection.clear();
            for i in lo..=hi {
                self.selection.insert(i);
            }
        } else if modifiers.command || modifiers.ctrl {
            if !self.selection.remove(&index) {
                self.selection.insert(index);
            }
            self.selection_anchor = Some(index);
        } else {
            self.selection.clear();
            self.selection.insert(index);
            self.selection_anchor = Some(index);
        }
    }
}

/// Owned, row-independent layout data computed once per frame and borrowed by each row's
/// [`RowLayout`].
struct ResultMetrics {
    visible: Vec<(usize, ColumnMetadata)>,
    layout: Rc<FieldLayout>,
    line_tops: Vec<f32>,
    line_heights: Vec<f32>,
    body_font: egui::FontId,
    small_font: egui::FontId,
    text_color: egui::Color32,
    weak_color: egui::Color32,
    row_height: f32,
}

/// The row-independent layout shared by every result row: which columns go where, their
/// widths and line positions, and the fonts/colors to draw them with.
struct RowLayout<'a> {
    visible: &'a [(usize, ColumnMetadata)],
    placements: &'a [Placement],
    line_tops: &'a [f32],
    line_heights: &'a [f32],
    body_font: egui::FontId,
    small_font: egui::FontId,
    text_color: egui::Color32,
    weak_color: egui::Color32,
    row_height: f32,
}

fn draw_row(
    ui: &mut egui::Ui,
    layout: &RowLayout,
    cells: &[String],
    selected: bool,
    is_current: bool,
) -> egui::Response {
    let desired = egui::vec2(ui.available_width(), layout.row_height);
    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

    // Snap the row to the physical pixel grid before painting. Allocated rows sit at
    // fractional pixel positions, so without this the background fills (and the
    // separator) land on different subpixel offsets row-to-row, making the separators
    // render with inconsistent darkness/thickness. Consecutive rows share an exact edge,
    // so snapping keeps them gap-free.
    let ppp = ui.ctx().pixels_per_point();
    let rect = rect.round_to_pixels(ppp);

    let visuals = ui.visuals();
    let base_bg = if selected {
        let base = visuals.selection.bg_fill;
        if response.hovered() {
            darken(base, 20)
        } else {
            base
        }
    } else if response.hovered() {
        // Only a slight darkening on hover, so the effect is subtle.
        darken(visuals.extreme_bg_color, ROW_HOVER_DARKEN)
    } else {
        visuals.extreme_bg_color
    };

    ui.painter().rect_filled(rect, 0.0, base_bg);

    if is_current && !selected {
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(46, 124, 246, 16),
        );
    }

    if is_current {
        let accent_rect =
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height()));
        ui.painter().rect_filled(accent_rect, 0.0, ACCENT_BLUE);
    }

    // Thin separator along the bottom of the row. Drawn as a pixel-aligned filled
    // rectangle (not a 1px line): the rect's edges land on physical pixel boundaries so
    // it renders crisply and identically on every row, and because it sits just *inside*
    // the row's bottom edge the next row's background fill cannot paint over it.
    // Soften the separator a touch by letting the row background show through.
    let sep_color = visuals
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.5);
    let sep_h = 1.0_f32.round_to_pixels(ppp);
    let sep_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - sep_h),
        rect.right_bottom(),
    );
    ui.painter().rect_filled(sep_rect, 0.0, sep_color);

    for (vis_idx, (col_idx, meta)) in layout.visible.iter().enumerate() {
        let placement = layout.placements[vis_idx];
        let value = cells.get(*col_idx).map_or("", String::as_str);
        let formatted = match &meta.formatter {
            Some(f) => f.format(value).unwrap_or_else(|| value.to_string()),
            None => value.to_string(),
        };
        let text = if meta.prefix.is_empty() && meta.suffix.is_empty() {
            formatted
        } else {
            format!("{}{}{}", meta.prefix, formatted, meta.suffix)
        };

        let font = match meta.font_size {
            FontSize::Small => layout.small_font.clone(),
            FontSize::Normal => layout.body_font.clone(),
        };
        let color = match meta.font_color {
            FontColor::Light => layout.weak_color,
            FontColor::Default => layout.text_color,
        };
        let format = egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        };
        let mut job = LayoutJob::single_section(text, format);
        job.wrap = TextWrapping::truncate_at_width(placement.width.max(0.0));
        let galley = ui.painter().layout_job(job);
        let size = galley.size();

        let cell_left = rect.left() + TEXT_PAD_X + placement.x;
        let slack = (placement.width - size.x).max(0.0);
        let x = match meta.text_align {
            TextAlign::Left => cell_left,
            TextAlign::Right => cell_left + slack,
            TextAlign::Center => cell_left + slack * 0.5,
        };
        let line_top = rect.top() + ROW_PAD_Y + layout.line_tops[placement.line];
        let y = line_top + (layout.line_heights[placement.line] - size.y) * 0.5;
        ui.painter().galley(egui::pos2(x, y), galley, color);
    }

    response
}
