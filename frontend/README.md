# Collectune Frontend

Frontend application for Collectune.

## Icons

Icons come from [Material Symbols](https://fonts.google.com/icons?icon.set=Material+Symbols&icon.size=24&icon.color=%231f1f1f&icon.style=Rounded) ("fill" style),
via the [`egui_material_icons`](https://crates.io/crates/egui_material_icons) crate.
App concepts are mapped to specific glyphs in [`src/icons.rs`](src/icons.rs); refer to
those semantic names (e.g. `icons::SAVE`) rather than raw `ICON_*` constants.

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
| `formatter`  | JSON object                           | —                | Transforms each cell value for display — see below. |

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

### Formatter

The `formatter` field is a JSON object whose `type` selects a typed transform applied
to each cell value before it is displayed. Cell values arrive as plain strings, so each
formatter parses the string it is given; if it can't parse a value (e.g. a `NULL`, an
empty cell, or unexpected text), that cell falls back to the raw, unformatted string.
`prefix`/`suffix` wrap the *formatted* result. Parsing and formatting live in
[`src/format.rs`](src/format.rs).

#### `number`

```json
{ "type": "number", "decimalPlaces": [0, 1] }
```

Formats a number to a minimum and maximum number of fraction digits (no thousands
grouping). `decimalPlaces` is `[min, max]`; a one-element list `[n]` or a plain number
`n` sets `min == max == n`; omitted defaults to `[0, 0]`. With `[0, 1]`: `3` → `3`,
`3.0` → `3`, `3.14` → `3.1`.

#### `timestamp`

```json
{ "type": "timestamp", "format": "%Y-%m-%d" }
```

Formats a timestamp according a string containing [formatting specifiers](https://docs.rs/jiff/latest/jiff/fmt/strtime/index.html#conversion-specifications) from the **jiff** library. Accepts a datetime (`2026-06-07 14:30:45`), a date (`2026-06-07`), or integer epoch seconds.

#### `duration`

```json
{ "type": "duration" }
```

Formats a number of seconds as `M:SS`, rounded to the nearest second. Minutes are shown
without padding (and as `0` when under a minute); seconds are zero-padded. `245` → `4:05`,
`0` → `0:00`. There is no configuration yet.

#### `relativeTime`

```json
{
  "type": "relativeTime",
  "units": ["minutes", "hours", "days", "weeks", "months", "years"]
}
```

Formats a timestamp as the time elapsed relative to now. It picks the largest of the
available `units` that yields a non-zero whole number, e.g. `4 days ago`. If even the
smallest available unit rounds to zero it still uses that unit (`0 minutes ago`); future
timestamps render as `in N units`. `units` defaults to all six when omitted.

