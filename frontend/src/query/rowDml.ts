// A DML request run in the context of one query result row.
//
// Both of the app's writes happen against a row that's on screen: the play log,
// written as a track finishes, and the record editor's save. Neither shows up in
// the grid on its own — the row was decoded from a result set that the write has
// just made out of date, by exactly one row.
//
// Re-running the whole query would be the wrong fix. A query filtered on "fewer
// than ten plays" drops the track the listener has just finished the moment its
// tenth play lands: the row they were looking at vanishes as its reward, and
// anything they wanted to do with it goes with it. Nor can the row be patched
// where it sits — a play count is an aggregate DuckDB computed, and guessing at
// what a write did to a derived value is exactly the arithmetic the frontend
// shouldn't be doing. What the row needs is the *same query, restricted to the
// one record it stands for*:
//
//   • A **sectioned** query — the builder's base/filter/sort/display — is picked
//     apart. The filter section is replaced with conditions matching the record's
//     key, the sort is dropped (one row has no order), and the base and display
//     are left exactly as they were: the same compiler run over the same display
//     produces the same columns, and the single row that comes back is the row,
//     refreshed.
//
//   • A **full**, hand-written query can't be picked apart, so it's re-run whole
//     and the row looked for in the answer, matched on the records it identifies.
//     It may well not be there — the query may have filtered it out, which is the
//     very thing the narrowed path exists to avoid — and then the row is left as
//     it was. That's the trade the whole-query path makes.
//
// The module deals only in *reading the row back*; where those cells then go
// (into the tab's stored result, in place) is the store's business.

import type { Table } from "apache-arrow";
import {
  dml,
  type DmlOperation,
  type DmlResult,
  type Preset,
} from "api-client";
import { runSql, stringifyArrowValue } from "../api/query";
import { compileSavedQuery } from "./compile";
import type { QueryDefinition } from "./definition";
import { analyzeColumnSources, recordKeyColumns } from "./lineage";
import { querydownReady } from "./querydown";
import { keyConditions, type RecordKey } from "./recordForm";
import { parseSchemaTables } from "./schema";

/** A record a result row identifies: the table, plus the key values that row
 * carries for it. Structurally the store's `RecordRef` — the store passes those
 * straight in. */
export interface RowRecord {
  table: string;
  key: RecordKey;
}

/** Where a re-read row's values live: `table`'s row `row` — what
 * `QueryResult.patchRow` takes to re-point the display row at it. */
export interface RowLocation {
  table: Table;
  row: number;
}

/** The row a DML request is being made in the context of, and everything needed
 * to read it back afterwards.
 *
 * The query arrives as the **definition** the user built rather than as
 * assembled Querydown or compiled SQL: taking the filter section out and putting
 * another one in is only possible while the parts are still separate. */
export interface RowContext {
  definition: QueryDefinition;
  /** The presets the definition's section references resolve against. */
  presets: Preset[];
  /** The enriched introspection JSON the compiler takes. */
  schemaJson: string;
  /** The Querydown prelude in force (the `querydown_prelude` setting), so the
   * re-read compiles against exactly what the displayed rows came from. */
  prelude: string;
  /** Every record the row identifies — one per table whose key it carries (what
   * the store's `rowRecords` returns). The base table's entry narrows the query;
   * all of them together find the row again in a whole-query re-run. */
  records: readonly RowRecord[];
}

/** What a row-context DML request produced: the API's answer, and where each row
 * now reads from. */
export interface RowDmlOutcome {
  /** Per-operation returned rows, exactly as the API gave them. */
  result: DmlResult;
  /** Where each row now reads from, index-aligned to the contexts asked for.
   * An entry is `undefined` when that row couldn't be read back — a query that
   * can't be narrowed *and* whose re-run no longer carries the row, or a failure
   * along the way. The write still happened. */
  rows: readonly (RowLocation | undefined)[];
}

/** Runs `operations` as one DML request, then re-reads the rows it was made
 * against — one context per row, since a write made from a multi-row selection
 * leaves every one of those rows out of date.
 *
 * A refresh that fails is reported and swallowed: the write has already landed,
 * and losing the redisplay must not read to the caller as the save having
 * failed. Only the DML call itself throws.
 */
