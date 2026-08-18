import { describe, expect, it } from "vitest";
import * as arrow from "apache-arrow";
import {
  isMonthDayNanoInterval,
  readMonthDayNanoInterval,
  stringifyArrowValue,
} from "./query";

// Regression coverage for the timestamp-stringification bug: apache-arrow's
// `.get()` normalizes every `Timestamp*` unit to milliseconds, so a naive
// `String(value)` produced a raw epoch-ms digit string DuckDB couldn't cast
// back into a `timestamp_s` column, and — one step downstream in
// `query/format.ts` — got double-converted by the relative-time formatter's
// epoch-seconds fallback. Passing the Arrow field type lets this render a
// proper naive datetime string instead.

describe("stringifyArrowValue", () => {
  it("renders a TimestampSecond cell as a naive datetime string, not raw epoch ms", () => {
    // vectorFromArray/builders always take Timestamp* input as epoch *ms*
    // (apache-arrow's own convention, regardless of the column's storage
    // unit) — this is the value shape `.get()` hands back too, and what
    // stringifyArrowValue must correctly turn into a naive datetime string.
    const type = new arrow.TimestampSecond();
    const vector = arrow.vectorFromArray([1782651944000], type);
    expect(stringifyArrowValue(vector.get(0), type)).toBe(
      "2026-06-28 13:05:44",
    );
  });

  it("renders TimestampMillisecond/Microsecond/Nanosecond cells identically for the same instant", () => {
    const ms = 1782651944000;

    const secType = new arrow.TimestampSecond();
    const expected = stringifyArrowValue(
      arrow.vectorFromArray([ms], secType).get(0),
      secType,
    );

    const msType = new arrow.TimestampMillisecond();
    const msVector = arrow.vectorFromArray([ms], msType);
    expect(stringifyArrowValue(msVector.get(0), msType)).toBe(expected);

    const usType = new arrow.TimestampMicrosecond();
    const usVector = arrow.vectorFromArray([ms], usType);
    expect(stringifyArrowValue(usVector.get(0), usType)).toBe(expected);

    const nsType = new arrow.TimestampNanosecond();
    const nsVector = arrow.vectorFromArray([ms], nsType);
    expect(stringifyArrowValue(nsVector.get(0), nsType)).toBe(expected);
  });

  it("falls back to String(value) for non-timestamp types", () => {
    expect(stringifyArrowValue(42, new arrow.Int32())).toBe("42");
    expect(stringifyArrowValue(42)).toBe("42");
  });

  it("returns '' for null/undefined regardless of type", () => {
    const type = new arrow.TimestampSecond();
    expect(stringifyArrowValue(null, type)).toBe("");
    expect(stringifyArrowValue(undefined, type)).toBe("");
  });
});

// Regression coverage for the interval-decoding bug: apache-arrow has no
// MONTH_DAY_NANO branch — the only interval flavor DuckDB emits — so its getter
// silently reads a single int32 where the value is 16 bytes wide. Every
// `file.duration` cell decoded to the string "0,0". `readMonthDayNanoInterval`
// reads the documented layout out of the buffer instead.

/** A one-chunk `Interval(MONTH_DAY_NANO)` vector over the exact byte layout
 * arrow-rs writes: int32 months, int32 days, int64 nanoseconds, little-endian. */
function intervalVector(
  values: readonly (readonly [number, number, bigint] | null)[],
): arrow.Vector {
  const buffer = new ArrayBuffer(16 * values.length);
  const view = new DataView(buffer);
  const nullBitmap = new Uint8Array(Math.ceil(values.length / 8)).fill(0xff);
  values.forEach((value, i) => {
    if (value === null) {
      nullBitmap[i >> 3] &= ~(1 << (i & 7));
      return;
    }
    const [months, days, nanos] = value;
    view.setInt32(i * 16, months, true);
    view.setInt32(i * 16 + 4, days, true);
    view.setBigInt64(i * 16 + 8, nanos, true);
  });
  return new arrow.Vector([
    arrow.makeData({
      type: new arrow.Interval(arrow.IntervalUnit.MONTH_DAY_NANO),
      length: values.length,
      nullCount: values.filter((v) => v === null).length,
      nullBitmap,
      data: new Int32Array(buffer),
    }),
  ]);
}

const SECOND = 1_000_000_000n;

describe("readMonthDayNanoInterval", () => {
  it("recovers durations apache-arrow decodes to garbage", () => {
    const vector = intervalVector([
      [0, 0, 222_250n * 1_000_000n],
      [0, 0, -90n * SECOND],
      [0, 0, 3725n * SECOND],
    ]);
    expect(isMonthDayNanoInterval(vector.type)).toBe(true);
    // What the library's own getter makes of the first one, for contrast.
    expect(String(vector.get(0))).toBe("0,0");

    expect(readMonthDayNanoInterval(vector, 0)).toBe("00:03:42.25");
    expect(readMonthDayNanoInterval(vector, 1)).toBe("-00:01:30");
    expect(readMonthDayNanoInterval(vector, 2)).toBe("01:02:05");
  });

  it("renders the month and day components, zero, and NULL", () => {
    const vector = intervalVector([
      [0, 2, 90n * SECOND],
      [14, 0, 0n],
      [0, 0, 0n],
      null,
    ]);
    expect(readMonthDayNanoInterval(vector, 0)).toBe("2 days 00:01:30");
    expect(readMonthDayNanoInterval(vector, 1)).toBe("14 months");
    expect(readMonthDayNanoInterval(vector, 2)).toBe("00:00:00");
    expect(readMonthDayNanoInterval(vector, 3)).toBeNull();
  });
});
