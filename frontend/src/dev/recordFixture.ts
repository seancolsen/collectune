// A canned database for the record editor, so the form can be driven — and
// snapshotted — without a backend. Two halves, matching the two things the form
// needs from one: an introspection document (its whole structure comes from
// there) and answers to the Querydown queries it builds (its data).
//
// The schema is RadioCrate's own, as the migrations leave it
// (`backend/src/migrations/`), trimmed to the tables and columns the form has
// something to show for. The rows continue the seeded "Lemonade" grid:
// `?records=track,album` gives its rows the keys `track-1` … `track-5`, which is
// what the ids below are.
//
// The fake query runner interprets only the shapes `query/recordForm.ts` and
// `query/embeddedRecord.ts` generate — `col:="value"` conditions, `[{…} {…}]`
// alternatives, `$col` / `$link.col` / `$#table` display expressions, `\\col` /
// `\\col \d` sorting — rather than being a SQL engine. If the generators learn a
// new shape, this learns it too.

import { addInferredLinks } from "../query/schema";
import { setRecordQueryRunner } from "../query/recordData";
import type { RecordQuery } from "../query/recordForm";

const T = (
  name: string,
  columns: [string, string, boolean?][],
  uniques: string[][],
) => ({
  name,
  columns: columns.map(([n, type, nullable]) => ({
    name: n,
    type,
    nullable: nullable ?? false,
  })),
  unique_constraints: uniques,
});

/** The enriched introspection JSON — RadioCrate's schema, with the links the
 * compiler and the form both read inferred into it. */
export const FIXTURE_SCHEMA_JSON = addInferredLinks(
  JSON.stringify({
    tables: [
      T(
        "album",
        [
          ["id", "UUID"],
          ["title", "VARCHAR", true],
          ["year", "USMALLINT", true],
        ],
        [["id"]],
      ),
      T(
        "artist",
        [
          ["id", "UUID"],
          ["name", "VARCHAR"],
        ],
        [["id"], ["name"]],
      ),
      T(
        "credit",
        [
          ["track", "UUID"],
          ["artist", "UUID"],
          ["order", "FLOAT", true],
          // Dropped and re-added as a link by migration 0003, which is why it
          // trails the columns it used to sit among.
          ["role", "UUID", true],
        ],
        [["track", "artist"]],
      ),
      T(
        "file",
        [
          ["id", "UUID"],
          ["path", "VARCHAR"],
          ["size", "UINTEGER"],
          ["format", "format"],
          ["duration", "INTERVAL"],
          ["added", "TIMESTAMP"],
        ],
        [["id"], ["path"]],
      ),
      T(
        "play",
        [
          ["track", "UUID"],
          ["timestamp", "TIMESTAMP_S"],
        ],
        [["track", "timestamp"]],
      ),
      T(
        "rating",
        [
          ["id", "UUID"],
          ["value", "FLOAT"],
        ],
        [["id"], ["value"]],
      ),
      T(
        "role",
        [
          ["id", "UUID"],
          ["name", "VARCHAR"],
        ],
        [["id"], ["name"]],
      ),
      T(
        "tag",
        [
          ["id", "UUID"],
          ["name", "VARCHAR"],
        ],
        [["id"], ["name"]],
      ),
      T(
        "track",
        [
          ["id", "UUID"],
          ["file", "UUID"],
          ["title", "VARCHAR"],
          ["album", "UUID", true],
          ["disc_number", "UTINYINT", true],
          ["track_number", "UTINYINT", true],
          ["rating", "UUID", true],
        ],
        [["id"]],
      ),
      T(
        "track_tag",
        [
          ["track", "UUID"],
          ["tag", "UUID"],
        ],
        [["track", "tag"]],
      ),
    ],
    links: [],
  }),
);

/** One fixture row: column → value, `null` for NULL. */
type Row = Record<string, string | null>;

const TITLES = [
  "Pray You Catch Me",
  "Hold Up",
  "Don't Hurt Yourself",
  "Sorry",
  "6 Inch",
];

/** A title long enough to overflow the sidebar's width — and carrying a
 * linebreak, which makes a value expandable whatever its width — so the
 * expandable-text behavior has something to expand.
 *
 * It lives on `title` because that is the only free text `track` has left: the
 * long value used to be `genre`, which migration 0003 replaced with the `tag`
 * table. A track whose title runs this long is not a contrivance — a live
 * recording that names its medley is exactly this shape. */
