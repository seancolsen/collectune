// What an embedded record shows, and the Querydown that fetches it.
//
// An embedded record is a preview of one row — the record behind a scalar linked
// record field, or each of the records under a multi-record field — and its whole
// job is to let the user recognize that record at a glance. Which columns do that
// is eventually going to be user-configurable; until then it's derived from the
// schema alone, by the points-based heuristic the spec lays out ("Loading data to
// render embedded records").
//
// The heuristic, in one paragraph: assemble the candidate result-column paths for
// the target table (its own non-foreign-key columns, plus one level of recursion
// through each foreign key that isn't the contextual filter), score each on four
// axes — hops, nullability, uniqueness, type — and display the (up to two)
// columns tied for the top score. The sort section reuses those columns, led by
// `order` when the table has one and trailed by the record's key so the order is
// stable across refreshes.
//
// Pure: schema in, Querydown out. The reactive side (when a preview is fetched,
// where its cells land) lives in `components/record/formModel.ts`.

import { identifyingColumns, inferLinks, type SchemaTable } from "./schema";
import {
  keyConditions,
  quoteValue,
  valueTypeOf,
  type MultiRecordField,
  type RecordKey,
  type RecordQuery,
} from "./recordForm";

/** The generated Querydown for previewing records of one table. */
export interface EmbedSpec {
  /** One display expression per preview column, e.g. `["$artist.name"]`.
   * Empty when the schema offers nothing informative — the caller falls back to
   * the record's key, which always identifies it, if drily. */
  display: readonly string[];
  /** The sorting section, e.g. `"\\order \\artist.name \\artist"`; empty when
   * there is nothing to sort by. */
  sort: string;
}

/** One candidate result column: where it is, what it scored, and whether it's
 * temporal (which sorts descending — most recent first). */
interface Candidate {
  /** The path from the target table, e.g. `"role"` or `"artist.name"`. */
  path: string;
  points: number;
  temporal: boolean;
}

/** The type axis: text-like columns read well in a preview, opaque UUIDs read
 * badly. `valueTypeOf` classifies a user-defined ENUM as "other" (introspection
 * reports it under the enum's own name), so DuckDB's inline `ENUM(...)` spelling
 * is caught here separately. */
function typePoints(type: string | undefined): number {
  if ((type ?? "").trim().toUpperCase().startsWith("ENUM")) return 1;
  switch (valueTypeOf(type)) {
    case "text":
      return 1;
    case "uuid":
      return -1;
    default:
      return 0;
  }
}

function isTemporal(type: string | undefined): boolean {
  return valueTypeOf(type) === "timestamp";
}

/** Whether a column counts as unique in the query's context: it has a
 * single-column unique constraint, or it shares a two-column one with the
 * contextual filter column — `credit (track, "order")` makes `order` unique *within*
 * one track, which is exactly the context a preview of that track's credits is
 * shown in. */
function isUnique(
  table: SchemaTable,
  column: string,
  contextColumn: string | undefined,
): boolean {
  return table.uniqueConstraints.some((constraint) => {
    if (constraint.length === 1) return constraint[0] === column;
    if (constraint.length !== 2) return false;
    return (
      constraint.includes(column) &&
      contextColumn !== undefined &&
      constraint.includes(contextColumn)
    );
  });
}

/** Scores one column on the spec's four axes. `direct` is whether it lives in
 * the target table itself rather than being reached through a foreign key;
 * `contextColumn` only ever names a column of the target table, so it plays no
 * part in scoring a transitive one. */
function score(
  table: SchemaTable,
  columnName: string,
  direct: boolean,
  contextColumn: string | undefined,
): number {
  const column = table.columns.find((c) => c.name === columnName);
  if (!column) return 0;
  return (
    (direct ? 1 : 0) +
    (column.nullable ? 0 : 1) +
    (isUnique(table, columnName, contextColumn) ? 1 : 0) +
    typePoints(column.type)
  );
}

/** The candidate columns for a preview of `table`: its own columns, minus the
 * foreign keys (never shown as the raw id they hold) and minus the contextual
 * filter (the same value on every row it appears beside), plus one level of
 * recursion through each remaining foreign key. */
function assembleCandidates(
  tables: readonly SchemaTable[],
  table: SchemaTable,
  contextColumn: string | undefined,
): Candidate[] {
  const links = inferLinks(tables);
  const byName = new Map(tables.map((t) => [t.name, t]));
  const candidates: Candidate[] = [];

  for (const column of table.columns) {
    if (column.name === contextColumn) continue;
    const link = links.find(
      (l) => l.fromTable === table.name && l.fromColumn === column.name,
    );
    if (link) {
      const target = byName.get(link.toTable);
      if (!target) continue;
      for (const sub of target.columns) {
        candidates.push({
          path: `${column.name}.${sub.name}`,
          points: score(target, sub.name, false, undefined),
          temporal: isTemporal(sub.type),
        });
      }
    } else {
      candidates.push({
        path: column.name,
        points: score(table, column.name, true, contextColumn),
        temporal: isTemporal(column.type),
      });
    }
  }
  return candidates;
}

