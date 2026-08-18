// Per-column value formatters.
//
// A column's metadata may carry a `formatter` blob selecting a typed transform
// that reshapes each cell's display text. Cell values arrive as plain strings
// (stringified Arrow values), so every formatter parses the string it is given
// and returns `null` when it can't — the caller then falls back to the raw value.
//
// DEVIATION FROM RUST: the reference uses the `jiff` crate for `Timestamp` /
// `RelativeTime`. Per the plan (§3.3) we add no date library and use the built-in
// `Date` plus hand-rolled elapsed math. Consequences:
//   - `Timestamp` supports a reduced `strftime` (the specifiers RadioCrate's
//     queries actually use: %Y %m %d %H %M %S %y %j %% and lowercase am/pm-free),
//     not jiff's full grammar.
//   - `RelativeTime` months/years use average lengths (30.44 / 365.25 days)
//     rather than jiff's calendar-aware spans, so long spans can differ by a day
//     near month boundaries. `whole_app.png` only needs number/duration fidelity.

import type { AnnotationValue } from "querydown-js";
import type { ColumnMetadata } from "./columns";

/** A unit available to the `relativeTime` formatter. */
export type RelativeUnit =
  "minutes" | "hours" | "days" | "weeks" | "months" | "years";

/** The user-facing `decimalPlaces` spec: a plain number, or a one/two-element
 * list of `[min]` / `[min, max]` fraction digits. */
export type DecimalPlaces = number | number[];

/** A typed value formatter, parsed from a column's `formatter` metadata blob.
 * The `type` discriminant selects the variant; each carries its own config. */
export type Formatter =
  | { type: "number"; decimalPlaces: DecimalPlaces }
  | { type: "timestamp"; format: string }
  | { type: "duration" }
  | { type: "relativeTime"; units: RelativeUnit[] };

const RELATIVE_UNITS: RelativeUnit[] = [
  "minutes",
  "hours",
  "days",
  "weeks",
  "months",
  "years",
];

/** Parses a `formatter` metadata blob into a {@link Formatter}, or `undefined`
 * when it's malformed or of an unknown type. Mirrors serde's `tag = "type",
 * rename_all = "camelCase"`: `Number.decimalPlaces` and `RelativeTime.units`
 * default when absent; `Timestamp.format` is required. */
export function parseFormatter(
  blob: AnnotationValue | undefined,
): Formatter | undefined {
  if (blob == null || typeof blob !== "object" || Array.isArray(blob)) {
    return undefined;
  }
  const o = blob as Record<string, AnnotationValue>;
  switch (o.type) {
    case "number":
      return {
        type: "number",
        decimalPlaces: parseDecimalPlaces(o.decimalPlaces),
      };
    case "duration":
      return { type: "duration" };
    case "timestamp":
      if (typeof o.format !== "string") return undefined;
      return { type: "timestamp", format: o.format };
    case "relativeTime":
      return { type: "relativeTime", units: parseUnits(o.units) };
    default:
      return undefined;
  }
}

function parseDecimalPlaces(v: AnnotationValue | undefined): DecimalPlaces {
  if (typeof v === "number") return v;
  if (Array.isArray(v))
    return v.filter((x): x is number => typeof x === "number");
  return [];
}

function parseUnits(v: AnnotationValue | undefined): RelativeUnit[] {
  if (!Array.isArray(v)) return [];
  return v.filter((x): x is RelativeUnit =>
    RELATIVE_UNITS.includes(x as RelativeUnit),
  );
}

/** Formats `value` for display, or returns `null` when it can't be parsed (the
 * caller then renders the raw value unchanged). */
export function formatValue(f: Formatter, value: string): string | null {
  switch (f.type) {
    case "number": {
      const [min, max] = resolveDecimals(f.decimalPlaces);
      return formatNumber(value, min, max);
    }
    case "duration":
      return formatDuration(value);
    case "timestamp":
      return formatTimestamp(value, f.format);
    case "relativeTime":
      return formatRelative(value, f.units, Date.now());
  }
}

