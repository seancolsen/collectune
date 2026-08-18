// A small, fixed structured result for the deterministic grid snapshot (§6/§7):
// the five "Lemonade" rows from `whole_app.png`. This is the whole visual
// baseline — a canned fixture for one snapshot, NOT the forbidden large-dataset
// generator. Built as a real Arrow table (through `buildResultFromCells`) with
// honest column types, so the seeded grid exercises the same decode path
// production does — list detection, `stringifyArrowValue`, the formatters —
// rather than a shortcut that only looks right.

import * as arrow from "apache-arrow";
import { defaultColumnMetadata, type ColumnMetadata } from "../query/columns";
import type { Formatter } from "../query/format";
import type { LineageMapping, RecordKeyColumns } from "../query/lineage";
import { buildResultFromCells, type QueryResult } from "../query/result";

/** The fixed row count of the canned Lemonade grid. */
export const ROW_COUNT = 5;

function meta(overrides: Partial<ColumnMetadata>): ColumnMetadata {
  return { ...defaultColumnMetadata(), ...overrides };
}

const durationFmt: Formatter = { type: "duration" };
const ratingFmt: Formatter = { type: "number", decimalPlaces: [0, 1] };

/** Builds an `Interval(MONTH_DAY_NANO)` vector holding whole seconds — the type
 * and byte layout DuckDB sends for an INTERVAL column, which `file.duration`
 * became in migration 0003.
 *
 * Hand-built because apache-arrow has no builder for this interval flavor (it
 * cannot read one either — see `readMonthDayNanoInterval` in `api/query.ts`).
 * Laying the bytes down the way arrow-rs writes them is what keeps this fixture
 * honest: the seeded grid decodes a duration exactly as production does, so the
 * snapshot would catch a regression in that decode. */
function intervalColumn(seconds: readonly number[]): arrow.Vector {
  const buffer = new ArrayBuffer(16 * seconds.length);
  const view = new DataView(buffer);
  seconds.forEach((s, i) => {
    view.setInt32(i * 16, 0, true); // months
    view.setInt32(i * 16 + 4, 0, true); // days
    view.setBigInt64(i * 16 + 8, BigInt(s) * 1_000_000_000n, true); // nanoseconds
  });
  return new arrow.Vector([
    arrow.makeData({
      type: new arrow.Interval(arrow.IntervalUnit.MONTH_DAY_NANO),
      length: seconds.length,
      nullCount: 0,
      data: new Int32Array(buffer),
    }),
  ]);
}

/** Builds a `List<Utf8>` vector from per-row pill arrays — the same shape a
 * DuckDB list column decodes to (see `result.test.ts`, which this mirrors). */
function listColumn(rows: readonly (readonly string[])[]): arrow.Vector {
  const type = new arrow.List(
    arrow.Field.new({ name: "item", type: new arrow.Utf8(), nullable: true }),
  );
  const b = arrow.makeBuilder({ type, nullValues: [null] });
  // The List builder accepts a plain array at runtime; its typed value is a
  // Vector, so cast to satisfy the overload.
  for (const r of rows) b.append(r as unknown as arrow.Vector<arrow.Utf8>);
  return b.finish().toVector();
}

/** Per-row identity data `dev/seed.ts` can ask to have appended as extra hidden
 * columns, so `?tracks=`/`?records=` point the lineage mapping at a real Arrow
 * column instead of a value snapshotted outside the table (see the plan's "the
 * store keeps only what the analysis found" — the fixture has to hand it a
 * column to keep, not a value). */
export interface LemonadeIdentity {
  /** Per-row track ids; a row past the array's end carries none (unplayable),
   * matching a query where only some rows trace to `track.id`. */
  trackIds?: readonly string[];
  /** Per-row primary-key values, one hidden column per named table (keyed by a
   * single `id` column, as the schema's tables all are). */
  recordKeys?: Readonly<Record<string, readonly string[]>>;
}

/** The canned Lemonade grid result plus the lineage mapping `identity` asks
 * for. Column metadata mirrors the display treatment in `whole_app.png`
 * (widths/fonts/colors/alignment/prefix/suffix); column *types* are honest
 * (`Int32`, `Float64`, a `List<Utf8>`) rather than the plain strings a
 * stringified fixture would use. */