const LONG_TITLE =
  'Don\'t Hurt Yourself — Live at the Superdome, with the full horn\nsection, running into a reprise of "Ring the Alarm" nobody had rehearsed.';

const TABLE_ROWS: Record<string, Row[]> = {
  // One album per grid row, so a row's `album-N` key (from `?records=`) resolves
  // to a record whichever row the editor is opened on.
  album: TITLES.map((_, i) => ({
    id: `album-${i + 1}`,
    title: "Lemonade",
    year: "2016",
  })),
  artist: [
    { id: "artist-1", name: "Beyoncé" },
    { id: "artist-2", name: "Jack White" },
    { id: "artist-3", name: "The Weeknd" },
  ],
  file: TITLES.map((title, i) => ({
    id: `file-${i + 1}`,
    path: `./Beyoncé/Lemonade/Beyoncé - ${i + 1} - ${title}.flac`,
    size: `${34_000_000 + i * 1_000_000}`,
    format: "flac",
    duration: ["00:03:16", "00:03:41", "00:03:54", "00:03:53", "00:04:20"][i],
    added: "2016-04-23 19:04:00",
  })),
  // Ratings and roles are shared values as of migration 0003: a handful of
  // records the tracks and credits point at, rather than a number and a string
  // repeated down each column.
  rating: [
    { id: "rating-35", value: "3.5" },
    { id: "rating-4", value: "4" },
    { id: "rating-45", value: "4.5" },
  ],
  role: [{ id: "role-featured", name: "Featured" }],
  tag: [
    { id: "tag-rnb", name: "R&B" },
    { id: "tag-neo-soul", name: "Neo soul" },
    { id: "tag-rock", name: "Rock" },
    { id: "tag-trap-soul", name: "Trap soul" },
  ],
  track: TITLES.map((title, i) => ({
    id: `track-${i + 1}`,
    file: `file-${i + 1}`,
    // The third track's title is the long one, so the expandable-text behavior
    // is reachable on a record that also has credits, plays and tags to show.
    title: i === 2 ? LONG_TITLE : title,
    // The last track is left unfiled, so a NULL scalar linked record field (and
    // its pencil button) is on screen.
    album: i === 4 ? null : `album-${i + 1}`,
    disc_number: "1",
    // The last track's number is left off, so a NULL *primitive* field — whose
    // pencil activates a text box, unlike a link's, which opens the record
    // picker — is on screen beside that NULL `album` link.
    track_number: i === 4 ? null : `${i + 1}`,
    rating: ["rating-35", "rating-4", "rating-4", "rating-4", "rating-45"][i],
  })),
  // Several tags on one track — the thing the old comma-joined `genre` string
  // could not represent, and the reason `track_tag` exists.
  track_tag: [
    { track: "track-1", tag: "tag-rnb" },
    { track: "track-1", tag: "tag-neo-soul" },
    { track: "track-3", tag: "tag-rnb" },
    { track: "track-3", tag: "tag-rock" },
    { track: "track-5", tag: "tag-trap-soul" },
  ],
  credit: [
    { track: "track-1", artist: "artist-1", order: "1", role: null },
    { track: "track-3", artist: "artist-1", order: "1", role: null },
    { track: "track-3", artist: "artist-2", order: "2", role: "role-featured" },
    { track: "track-5", artist: "artist-1", order: "1", role: null },
    { track: "track-5", artist: "artist-3", order: "2", role: "role-featured" },
  ],
  play: [
    { track: "track-1", timestamp: "2016-04-24 08:12:00" },
    { track: "track-1", timestamp: "2016-05-02 21:47:00" },
    { track: "track-3", timestamp: "2016-06-11 10:05:00" },
  ],
};

/** One parsed filter line: a column, a value, and whether the match is exact
 * (`:=`) or a substring (`:`). */
interface Condition {
  column: string;
  value: string;
  exact: boolean;
}

/** One condition per line: `col:="value"` as `keyConditions` writes them, and
 * the looser `col:value` / `col:"value"` substring form a user types into the
 * record picker's search box. */
