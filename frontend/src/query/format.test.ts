import { describe, expect, it } from "vitest";
import {
  displayText,
  formatValue,
  parseFormatter,
  relativeBetween,
  type Formatter,
} from "./format";
import { defaultColumnMetadata } from "./columns";

// The relative-time cases use the exported `relativeBetween` with explicit
// instants (the public `formatValue` uses `Date.now()`).

const numberFmt = (dp: number | number[]): Formatter => ({
  type: "number",
  decimalPlaces: dp,
});

describe("Number formatter", () => {
  it("min/max fraction digits", () => {
    const f = numberFmt([0, 1]);
    expect(formatValue(f, "3")).toBe("3");
    expect(formatValue(f, "3.0")).toBe("3");
    expect(formatValue(f, "3.14")).toBe("3.1");
    expect(formatValue(f, "1234567.8")).toBe("1234567.8");
    expect(formatValue(f, "-3.0")).toBe("-3");
  });

  it("keeps minimum digits", () => {
    const f = numberFmt([2, 4]);
    expect(formatValue(f, "3.5")).toBe("3.50");
    expect(formatValue(f, "3")).toBe("3.00");
  });

  it("bad input is null", () => {
    const f = numberFmt([]);
    expect(formatValue(f, "")).toBeNull();
    expect(formatValue(f, "N/A")).toBeNull();
  });
});

describe("Duration formatter", () => {
  it("formats M:SS", () => {
    const f: Formatter = { type: "duration" };
    expect(formatValue(f, "245")).toBe("4:05");
    expect(formatValue(f, "245.0")).toBe("4:05");
    expect(formatValue(f, "0")).toBe("0:00");
    expect(formatValue(f, "66.4")).toBe("1:06");
    expect(formatValue(f, "3600")).toBe("60:00");
    expect(formatValue(f, "nope")).toBeNull();
  });

  // `file.duration` and the `track.*_position` columns are INTERVALs as of
  // migration 0003, so a duration cell now usually arrives as the interval text
  // `api/query.ts` reads out of the Arrow buffer rather than as bare seconds.
  it("formats an INTERVAL cell's text as well as bare seconds", () => {
    const f: Formatter = { type: "duration" };
    expect(formatValue(f, "00:03:42.25")).toBe("3:42");
    expect(formatValue(f, "00:00:00")).toBe("0:00");
    expect(formatValue(f, "01:02:05")).toBe("62:05");
    expect(formatValue(f, "-00:01:30")).toBe("-1:30");
    expect(formatValue(f, "2 days 00:01:30")).toBe("2881:30");
    expect(formatValue(f, "1 day -00:00:30")).toBe("1439:30");
  });

  // A month is not a fixed number of seconds, so an interval carrying one is
  // not a duration: the formatter declines it and the caller shows it raw.
  it("declines an interval with a month component", () => {
    expect(formatValue({ type: "duration" }, "14 months")).toBeNull();
  });
});

describe("Timestamp formatter", () => {
  it("formats via reduced strftime", () => {
    const f: Formatter = { type: "timestamp", format: "%Y-%m-%d" };
    expect(formatValue(f, "2026-06-07 14:30:45")).toBe("2026-06-07");
    expect(formatValue(f, "2026-06-07")).toBe("2026-06-07");
    expect(formatValue(f, "not a date")).toBeNull();
  });

  it("supports time specifiers", () => {
    const f: Formatter = { type: "timestamp", format: "%H:%M:%S" };
    expect(formatValue(f, "2026-06-07 14:30:45")).toBe("14:30:45");
  });
});

describe("relativeBetween", () => {
  const at = (y: number, mo: number, d: number, h: number, mi: number) =>
    Date.UTC(y, mo - 1, d, h, mi, 0);

  it("picks the largest fitting unit", () => {
    const now = at(2026, 6, 7, 12, 0);
    const then = at(2026, 6, 3, 12, 0); // 4 days earlier
    expect(relativeBetween(then, now, [])).toBe("4 days ago");
  });

  it("singular and zero fallback", () => {
    const now = at(2026, 6, 7, 12, 0);
    expect(relativeBetween(at(2026, 6, 6, 12, 0), now, ["days"])).toBe(
      "1 day ago",
    );
    expect(relativeBetween(at(2026, 6, 7, 11, 59), now, ["minutes"])).toBe(
      "1 minute ago",
    );
    expect(relativeBetween(at(2026, 6, 7, 12, 0), now, ["minutes"])).toBe(
      "0 minutes ago",
    );
  });

  it("future uses 'in'", () => {
    const now = at(2026, 6, 7, 12, 0);
    const later = at(2026, 6, 10, 12, 0); // 3 days later
    expect(relativeBetween(later, now, [])).toBe("in 3 days");
  });
});

describe("parseFormatter", () => {
  it("parses a duration blob", () => {
    expect(parseFormatter({ type: "duration" })).toEqual({ type: "duration" });
  });

  it("defaults decimalPlaces when absent", () => {
    expect(parseFormatter({ type: "number" })).toEqual({
      type: "number",
      decimalPlaces: [],
    });
  });

  it("requires a timestamp format", () => {
    expect(parseFormatter({ type: "timestamp" })).toBeUndefined();
    expect(parseFormatter({ type: "timestamp", format: "%Y" })).toEqual({
      type: "timestamp",
      format: "%Y",
    });
  });

  it("returns undefined for unknown/malformed blobs", () => {
    expect(parseFormatter({ type: "bogus" })).toBeUndefined();
    expect(parseFormatter("nonsense")).toBeUndefined();
    expect(parseFormatter(null)).toBeUndefined();
  });
});

describe("displayText", () => {
  it("applies the formatter then wraps with prefix/suffix", () => {
    const meta = {
      ...defaultColumnMetadata(),
      prefix: "#",
      formatter: { type: "duration" } as Formatter,
    };
    expect(displayText(meta, "245")).toBe("#4:05");
  });

  it("falls back to the raw value when the formatter fails", () => {
    const meta = {
      ...defaultColumnMetadata(),
      formatter: { type: "duration" } as Formatter,
    };
    expect(displayText(meta, "nope")).toBe("nope");
  });

  it("passes the raw value through when there is no formatter", () => {
    expect(displayText(defaultColumnMetadata(), "Lemonade")).toBe("Lemonade");
  });
});