export function lemonadeGridResult(identity: LemonadeIdentity = {}): {
  result: QueryResult;
  lineage: LineageMapping;
} {
  const vectors: Record<string, arrow.Vector> = {};
  const metas: ColumnMetadata[] = [];
  const push = (
    name: string,
    vector: arrow.Vector,
    m: ColumnMetadata,
  ): number => {
    vectors[name] = vector;
    metas.push(m);
    return metas.length - 1;
  };

  // Hidden id column — dropped from the visible set but still occupies a
  // result-column index (exercises the hide path).
  push(
    "id",
    arrow.vectorFromArray(["t1", "t2", "t3", "t4", "t5"]),
    meta({ hide: true }),
  );
  push(
    "artists",
    listColumn([
      ["Beyoncé"],
      ["Beyoncé"],
      ["Beyoncé", "Jack White"],
      ["Beyoncé"],
      ["Beyoncé", "The Weeknd"],
    ]),
    meta({ min_width: 130, max_width: 300 }),
  );
  push(
    "year",
    arrow.vectorFromArray([2016, 2016, 2016, 2016, 2016], new arrow.Int32()),
    meta({ min_width: 44, max_width: 50 }),
  );
  push(
    "album",
    arrow.vectorFromArray([
      "Lemonade",
      "Lemonade",
      "Lemonade",
      "Lemonade",
      "Lemonade",
    ]),
    meta({ min_width: 80, max_width: 200 }),
  );
  push(
    "track_number",
    arrow.vectorFromArray([1, 2, 3, 4, 5], new arrow.Int32()),
    meta({
      min_width: 26,
      max_width: 40,
      font_size: "small",
      font_color: "light",
      text_align: "right",
      prefix: "#",
    }),
  );
  push(
    "title",
    arrow.vectorFromArray([
      "Pray You Catch Me",
      "Hold Up",
      "Don't Hurt Yourself",
      "Sorry",
      "6 Inch",
    ]),
    meta({ min_width: 150, max_width: 420 }),
  );
  push(
    "duration",
    intervalColumn([196, 221, 234, 233, 260]),
    meta({
      min_width: 40,
      max_width: 50,
      font_size: "small",
      font_color: "light",
      text_align: "right",
      formatter: durationFmt,
    }),
  );
  push(
    "rating",
    arrow.vectorFromArray([3.5, 4, 4, 4, 4.5], new arrow.Float64()),
    meta({
      min_width: 46,
      max_width: 56,
      text_align: "right",
      suffix: " ☆",
      formatter: ratingFmt,
    }),
  );

  let trackIdColumn: number | undefined;
  if (identity.trackIds) {
    const ids = identity.trackIds;
    const padded = Array.from({ length: ROW_COUNT }, (_, r) => ids[r] ?? null);
    trackIdColumn = push(
      "seed_track_id",
      arrow.vectorFromArray(padded),
      meta({ hide: true }),
    );
  }

  const records: RecordKeyColumns[] = [];
  for (const [table, keys] of Object.entries(identity.recordKeys ?? {})) {
    const padded = Array.from({ length: ROW_COUNT }, (_, r) => keys[r] ?? null);
    const idx = push(
      `seed_record_${table}_id`,
      arrow.vectorFromArray(padded),
      meta({ hide: true }),
    );
    records.push({ table, keyColumns: ["id"], keyIndices: [idx] });
  }

  const table = new arrow.Table(vectors);
  const result = buildResultFromCells(table, metas);
  return { result, lineage: { trackIdColumn, records } };
}

/** An empty result of `rowCount` rows and no visible columns — what stands in
 * for "N results" without a real compile, so the toolbar's result count has
 * something to report. A real `Table` (one hidden `Null`-typed column, so
 * `numRows` is honest) rather than a bare object literal: `QueryResult` is a
 * class with private fields, so nothing but a real instance satisfies its
 * type. */
export function emptyCountResult(rowCount: number): QueryResult {
  const table = new arrow.Table({
    seed_count: arrow.vectorFromArray(new Array<null>(rowCount).fill(null)),
  });
  return buildResultFromCells(table, [
    { ...defaultColumnMetadata(), hide: true },
  ]);
}
