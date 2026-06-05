# Collectune Frontend

Frontend application for Collectune.

## Icons

https://phosphoricons.com/?weight=bold

## Result column display metadata

Querydown lets a query attach a JSON-like metadata blob to each result column. The
compiler surfaces these as one entry per output column (positionally aligned with the
result columns, `null` where a column has no metadata), and the frontend uses them to
configure how each column is displayed. Parsing and normalization live in
[`src/columns.rs`](src/columns.rs); the wrapping/width layout lives in
[`src/field_layout.rs`](src/field_layout.rs); rendering lives in
[`src/results.rs`](src/results.rs).

Every field is optional and has a default, so there is always a concrete value to render
with. Unknown keys are ignored, and a blob that isn't a JSON object is treated as all
defaults.

| Field        | Type                                  | Default          | Meaning |
|--------------|---------------------------------------|------------------|---------|
| `hide`       | `"yes"` \| `"no"`                     | `"no"`           | When `"yes"`, the column is not rendered, but its data is still available to the row (e.g. a track id needed to play a track but not shown). |
| `width`      | `number` \| `[number]` \| `[number, number]` | `[0, 500]` | Column min/max width in pixels — see below. |
| `color`      | `"default"` \| `"light"`              | `"default"`      | `"light"` renders the text in the de-emphasized (weak) color. |
| `size`       | `"normal"` \| `"small"`               | `"normal"`       | `"small"` renders the text in a smaller font. |
| `align`      | `"left"` \| `"right"` \| `"center"`   | `"left"`         | Horizontal alignment of the text within the column. |
| `prefix`     | `string`                              | `""`             | Text prepended to every cell value in the column. |
| `suffix`     | `string`                              | `""`             | Text appended to every cell value in the column. |

### Width

Each column resolves to a `min_width` and a `max_width` in pixels:

- A two-element list `[min, max]` sets both bounds.
- A one-element list `[min]` sets the min and keeps the default max (`500`).
- A plain number `n` sets `min == max == n` (a fixed width).
- Nothing / an empty list falls back to `[0, 500]`.
- If `min` ends up greater than `max`, the two are swapped.

### Layout

There is no horizontal scrolling. Given the min/max widths of all *visible* columns and
the available row width, the layout is computed as follows:

- When the window is wide enough for every column's `max_width`, each column gets its max
  and the remaining width is distributed proportionally to each column's max, spreading
  the columns across the row.
- As the window narrows, columns scale down from their max toward their min.
- Once all columns are at their min and still don't fit, columns wrap onto additional
  lines. Wrapping is balanced so columns are distributed across the lines as evenly as
  possible rather than cramming the early lines.

The layout is identical for every row, so it is computed once per frame and memoized
(keyed on the visible columns' bounds and the available width), recomputing only while
the window is being resized.

