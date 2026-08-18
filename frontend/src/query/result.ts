// The structured result of a query, backed by the Arrow table DuckDB sent.
//
// A `QueryResult` keeps every column of the decoded table — hidden ones
// included, since a lineage index addresses columns positionally — and derives
// display text on demand rather than stringifying the whole result up front.
// That keeps a `relativeTime` cell fresh across repaints (it re-reads
// `Date.now()` every call) and keeps raw values around for lineage to read,
// without a second copy of anything.
//
// `QueryResult` is a *class*, not a stylistic choice: Solid's store merges an
// assigned object into the one already at a store leaf when both are
// "wrappable", and a class instance isn't — so assigning one always swaps the
// reference, which is what the grid's effect watches (see `setTabResult` in
// `state/store.tsx`).

import * as arrow from "apache-arrow";
import type { DataType as ArrowType, Table } from "apache-arrow";
import type { AnnotationValue } from "querydown-js";
import {
  isListLikeValue,
  isListType,
  isMonthDayNanoInterval,
  readMonthDayNanoInterval,
  stringifyArrowValue,
} from "../api/query";
import {
  columnMetadataFromAnnotation,
  defaultColumnMetadata,
  type ColumnMetadata,
} from "./columns";
import { displayText } from "./format";
import type { ColSize } from "./fieldLayout";

/** One output column of a query: what it is, and how to read it. Columns are
 * kept in projection order — the Arrow column order — hidden ones included, so
 * a lineage index always addresses the right one. */
export interface ResultColumn {
  /** Position in the query's projection (= the Arrow column index). */
  index: number;
  meta: ColumnMetadata;
  isList: boolean;
}

/** Where a display row's values live, once a re-read has patched it in. */
interface Patch {
  table: Table;
  row: number;
}

/** A query result: the row count, every column (hidden included), and the
 * visible ones the grid lays out and paints — computed once at construction.
 * Cell text is derived on read, not precomputed. */
export class QueryResult {
  readonly rowCount: number;
  readonly columns: readonly ResultColumn[];
  /** The columns the grid lays out and paints, in order (`!meta.hide`). */
  readonly visible: readonly ResultColumn[];

  private readonly table: Table;
  private readonly types: readonly (ArrowType | undefined)[];
  /** Rows a re-read has pointed at a fresh one-row table (see `patchRow`).
   * Empty in the ordinary case; dropped wholesale when the tab re-runs. */
  private readonly patches = new Map<number, Patch>();

  constructor(table: Table, columns: readonly ResultColumn[]) {
    this.table = table;
    this.rowCount = table.numRows;
    this.columns = columns;
    this.visible = columns.filter((c) => !c.meta.hide);
    this.types = table.schema.fields.map((f) => f.type);
  }

  /** The raw value in a cell, as Arrow gives it — what a key or a track id is
   * read from. `null`/`undefined` for a NULL cell.
   *
   * Interval columns are the one exception to "as Arrow gives it": apache-arrow
   * decodes them to garbage, so they come back as text read from the buffer
   * directly (see {@link readMonthDayNanoInterval}). */
  value(row: number, column: number): unknown {
    const patch = this.patches.get(row);
    const vector = (patch ? patch.table : this.table).getChildAt(column);
    if (!vector) return undefined;
    const index = patch ? patch.row : row;
    return isMonthDayNanoInterval(this.types[column])
      ? readMonthDayNanoInterval(vector, index)
      : vector.get(index);
  }

  /** The raw value's plain string form — no formatter, prefix or suffix — what
   * a lineage key's equality filter is built from (a composite key can include
   * a typed column, e.g. a `TIMESTAMP`, whose text form has to match what the
   * compiler would emit for it). */
  keyText(row: number, column: number): string {
    return stringifyArrowValue(this.value(row, column), this.types[column]);
  }

  /** A scalar cell's display text, formatted **now**. */
  text(row: number, column: ResultColumn): string {
    const raw = this.value(row, column.index);
    return displayText(
      column.meta,
      stringifyArrowValue(raw, this.types[column.index]),
    );
  }

  /** A list cell's pill texts, formatted now. */
  pills(row: number, column: ResultColumn): readonly string[] {
    const raw = this.value(row, column.index);
    if (raw == null) return [];
    return Array.from(raw as Iterable<unknown>, (e) =>
      displayText(column.meta, stringifyArrowValue(e)),
    );
  }

