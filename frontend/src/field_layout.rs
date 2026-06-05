//! The result-row field layout algorithm.
//!
//! Given the min/max width of each visible column and the available row width, this
//! decides how wide each column is and which line it sits on. There is no horizontal
//! scrolling: when columns can't all fit on one line at their minimum widths, they wrap
//! onto additional lines, spread as evenly as possible. The function is pure (no egui,
//! no I/O) so it can be unit-tested and memoized by the caller.

/// The width bounds of one visible column.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ColSize {
    pub(crate) min: f32,
    pub(crate) max: f32,
}

/// Where one column lands: its line index and its horizontal extent within the content
/// area (x offset from the left padding, and width).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    pub(crate) line: usize,
    pub(crate) x: f32,
    pub(crate) width: f32,
}

/// The computed layout: one [`Placement`] per input column (same order), plus the total
/// number of lines used.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FieldLayout {
    pub(crate) placements: Vec<Placement>,
    pub(crate) line_count: usize,
}

/// The cache key for a memoized [`FieldLayout`]: the visible columns' rounded bounds and
/// the rounded available width and gap. Layout only needs to be recomputed when this
/// changes, which is roughly once per frame while the window is being resized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutKey {
    cols: Vec<(u32, u32)>,
    avail: u32,
    gap: u32,
}

impl LayoutKey {
    pub(crate) fn new(cols: &[ColSize], avail: f32, gap: f32) -> Self {
        Self {
            cols: cols
                .iter()
                .map(|c| (c.min.round() as u32, c.max.round() as u32))
                .collect(),
            avail: avail.round().max(0.0) as u32,
            gap: gap.round().max(0.0) as u32,
        }
    }
}

/// Computes the field layout for `cols` (the visible columns, in order) given `avail`
/// content width and `gap` horizontal spacing between adjacent columns on a line.
pub(crate) fn compute_field_layout(cols: &[ColSize], avail: f32, gap: f32) -> FieldLayout {
    if cols.is_empty() {
        return FieldLayout {
            placements: Vec::new(),
            line_count: 0,
        };
    }

    let line_count = min_line_count(cols, avail, gap);
    let bounds = balanced_partition(cols, gap, line_count);

    let mut placements = vec![
        Placement {
            line: 0,
            x: 0.0,
            width: 0.0,
        };
        cols.len()
    ];
    for (line_idx, &(start, end)) in bounds.iter().enumerate() {
        let line = &cols[start..end];
        let widths = distribute_widths(line, avail, gap);
        let mut x = 0.0;
        for (offset, width) in widths.into_iter().enumerate() {
            placements[start + offset] = Placement {
                line: line_idx,
                x,
                width,
            };
            x += width + gap;
        }
    }

    FieldLayout {
        placements,
        line_count: bounds.len(),
    }
}

/// The fewest lines needed to lay the columns out at their minimum widths, found by
/// greedy next-fit: keep adding columns to the current line until the next one wouldn't
/// fit, then wrap. A single column wider than `avail` takes its own line and overflows.
fn min_line_count(cols: &[ColSize], avail: f32, gap: f32) -> usize {
    let mut lines = 1;
    let mut used = 0.0;
    for (i, col) in cols.iter().enumerate() {
        let added = if i == 0 || used == 0.0 {
            col.min
        } else {
            used + gap + col.min
        };
        if used > 0.0 && added > avail {
            lines += 1;
            used = col.min;
        } else {
            used = added;
        }
    }
    lines
}

