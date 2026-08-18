// The SQL runner: POST the raw SQL string to `/api/query`, buffer the whole
// Arrow IPC response (no streaming), and decode it. The backend runs SQL and
// streams Arrow, full stop — all Querydown→SQL work happens upstream (see plan
// §4). Cell extraction (list detection + stringification) lives here too; the
// structured result is assembled in `query/result.ts`.

import {
  DataType,
  IntervalUnit,
  tableFromIPC,
  type DataType as ArrowType,
  type Vector,
} from "apache-arrow";
import { postQuery } from "api-client";

/** Runs a SQL string against the query API and returns the decoded Arrow table.
 * The transport lives in the generated client (`postQuery`); Arrow decoding
 * stays here. Throws on a non-2xx response (a bad SQL string returns 400 + a
 * plain-text DuckDB error). */
export async function runSql(sql: string) {
  return tableFromIPC(new Uint8Array(await postQuery(sql)));
}

/** Reads the single JSON text cell from a one-row/one-column result — how the
 * introspection SQL returns the schema document. */
export async function runSqlScalar(sql: string): Promise<string> {
  const table = await runSql(sql);
  const first = table.get(0);
  if (!first) throw new Error("Query returned no rows.");
  const value = first[table.schema.fields[0].name];
  return String(value);
}

/** Whether an Arrow column type is a list (renders as pills). Mirrors
 * `ResultRows::is_list_column`.
 *
 * NOTE: apache-arrow JS (18.x) has no public `LargeList` support and its
 * `Type` enum omits it, yet DuckDB emits list columns that decode with a typeId
 * the public `DataType.isList` guard misses. So this is only a *fast path*; when
 * it returns false the caller must still fall back to {@link isListLikeValue}. */
export function isListType(type: ArrowType): boolean {
  return DataType.isList(type);
}

/** Whether `type` is the interval flavor DuckDB sends — the one apache-arrow
 * cannot decode (see {@link readMonthDayNanoInterval}). */
export function isMonthDayNanoInterval(type: ArrowType | undefined): boolean {
  return (
    type !== undefined &&
    DataType.isInterval(type) &&
    type.unit === IntervalUnit.MONTH_DAY_NANO
  );
}

/** Bytes per `Interval(MONTH_DAY_NANO)` value: int32 months, int32 days, int64
 * nanoseconds, little-endian. */
const INTERVAL_BYTES = 16;

const NS_PER_SECOND = 1_000_000_000n;

/** Reads one `Interval(MONTH_DAY_NANO)` cell straight out of the column's
 * buffer, as DuckDB's own interval text (`"00:03:42.25"`). `null` for a NULL
 * cell.
 *
 * NOTE: another apache-arrow (18.x) gap, in the same spirit as {@link isListType}
 * above, but a silent one. MONTH_DAY_NANO is the only interval flavor DuckDB
 * emits and the only one apache-arrow's getter has no branch for: it falls
 * through to the YEAR_MONTH branch, which reads a *single* int32 where the value
 * is 16 bytes wide. Nothing throws — a 3m42.25s duration just decodes to the
 * `Int32Array` `0,0`, and longer ones to arbitrary garbage. The IPC reader does
 * store the bytes faithfully, so reading the documented layout here recovers the
 * value.
 *
 * Indexes `values` from zero the way every apache-arrow getter does (`Data`
 * slicing rebases the buffer, so `data.offset` is not applied on read). */
export function readMonthDayNanoInterval(
  vector: Vector,
  row: number,
): string | null {
  if (!vector.isValid(row)) return null;
  let index = row;
  for (const data of vector.data) {
    if (index >= data.length) {
      index -= data.length;
      continue;
    }
    const values = data.values as unknown as ArrayBufferView;
    const at = INTERVAL_BYTES * index;
    if (at + INTERVAL_BYTES > values.byteLength) return null;
    const view = new DataView(
      values.buffer,
      values.byteOffset,
      values.byteLength,
    );
    return formatInterval(
      view.getInt32(at, true),
      view.getInt32(at + 4, true),
      view.getBigInt64(at + 8, true),
    );
  }
  return null;
}

/** Renders an interval's three components the way DuckDB prints them: the month
 * and day parts only when they carry something, then `HH:MM:SS[.fff]` for the
 * time. A pure duration — every interval this app stores — is just the time
 * part, which is what the `duration` formatter then reads. */
function formatInterval(months: number, days: number, nanos: bigint): string {
  const parts: string[] = [];
  if (months !== 0) parts.push(`${months} month${months === 1 ? "" : "s"}`);
  if (days !== 0) parts.push(`${days} day${days === 1 ? "" : "s"}`);
  if (nanos !== 0n || parts.length === 0) parts.push(formatIntervalTime(nanos));
  return parts.join(" ");
}

/** `HH:MM:SS[.fff]` for a nanosecond count, with the fraction trimmed of
 * trailing zeros and hours allowed past 24 (an interval is a span, not a clock
 * reading). */
function formatIntervalTime(nanos: bigint): string {
  const sign = nanos < 0n ? "-" : "";
  const abs = nanos < 0n ? -nanos : nanos;
  const seconds = abs / NS_PER_SECOND;
  const pad = (n: bigint) => String(n).padStart(2, "0");
  const hms = `${pad(seconds / 3600n)}:${pad((seconds / 60n) % 60n)}:${pad(seconds % 60n)}`;
  const fraction = String(abs % NS_PER_SECOND)
    .padStart(9, "0")
    .replace(/0+$/, "");
  return `${sign}${hms}${fraction === "" ? "" : `.${fraction}`}`;
}

/** Whether a decoded cell value is list-like (an Arrow sub-vector / array), used
 * as the value-level fallback when {@link isListType} can't classify a column
 * (see its note). Strings are iterable too, so they're excluded; scalars, dates,
 * and struct rows aren't iterable. */
export function isListLikeValue(v: unknown): boolean {
  return (
    v != null &&
    typeof v !== "string" &&
    typeof (v as { [Symbol.iterator]?: unknown })[Symbol.iterator] ===
      "function"
  );
}

/** Stringifies one scalar Arrow cell value (null/undefined → ""). Mirrors
 * `format_cell` / Arrow's `ArrayFormatter` for the common types.
 *
 * `type` is the column's Arrow field type, when known — required to render
 * `Timestamp*` cells correctly (see {@link formatNaiveTimestamp}); every other
 * type falls through to the crude `String(value)` default (dates and decimals
 * should still be verified against a real backend, per plan §3.4).
 *
 * DEVIATION: Arrow's JS `.get()` already returns display-ready primitives for
 * strings/numbers/bools. */
export function stringifyArrowValue(value: unknown, type?: ArrowType): string {
  if (value == null) return "";
  if (type && DataType.isTimestamp(type) && typeof value === "number") {
    return formatNaiveTimestamp(value);
  }
  return String(value);
}

/** Formats an Arrow `Timestamp*` cell as a naive `YYYY-MM-DD HH:MM:SS` string.
 *
 * apache-arrow's `.get()` normalizes every `Timestamp*` unit (s/ms/us/ns) to
 * milliseconds since the epoch, so `ms` here is always milliseconds regardless
 * of the column's stored resolution (`timestamp_s` included). DuckDB's `TIMESTAMP`
 * columns are timezone-naive: it computes that epoch by treating the stored
 * civil fields as UTC (no real timezone conversion), so reading them back with
 * the UTC getters recovers exactly the civil fields DuckDB has — the string this
 * returns round-trips through a DuckDB text-to-timestamp cast unchanged. */
function formatNaiveTimestamp(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ` +
    `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())}`
  );
}