/** Applies the column's formatter and prefix/suffix to one raw cell value. */
export function displayText(meta: ColumnMetadata, value: string): string {
  const formatted = meta.formatter
    ? (formatValue(meta.formatter, value) ?? value)
    : value;
  if (meta.prefix === "" && meta.suffix === "") return formatted;
  return `${meta.prefix}${formatted}${meta.suffix}`;
}

/** Resolves a {@link DecimalPlaces} spec into a `[min, max]` pair, swapping if
 * min > max. */
function resolveDecimals(spec: DecimalPlaces): [number, number] {
  let min: number;
  let max: number;
  if (typeof spec === "number") {
    min = spec;
    max = spec;
  } else if (spec.length === 0) {
    min = 0;
    max = 0;
  } else if (spec.length === 1) {
    min = spec[0];
    max = spec[0];
  } else {
    min = spec[0];
    max = spec[1];
  }
  if (min > max) [min, max] = [max, min];
  return [min, max];
}

/** Formats a number to at most `max` and at least `min` fraction digits, trimming
 * trailing zeros in between. No thousands grouping. */
function formatNumber(value: string, min: number, max: number): string | null {
  const trimmed = value.trim();
  // `Number("")` is 0, not NaN — guard the empty string like Rust's parse.
  if (trimmed === "") return null;
  const n = Number(trimmed);
  if (Number.isNaN(n)) return null;
  let s = n.toFixed(max);
  // `s` now has exactly `max` fraction digits. Trim trailing zeros down to `min`.
  if (max > min) {
    const dot = s.indexOf(".");
    if (dot !== -1) {
      const minLen = min === 0 ? dot : dot + 1 + min;
      while (s.length > minLen && s.endsWith("0")) s = s.slice(0, -1);
      if (s.endsWith(".")) s = s.slice(0, -1);
    }
  }
  return s;
}

/** An interval cell as `api/query.ts` renders it: an optional day part (months
 * are not a fixed number of seconds, so an interval carrying one is not a
 * duration), then `HH:MM:SS[.fff]`. */
const INTERVAL_RE = /^(?:(-?\d+) days? )?(-?)(\d+):(\d{2}):(\d{2}(?:\.\d+)?)$/;

/** Seconds in a duration cell, which reaches here in either of two forms: the
 * bare number a `REAL`/`DOUBLE` column gives, or the `HH:MM:SS` text an
 * `INTERVAL` column does (`file.duration` and the `track.*_position` columns are
 * intervals — see migration 0003). `null` when it is neither. */
function durationSeconds(value: string): number | null {
  const numeric = Number(value);
  if (!Number.isNaN(numeric)) return numeric;

  const m = INTERVAL_RE.exec(value);
  if (!m) return null;
  // The day and time parts carry their own signs, as DuckDB prints them
  // (`2 days -00:01:30`).
  const time = Number(m[3]) * 3600 + Number(m[4]) * 60 + Number(m[5]);
  return Number(m[1] ?? 0) * 86_400 + (m[2] === "-" ? -time : time);
}

/** Formats a duration as `M:SS` (minutes unpadded, seconds zero-padded),
 * rounded to the nearest second. */
function formatDuration(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed === "") return null;
  const secs = durationSeconds(trimmed);
  if (secs === null) return null;
  const total = Math.round(secs);
  const sign = total < 0 ? "-" : "";
  const abs = Math.abs(total);
  const minutes = Math.floor(abs / 60);
  const seconds = abs % 60;
  return `${sign}${minutes}:${String(seconds).padStart(2, "0")}`;
}

/** Civil datetime fields parsed from a cell value. */
interface Civil {
  year: number;
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
}

const DATETIME_RE = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2}):(\d{2})/;
const DATE_RE = /^(\d{4})-(\d{2})-(\d{2})$/;

/** Parses a cell value into civil datetime fields. Tries a civil datetime, then a
 * date, then integer epoch seconds (converted in local time). Mirrors the
 * fallback chain of `format_timestamp` / `parse_zoned`. */