function parseConditions(filter: string): Condition[] {
  const conditions: Condition[] = [];
  for (const line of filter.split("\n")) {
    const match = /^(\w+):(=?)\s*(?:"((?:[^"\\]|\\.)*)"|(\S+))$/.exec(
      line.trim(),
    );
    if (!match) continue;
    conditions.push({
      column: match[1],
      value: (match[3] ?? match[4]).replace(/\\(.)/g, "$1"),
      exact: match[2] === "=",
    });
  }
  return conditions;
}

/** A filter as alternatives, each a set of conditions AND-ed together: one
 * group for the ordinary filter, and one per `{…}` when `keyConditions` writes
 * out several records' keys inside `[…]`. A row satisfies the filter when it
 * satisfies any group. */
function parseFilter(filter: string): Condition[][] {
  if (!filter.trim().startsWith("[")) return [parseConditions(filter)];
  const groups = [...filter.matchAll(/\{([^}]*)\}/g)].map((m) =>
    parseConditions(m[1]),
  );
  // `[…]` around bare conditions (rather than `{…}` groups) is one alternative
  // per condition — the shape a single-column key takes.
  return groups.length > 0
    ? groups
    : parseConditions(filter).map((condition) => [condition]);
}

/** Whether one row satisfies one condition. */
function matches(row: Row, condition: Condition): boolean {
  const cell = row[condition.column];
  if (condition.exact) return cell === condition.value;
  return (cell ?? "").toLowerCase().includes(condition.value.toLowerCase());
}

/** How many rows of `childTable` point at `row` — the `$#table` count. */
function relatedCount(childTable: string, base: string, row: Row): number {
  return (TABLE_ROWS[childTable] ?? []).filter((r) => r[base] === row.id)
    .length;
}

/** Reads a column path out of a row: `title`, or `artist.name` — one hop
 * through the foreign key named by the first part, which the fixture resolves
 * the way the compiler's inferred links do (`<column>` → `<column>.id`). */
function readPath(base: string, row: Row, path: string): string | null {
  const [first, second] = path.split(".");
  if (second === undefined) return row[first] ?? null;
  const id = row[first];
  const target = (TABLE_ROWS[first] ?? []).find((r) => r.id === id);
  return target ? (target[second] ?? null) : null;
}

/** One sort term: a column path and its direction (`\d` makes it descending). */
function parseSort(sort: string): { path: string; descending: boolean }[] {
  const terms: { path: string; descending: boolean }[] = [];
  for (const token of sort.split(/\s+/).filter(Boolean)) {
    if (token === "\\d") {
      if (terms.length > 0) terms[terms.length - 1].descending = true;
      continue;
    }
    terms.push({ path: token.replace(/^\\+/, ""), descending: false });
  }
  return terms;
}

/** Answers one record editor query out of the fixture rows — a mock of the
 * query API a story can hand a component directly, with no runner install. */
export function fixtureQuery(q: RecordQuery): (string | null)[][] {
  const groups = parseFilter(q.filter);
  const rows = (TABLE_ROWS[q.base] ?? []).filter((row) =>
    groups.some((conditions) =>
      conditions.every((condition) => matches(row, condition)),
    ),
  );
  const terms = parseSort(q.sort);
  rows.sort((a, b) => {
    for (const { path, descending } of terms) {
      const order = (readPath(q.base, a, path) ?? "").localeCompare(
        readPath(q.base, b, path) ?? "",
      );
      if (order !== 0) return descending ? -order : order;
    }
    return 0;
  });
  const exprs = q.display.split(/\s+/).filter(Boolean);
  return rows.map((row) =>
    exprs.map((expr) =>
      expr.startsWith("$#")
        ? String(relatedCount(expr.slice(2), q.base, row))
        : readPath(q.base, row, expr.slice(1)),
    ),
  );
}

/** Points the record editor at the fixture instead of the backend. `delayMs`
 * holds every answer back by that long, so the loading states (the wash over the
 * form, the placeholder rows under a multi-record field) can be seen and
 * asserted. */
export function installRecordFixture(delayMs: number): void {
  setRecordQueryRunner(
    (q) =>
      new Promise((resolve) =>
        setTimeout(() => resolve(fixtureQuery(q)), delayMs),
      ),
  );
}