/// Partitions the columns into exactly `line_count` contiguous groups, minimizing the
/// largest per-line load (sum of mins plus inter-column gaps). This spreads the columns
/// across the available lines rather than cramming the early ones. Returns each line's
/// `[start, end)` range over `cols`.
fn balanced_partition(cols: &[ColSize], gap: f32, line_count: usize) -> Vec<(usize, usize)> {
    if line_count <= 1 {
        return vec![(0, cols.len())];
    }

    // Binary-search the smallest "max load" for which the columns fit in <= line_count
    // contiguous groups, then materialize a partition at that threshold.
    let total_min: f32 = cols.iter().map(|c| c.min).sum();
    let total_gap = gap * (cols.len().saturating_sub(1)) as f32;
    let single_max = cols.iter().fold(0.0_f32, |acc, c| acc.max(c.min));
    let mut lo = single_max.max((total_min + total_gap) / line_count as f32);
    let mut hi = total_min + total_gap;
    // 30 iterations resolves px-scale widths far below sub-pixel precision.
    for _ in 0..30 {
        let mid = f32::midpoint(lo, hi);
        if lines_needed(cols, gap, mid) <= line_count {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    materialize_partition(cols, gap, hi, line_count)
}

/// Greedy count of how many contiguous lines are needed if no line's load (mins + gaps)
/// may exceed `limit`.
fn lines_needed(cols: &[ColSize], gap: f32, limit: f32) -> usize {
    let mut lines = 1;
    let mut used = 0.0;
    for col in cols {
        let added = if used == 0.0 {
            col.min
        } else {
            used + gap + col.min
        };
        if used > 0.0 && added > limit {
            lines += 1;
            used = col.min;
        } else {
            used = added;
        }
    }
    lines
}

/// Builds the actual line ranges for a given load `limit`, greedily filling each line.
/// `line_count` caps the number of lines: once on the last line, remaining columns go
/// there regardless of `limit`, so the partition always covers every column.
fn materialize_partition(
    cols: &[ColSize],
    gap: f32,
    limit: f32,
    line_count: usize,
) -> Vec<(usize, usize)> {
    let mut bounds = Vec::with_capacity(line_count);
    let mut start = 0;
    let mut used = 0.0;
    for (i, col) in cols.iter().enumerate() {
        let on_last_line = bounds.len() == line_count - 1;
        let added = if used == 0.0 {
            col.min
        } else {
            used + gap + col.min
        };
        if used > 0.0 && added > limit && !on_last_line {
            bounds.push((start, i));
            start = i;
            used = col.min;
        } else {
            used = added;
        }
    }
    bounds.push((start, cols.len()));
    bounds
}

/// Distributes `avail` width across one line's columns. With slack beyond the columns'
/// max widths, each gets its max plus a share of the surplus proportional to its max.
/// Between the sum of mins and sum of maxes, columns shrink from max toward min
/// proportional to their flex room. Below the sum of mins, each takes its min (and the
/// line overflows — text will clip).
fn distribute_widths(line: &[ColSize], avail: f32, gap: f32) -> Vec<f32> {
    let count = line.len();
    let content = avail - gap * (count.saturating_sub(1)) as f32;
    let sum_min: f32 = line.iter().map(|c| c.min).sum();
    let sum_max: f32 = line.iter().map(|c| c.max).sum();

    if content >= sum_max {
        let surplus = content - sum_max;
        if sum_max > 0.0 {
            line.iter()
                .map(|c| c.max + surplus * (c.max / sum_max))
                .collect()
        } else {
            // All columns have zero max: split the whole width equally.
            vec![content / count as f32; count]
        }
    } else if content >= sum_min {
        let deficit = sum_max - content;
        let flex = sum_max - sum_min;
        if flex > 0.0 {
            line.iter()
                .map(|c| c.max - deficit * ((c.max - c.min) / flex))
                .collect()
        } else {
            // No flex room (every column is fixed); fall back to equal widths.
            vec![content / count as f32; count]
        }
    } else {
        line.iter().map(|c| c.min).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(min: f32, max: f32) -> ColSize {
        ColSize { min, max }
    }

    /// Sum of widths on a given line, plus the gaps between them.
    fn line_extent(layout: &FieldLayout, line: usize, gap: f32) -> f32 {
        let widths: Vec<f32> = layout
            .placements
            .iter()
            .filter(|p| p.line == line)
            .map(|p| p.width)
            .collect();
        let gaps = gap * (widths.len().saturating_sub(1)) as f32;
        widths.iter().sum::<f32>() + gaps
    }

    #[test]
    fn empty_input() {
        let layout = compute_field_layout(&[], 1000.0, 8.0);
        assert_eq!(layout.line_count, 0);
        assert!(layout.placements.is_empty());
    }

    #[test]
    fn wide_window_expands_past_max_and_fills_width() {
        let cols = [col(50.0, 100.0), col(50.0, 300.0)];
        let layout = compute_field_layout(&cols, 1000.0, 0.0);
        assert_eq!(layout.line_count, 1);
        // Everything on one line; widths exceed the maxes and fill the full width.
        assert!(layout.placements.iter().all(|p| p.line == 0));
        assert!((line_extent(&layout, 0, 0.0) - 1000.0).abs() < 0.5);
        assert!(layout.placements[0].width > 100.0);
        assert!(layout.placements[1].width > 300.0);
        // Surplus is proportional to max, so the wider column stays wider.
        assert!(layout.placements[1].width > layout.placements[0].width);
    }

    #[test]
    fn mid_window_shrinks_toward_min() {
        let cols = [col(50.0, 200.0), col(50.0, 200.0)];
        // Total max = 400, total min = 100; pick something in between.
        let layout = compute_field_layout(&cols, 300.0, 0.0);
        assert_eq!(layout.line_count, 1);
        assert!((line_extent(&layout, 0, 0.0) - 300.0).abs() < 0.5);
        for p in &layout.placements {
            assert!(p.width >= 50.0 - 0.5 && p.width <= 200.0 + 0.5);
        }
    }

    #[test]
    fn exact_min_width() {
        let cols = [col(100.0, 200.0), col(100.0, 200.0)];
        let layout = compute_field_layout(&cols, 200.0, 0.0);
        assert_eq!(layout.line_count, 1);
        for p in &layout.placements {
            assert!((p.width - 100.0).abs() < 0.5);
        }
    }

    #[test]
    fn wraps_and_balances_evenly() {
        // Four equal columns, room for ~two per line at min width.
        let cols = [
            col(100.0, 100.0),
            col(100.0, 100.0),
            col(100.0, 100.0),
            col(100.0, 100.0),
        ];
        let layout = compute_field_layout(&cols, 250.0, 0.0);
        assert_eq!(layout.line_count, 2);
        // Balanced: two columns per line, not 3 + 1.
        let line0 = layout.placements.iter().filter(|p| p.line == 0).count();
        let line1 = layout.placements.iter().filter(|p| p.line == 1).count();
        assert_eq!((line0, line1), (2, 2));
    }

    #[test]
    fn wrapping_preserves_column_order() {
        let cols = [
            col(100.0, 100.0),
            col(100.0, 100.0),
            col(100.0, 100.0),
            col(100.0, 100.0),
            col(100.0, 100.0),
        ];
        let layout = compute_field_layout(&cols, 250.0, 0.0);
        // Lines are non-decreasing across the column order (contiguous wrapping).
        for w in layout.placements.windows(2) {
            assert!(w[1].line >= w[0].line);
        }
    }

    #[test]
    fn single_oversized_column_overflows_on_its_own_line() {
        let cols = [col(100.0, 100.0), col(400.0, 400.0), col(100.0, 100.0)];
        let layout = compute_field_layout(&cols, 200.0, 0.0);
        // The 400px column can't share a line at min width, so it gets its own.
        let big_line = layout.placements[1].line;
        assert!((layout.placements[1].width - 400.0).abs() < 0.5);
        assert!(
            layout
                .placements
                .iter()
                .filter(|p| p.line == big_line)
                .count()
                == 1
        );
    }

    #[test]
    fn all_equal_columns_get_equal_widths() {
        let cols = [col(50.0, 100.0), col(50.0, 100.0), col(50.0, 100.0)];
        let layout = compute_field_layout(&cols, 600.0, 0.0);
        assert_eq!(layout.line_count, 1);
        let first = layout.placements[0].width;
        for p in &layout.placements {
            assert!((p.width - first).abs() < 0.5);
        }
    }

    #[test]
    fn x_offsets_account_for_gaps() {
        let cols = [col(100.0, 100.0), col(100.0, 100.0)];
        let layout = compute_field_layout(&cols, 208.0, 8.0);
        assert!((layout.placements[0].x - 0.0).abs() < 0.5);
        assert!((layout.placements[1].x - 108.0).abs() < 0.5);
    }
}