function parseCivil(value: string): Civil | null {
  const v = value.trim();
  const dt = DATETIME_RE.exec(v);
  if (dt) {
    return {
      year: +dt[1],
      month: +dt[2],
      day: +dt[3],
      hour: +dt[4],
      minute: +dt[5],
      second: +dt[6],
    };
  }
  const d = DATE_RE.exec(v);
  if (d) {
    return {
      year: +d[1],
      month: +d[2],
      day: +d[3],
      hour: 0,
      minute: 0,
      second: 0,
    };
  }
  if (/^-?\d+$/.test(v)) {
    const date = new Date(Number(v) * 1000);
    if (!Number.isNaN(date.getTime())) {
      return {
        year: date.getFullYear(),
        month: date.getMonth() + 1,
        day: date.getDate(),
        hour: date.getHours(),
        minute: date.getMinutes(),
        second: date.getSeconds(),
      };
    }
  }
  return null;
}

/** Parses a cell value into epoch milliseconds (local time for civil values), for
 * relative-time math. */
function parseEpochMs(value: string): number | null {
  const c = parseCivil(value);
  if (!c) return null;
  const date = new Date(c.year, c.month - 1, c.day, c.hour, c.minute, c.second);
  const ms = date.getTime();
  return Number.isNaN(ms) ? null : ms;
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** A reduced `strftime`: supports the specifiers RadioCrate's queries use. */
function strftime(c: Civil, format: string): string {
  return format.replace(/%(.)/g, (_m, spec: string) => {
    switch (spec) {
      case "Y":
        return String(c.year);
      case "y":
        return pad2(c.year % 100);
      case "m":
        return pad2(c.month);
      case "d":
        return pad2(c.day);
      case "H":
        return pad2(c.hour);
      case "M":
        return pad2(c.minute);
      case "S":
        return pad2(c.second);
      case "%":
        return "%";
      default:
        return `%${spec}`;
    }
  });
}

function formatTimestamp(value: string, format: string): string | null {
  const c = parseCivil(value);
  if (!c) return null;
  return strftime(c, format);
}

const MS: Record<RelativeUnit, number> = {
  minutes: 60_000,
  hours: 3_600_000,
  days: 86_400_000,
  weeks: 604_800_000,
  months: 2_629_800_000, // 30.44 days (average — see file-level deviation note)
  years: 31_557_600_000, // 365.25 days
};

const RANK: Record<RelativeUnit, number> = {
  minutes: 0,
  hours: 1,
  days: 2,
  weeks: 3,
  months: 4,
  years: 5,
};

const NOUN: Record<RelativeUnit, string> = {
  minutes: "minute",
  hours: "hour",
  days: "day",
  weeks: "week",
  months: "month",
  years: "year",
};

function formatRelative(
  value: string,
  units: RelativeUnit[],
  nowMs: number,
): string | null {
  const thenMs = parseEpochMs(value);
  if (thenMs === null) return null;
  return relativeBetween(thenMs, nowMs, units);
}

/** The signed whole number of `unit`s elapsed from `then` to `now` (truncated
 * toward zero). Positive when `then` is in the past. */
function elapsedIn(thenMs: number, nowMs: number, unit: RelativeUnit): number {
  return Math.trunc((nowMs - thenMs) / MS[unit]);
}

/** Renders the elapsed time between `then` and `now` using the largest of the
 * requested `units` that yields a non-zero integer; falls back to the smallest
 * unit (a `0`). Exported for unit tests (mirrors `relative_between`). */
export function relativeBetween(
  thenMs: number,
  nowMs: number,
  units: RelativeUnit[],
): string | null {
  const source = units.length === 0 ? RELATIVE_UNITS : units;
  // Largest unit first, deduped.
  const order = [...new Set(source)].sort((a, b) => RANK[b] - RANK[a]);
  const smallest = order[order.length - 1];
  if (smallest === undefined) return null;

  let chosen: [RelativeUnit, number] | undefined;
  for (const unit of order) {
    const n = elapsedIn(thenMs, nowMs, unit);
    if (Math.abs(n) >= 1) {
      chosen = [unit, n];
      break;
    }
  }
  const [unit, n] = chosen ?? [smallest, elapsedIn(thenMs, nowMs, smallest)];

  const count = Math.abs(n);
  const noun = count === 1 ? NOUN[unit] : `${NOUN[unit]}s`;
  return n < 0 ? `in ${count} ${noun}` : `${count} ${noun} ago`;
}