  /** Re-points one row at a re-read (see `query/rowDml.ts`): from then on its
   * cells come from `table`'s row `from` instead of the run's own table.
   * Returns `false` (skipping the patch) when `table`'s projection doesn't
   * match this result's — a different column count means a different query. */
  patchRow(row: number, table: Table, from: number): boolean {
    if (table.schema.fields.length !== this.table.schema.fields.length) {
      return false;
    }
    this.patches.set(row, { table, row: from });
    return true;
  }
}

/** The visible columns' width bounds, in order — the input to the field layout.
 * Derived, not stored, so callers memoize on `(colSizes, avail, gap)`. */
export function colSizesOf(result: QueryResult): ColSize[] {
  return result.visible.map((c) => ({
    min: c.meta.min_width,
    max: c.meta.max_width,
  }));
}

/** Classifies every column of `table` (list vs. scalar) and pairs it with its
 * metadata, in Arrow column order. Shared by {@link buildResultFromArrow} and
 * {@link buildResultFromCells}, which differ only in where the metadata comes
 * from. */
function buildResult(
  table: Table,
  metas: readonly ColumnMetadata[],
): QueryResult {
  const fields = table.schema.fields;
  const rowCount = table.numRows;
  const columns: ResultColumn[] = fields.map((field, i) => {
    // Detect list columns from the Arrow type; fall back to inspecting the
    // first non-null value, since apache-arrow 18 can't classify some DuckDB
    // list types (see `isListType`). Without this, a list column decodes down
    // the scalar path and renders as bracketed text instead of pills.
    //
    // Interval columns are exempt from the value-level fallback: apache-arrow
    // decodes them to an `Int32Array`, which is iterable and so reads as
    // list-like, and a duration would render as a row of pills of nonsense.
    let isList = isListType(field.type);
    if (!isList && !isMonthDayNanoInterval(field.type)) {
      const vector = table.getChildAt(i);
      if (vector) {
        for (let r = 0; r < rowCount; r++) {
          const v = vector.get(r);
          if (v != null) {
            isList = isListLikeValue(v);
            break;
          }
        }
      }
    }
    return { index: i, meta: metas[i], isList };
  });
  return new QueryResult(table, columns);
}

/** Builds a {@link QueryResult} from a decoded Arrow table plus the compiler's
 * positional `columnAnnotations`. Resolves each column's metadata; every
 * column is kept (hidden ones included — see {@link QueryResult}). */
export function buildResultFromArrow(
  table: Table,
  annotations: readonly (AnnotationValue | null)[],
): QueryResult {
  return buildResult(
    table,
    table.schema.fields.map((_, i) =>
      columnMetadataFromAnnotation(annotations[i] ?? null),
    ),
  );
}

/** Builds a {@link QueryResult} from a real Arrow table plus already-resolved
 * metadata — the dev/test seam's fixture path (no compiler, so no annotations
 * to resolve). Otherwise identical to {@link buildResultFromArrow}, so the
 * seeded grid exercises the same decode production does. */
export function buildResultFromCells(
  table: Table,
  metas: readonly ColumnMetadata[],
): QueryResult {
  return buildResult(table, metas);
}

/** Builds a {@link QueryResult} from rows of plain strings (and nulls) — what a
 * record editor query hands back (see `query/recordData.ts`), which has already
 * lost its Arrow typing on the way through. Every column comes back as a Utf8
 * vector, which is honest: display text is already all these rows are.
 *
 * `hiddenColumns` names the positions (the key columns the record picker
 * addresses a chosen row by, typically) that should occupy a column slot
 * without the grid laying one out for them — the same "kept but not shown"
 * treatment {@link QueryResult} gives every hidden column. The column count is
 * read off the first row; with none, the result is the same empty one a real
 * zero-row query would produce. */
export function buildResultFromStringRows(
  rows: readonly (readonly (string | null)[])[],
  hiddenColumns: readonly number[] = [],
): QueryResult {
  const columnCount = rows[0]?.length ?? 0;
  const vectors: Record<string, arrow.Vector> = {};
  const metas: ColumnMetadata[] = [];
  for (let c = 0; c < columnCount; c++) {
    vectors[`c${c}`] = arrow.vectorFromArray(rows.map((r) => r[c] ?? null));
    metas.push({ ...defaultColumnMetadata(), hide: hiddenColumns.includes(c) });
  }
  return buildResultFromCells(new arrow.Table(vectors), metas);
}