export async function runRowDml(
  operations: DmlOperation[],
  contexts: readonly RowContext[],
): Promise<RowDmlOutcome> {
  const result = await dml({ operations });
  const rows = await Promise.all(
    contexts.map(async (context) => {
      try {
        return await refreshRow(context);
      } catch (err) {
        console.error("result row refresh failed", err);
        return undefined;
      }
    }),
  );
  return { result, rows };
}

/** Re-reads one row: through the narrowed query when the definition can be
 * picked apart, else by finding it in a re-run of the query as it stands. */
async function refreshRow(
  context: RowContext,
): Promise<RowLocation | undefined> {
  await querydownReady();
  const narrowed = narrowedDefinition(context.definition, context.records);
  return narrowed === undefined
    ? await findRowInFullRun(context)
    : await fetchNarrowedRow(narrowed, context);
}

/** The query that re-reads just the record `records` identifies in `def`'s base
 * table: same base, same display, the filter replaced by that record's key, the
 * sort dropped.
 *
 * `undefined` when the definition is one hand-written query (`full` — there are
 * no sections to replace) or when the row identifies no record of the base table
 * (nothing to filter on). Either way the row can only be looked for in a re-run
 * of the query as it stands. */
export function narrowedDefinition(
  def: QueryDefinition,
  records: readonly RowRecord[],
): QueryDefinition | undefined {
  if (def.full != null) return undefined;
  const base = def.base.trim().toLowerCase();
  if (base === "") return undefined;
  const record = records.find((r) => r.table.trim().toLowerCase() === base);
  if (!record || record.key.length === 0) return undefined;
  return {
    base: def.base,
    filter: { custom: keyConditions([record.key]), presets: [] },
    sort: { custom: "" },
    display: def.display,
  };
}

/** Runs a narrowed query and reads the one row it returns. Anything but one row
 * means the key didn't pick out a single record after all — a query this can't
 * refresh, rather than a row to guess at. */
async function fetchNarrowedRow(
  def: QueryDefinition,
  context: RowContext,
): Promise<RowLocation | undefined> {
  const { sql } = compileSavedQuery(
    def,
    context.presets,
    context.schemaJson,
    context.prelude,
  );
  const table = await runSql(sql);
  if (table.numRows !== 1) return undefined;
  return { table, row: 0 };
}

/** Re-runs the query as it stands and finds the row again by the records it
 * identifies — every one of them, so that a row among near-duplicates (the same
 * album on every one of its tracks) is only claimed when the whole identity
 * matches. A row the query no longer returns, or one whose identity the write
 * itself changed, simply isn't found: nothing is spliced in and the row stays as
 * it was. */
async function findRowInFullRun(
  context: RowContext,
): Promise<RowLocation | undefined> {
  const { sql } = compileSavedQuery(
    context.definition,
    context.presets,
    context.schemaJson,
    context.prelude,
  );
  const table = await runSql(sql);
  // Which output column carries which key column: the same lineage question the
  // rows' affordances are built on, asked of this run's own SQL.
  const sources = await analyzeColumnSources(sql);
  if (!sources) return undefined;
  const targets = recordKeyColumns(
    sources,
    parseSchemaTables(context.schemaJson),
  );

  /** The output column / expected value pairs a row has to match on. */
  const probes: { column: number; value: string }[] = [];
  for (const record of context.records) {
    if (record.key.length === 0) continue;
    const target = targets.find((t) => t.table === record.table);
    if (!target) continue;
    const parts: { column: number; value: string }[] = [];
    for (const part of record.key) {
      const at = target.keyColumns.indexOf(part.column);
      if (at === -1) break;
      parts.push({ column: target.keyIndices[at], value: part.value });
    }
    // Half a composite key identifies nothing, so a record counts only when the
    // re-run carries every column of it.
    if (parts.length === record.key.length) probes.push(...parts);
  }
  if (probes.length === 0) return undefined;

  const matchers = probes.map((p) => ({
    vector: table.getChildAt(p.column),
    type: table.schema.fields[p.column]?.type,
    value: p.value,
  }));
  for (let row = 0; row < table.numRows; row++) {
    const matched = matchers.every((m) => {
      const value = m.vector?.get(row);
      return value != null && stringifyArrowValue(value, m.type) === m.value;
    });
    if (matched) return { table, row };
  }
  return undefined;
}