/** The columns a preview displays: those tied for the top score, capped at the
 * first two in assembly order. */
function pickDisplay(candidates: readonly Candidate[]): Candidate[] {
  if (candidates.length === 0) return [];
  const max = Math.max(...candidates.map((c) => c.points));
  return candidates.filter((c) => c.points === max).slice(0, 2);
}

/** The sorting section: `order` first when the table has such a column, then the
 * display columns (temporal ones descending, so the most recent is on top), then
 * the record's identifying key for an order that holds still across refreshes.
 * The contextual filter column is skipped throughout — it's constant over the
 * whole result set. */
function buildSort(
  table: SchemaTable,
  chosen: readonly Candidate[],
  contextColumn: string | undefined,
): string {
  const terms: { path: string; descending: boolean }[] = [];
  const push = (path: string, descending: boolean) => {
    if (path === contextColumn) return;
    if (terms.some((t) => t.path === path)) return;
    terms.push({ path, descending });
  };

  if (table.columns.some((c) => c.name === "order")) push("order", false);
  for (const candidate of chosen) push(candidate.path, candidate.temporal);
  for (const column of identifyingColumns(table)) push(column, false);

  // Two literal backslashes mark a sort term; a trailing `\d` makes it
  // descending (both verified against the vendored compiler).
  return terms
    .map((t) => `\\\\${t.path}${t.descending ? " \\d" : ""}`)
    .join(" ");
}

/** The display and sort sections for a preview of records from `tableName`.
 *
 * `contextColumn` names the column the query already pins to one value — the
 * referencing column of a multi-record field (`credit.track`, when listing one
 * track's credits). It is left out of the display and the sort, and it makes a
 * column it shares a two-column unique constraint with count as unique.
 *
 * An unknown table (or one with nothing worth showing) yields an empty spec; the
 * caller falls back to the record's key. */
export function embedSpec(
  tables: readonly SchemaTable[],
  tableName: string,
  contextColumn?: string,
): EmbedSpec {
  const table = tables.find((t) => t.name === tableName);
  if (!table) return { display: [], sort: "" };
  const chosen = pickDisplay(assembleCandidates(tables, table, contextColumn));
  if (chosen.length === 0) return { display: [], sort: "" };
  return {
    display: chosen.map((c) => `$${c.path}`),
    sort: buildSort(table, chosen, contextColumn),
  };
}

/** The query behind one embedded record: the preview columns of the single
 * record `key` identifies. Used for a scalar linked record field, whose id
 * arrives with the parent record's own data.
 *
 * The result's columns are positional — one per entry of `spec.display`. */
export function embeddedRecordQuery(
  table: string,
  key: RecordKey,
  spec: EmbedSpec,
): RecordQuery {
  return {
    base: table,
    filter: keyConditions([key]),
    sort: "",
    display: spec.display.join(" "),
  };
}

/** The query behind the modal record picker: the records of `table` the user's
 * filter admits, previewed the same way the form's embedded records are.
 *
 * `filter`, `sort` and `display` are the *live* sections of the picker — the
 * search box the user types into and the sort/display builders beside it, which
 * open pre-filled from {@link embedSpec} — rather than being derived here.
 *
 * The result's columns are positional: `keyColumns` first (how the picked record
 * is addressed once it's chosen), then whatever `display` asks for, which is
 * handed to the embedded record widget as it stands so choosing a record needs
 * no further request. */
export function recordPickerQuery(
  table: string,
  keyColumns: readonly string[],
  filter: string,
  sort: string,
  display: string,
): RecordQuery {
  const keys = keyColumns.map((c) => `$${c}`);
  return {
    base: table,
    filter,
    sort,
    display: [...keys, display.trim()].filter(Boolean).join(" "),
  };
}

/** The query behind a multi-record field: every record of `field.table` whose
 * `field.column` points at the record `parent` identifies, in one request —
 * both the identifying columns (which is how each child record is addressed
 * afterwards) and the preview columns its embedded record shows.
 *
 * The result's columns are positional: `field.keyColumns` first, then
 * `spec.display`. With no preview columns to be had, the keys stand in for them
 * and order the list themselves. */
export function childRecordsQuery(
  field: MultiRecordField,
  parent: string,
  spec: EmbedSpec,
): RecordQuery {
  const keys = field.keyColumns.map((c) => `$${c}`);
  return {
    base: field.table,
    filter: `${field.column}:=${quoteValue(parent)}`,
    sort: spec.sort || field.keyColumns.map((c) => `\\\\${c}`).join(" "),
    display: [...keys, ...spec.display].join(" "),
  };
}
