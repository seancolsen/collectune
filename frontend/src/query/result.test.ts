import { describe, expect, it, vi } from "vitest";
import * as arrow from "apache-arrow";
import type { AnnotationValue } from "querydown-js";
import { buildResultFromArrow } from "./result";

// Guards the full decode path against the two real-backend regressions: (1)
// annotations arriving as JS `Map`s (serde-wasm-bindgen), and (2) DuckDB list
// columns that apache-arrow can't classify by type. Builds a real Arrow table so
// the test exercises `getChildAt`/`.get()` exactly as production does.

function listColumn(rows: string[][]): arrow.Vector {
  const type = new arrow.List(
    arrow.Field.new({ name: "item", type: new arrow.Utf8(), nullable: true }),
  );
  const b = arrow.makeBuilder({ type, nullValues: [null] });
  // The List builder accepts a plain array at runtime; its typed value is a
  // Vector, so cast to satisfy the overload.
  for (const r of rows) b.append(r as unknown as arrow.Vector<arrow.Utf8>);
  return b.finish().toVector();
}

/** A Map-shaped annotation, mirroring what querydown-js actually returns. */
const mapAnno = (entries: [string, AnnotationValue][]): AnnotationValue =>
  new Map(entries) as unknown as AnnotationValue;

describe("buildResultFromArrow", () => {
  const table = new arrow.Table({
    id: arrow.vectorFromArray(["t1", "t2"], new arrow.Utf8()),
    artists: listColumn([["Beyoncé"], ["Beyoncé", "Jack White"]]),
    duration: arrow.vectorFromArray(["196", "234"], new arrow.Utf8()),
  });

  const annotations: (AnnotationValue | null)[] = [
    mapAnno([["hide", "yes"]]), // id: hidden
    null, // artists: no annotation → list detected from the value
    mapAnno([
      ["align", "right"],
      [
        "formatter",
        new Map([["type", "duration"]]) as unknown as AnnotationValue,
      ],
    ]),
  ];

  it("keeps every column (hidden included), and classifies list vs. scalar", () => {
    const result = buildResultFromArrow(table, annotations);
    expect(result.rowCount).toBe(2);
    expect(result.columns).toHaveLength(3);
    expect(result.columns[0].meta.hide).toBe(true);
    expect(result.columns[1].isList).toBe(true);
    expect(result.columns[2].isList).toBe(false);
  });

  it("drops hidden columns from `visible`, keeping the rest in order", () => {
    const result = buildResultFromArrow(table, annotations);
    expect(result.visible).toHaveLength(2);
    expect(result.visible[0].isList).toBe(true);
    expect(result.visible[1].isList).toBe(false);
  });

  it("renders a list column as pill arrays (value-level fallback)", () => {
    const result = buildResultFromArrow(table, annotations);
    const artists = result.visible[0];
    expect(artists.isList).toBe(true);
    expect(result.pills(0, artists)).toEqual(["Beyoncé"]);
    expect(result.pills(1, artists)).toEqual(["Beyoncé", "Jack White"]);
  });

  it("applies the formatter from a Map-shaped annotation", () => {
    const result = buildResultFromArrow(table, annotations);
    const duration = result.visible[1];
    expect(duration.meta.text_align).toBe("right");
    // 196s → 3:16, 234s → 3:54 (formatter resolved despite the Map annotation).
    expect(result.text(0, duration)).toBe("3:16");
    expect(result.text(1, duration)).toBe("3:54");
  });

  it("reads a hidden column's raw value, even though it's dropped from `visible`", () => {
    const result = buildResultFromArrow(table, annotations);
    expect(result.value(0, 0)).toBe("t1");
    expect(result.keyText(1, 0)).toBe("t2");
  });
});

describe("buildResultFromArrow with a Timestamp column", () => {
  it("feeds the relativeTime formatter a value it parses correctly (not off by 1000x)", () => {
    // Regression: apache-arrow normalizes every Timestamp* unit to epoch ms,
    // and a naive `String(value)` produced a raw ms digit string that
    // `format.ts`'s relative-time parser then misread as epoch *seconds* and
    // multiplied by 1000 again — landing tens of thousands of years off.
    const oneDayAgoMs = Math.floor(Date.now() / 1000) * 1000 - 86_400_000;
    const type = new arrow.TimestampSecond();
    const played = new arrow.Table({
      played_at: arrow.vectorFromArray([oneDayAgoMs], type),
    });
    const annotations: (AnnotationValue | null)[] = [
      mapAnno([
        [
          "formatter",
          new Map<string, unknown>([
            ["type", "relativeTime"],
            ["units", ["days"]],
          ]) as unknown as AnnotationValue,
        ],
      ]),
    ];
    const result = buildResultFromArrow(played, annotations);
    // Exact wording can land on either side of a day boundary depending on
    // the host's local timezone offset — what matters is that it's ~1 day,
    // not (as the bug produced) tens of thousands of years away.
    expect(result.text(0, result.visible[0])).toMatch(/^[01] days? ago$/);
  });

  it("re-derives a relativeTime cell on every read, not just at decode time", () => {
    // The whole point of deriving at paint time: the same stored value reads
    // differently once the clock has moved on, with no re-decode in between —
    // the regression a value frozen at decode time would fail.
    const oneMinuteAgoMs = Math.floor(Date.now() / 1000) * 1000 - 60_000;
    const type = new arrow.TimestampSecond();
    const played = new arrow.Table({
      played_at: arrow.vectorFromArray([oneMinuteAgoMs], type),
    });
    const annotations: (AnnotationValue | null)[] = [
      mapAnno([
        [
          "formatter",
          new Map<string, unknown>([
            ["type", "relativeTime"],
            ["units", ["minutes", "hours"]],
          ]) as unknown as AnnotationValue,
        ],
      ]),
    ];
    const result = buildResultFromArrow(played, annotations);
    const col = result.visible[0];
    const first = result.text(0, col);
    expect(first).toMatch(/^[01] minutes? ago$/);

    try {
      vi.useFakeTimers();
      vi.setSystemTime(new Date(Date.now() + 90 * 60_000)); // jump 90 minutes
      const second = result.text(0, col);
      expect(second).not.toBe(first);
      expect(second).toMatch(/^\d+ hours? ago$/);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("QueryResult.patchRow", () => {
  const table = new arrow.Table({
    id: arrow.vectorFromArray(["t1", "t2", "t3"], new arrow.Utf8()),
    title: arrow.vectorFromArray(["Pray You Catch Me", "Hold Up", "Sorry"]),
  });
  const annotations: (AnnotationValue | null)[] = [
    mapAnno([["hide", "yes"]]),
    null,
  ];

  it("re-points a patched row's cells, leaving its neighbours alone", () => {
    const result = buildResultFromArrow(table, annotations);
    const patch = new arrow.Table({
      id: arrow.vectorFromArray(["t2"], new arrow.Utf8()),
      title: arrow.vectorFromArray(["Hold Up (refreshed)"]),
    });
    expect(result.patchRow(1, patch, 0)).toBe(true);

    const title = result.visible[0];
    expect(result.text(0, title)).toBe("Pray You Catch Me");
    expect(result.text(1, title)).toBe("Hold Up (refreshed)");
    expect(result.text(2, title)).toBe("Sorry");
    expect(result.value(1, 0)).toBe("t2");
  });

  it("skips a patch whose projection doesn't match (a different column count)", () => {
    const result = buildResultFromArrow(table, annotations);
    const mismatched = new arrow.Table({
      title: arrow.vectorFromArray(["Hold Up (refreshed)"]),
    });
    expect(result.patchRow(1, mismatched, 0)).toBe(false);
    expect(result.text(1, result.visible[0])).toBe("Hold Up");
  });
});

// The third real-backend regression: a DuckDB INTERVAL column. apache-arrow
// decodes it to an `Int32Array` of nonsense — iterable, so the list-detection
// fallback above would classify the column as a list and paint a row of pills.
// The decode path has to read the buffer itself and keep the column scalar.
describe("buildResultFromArrow with an INTERVAL column", () => {
  /** `Interval(MONTH_DAY_NANO)` over the byte layout arrow-rs writes: int32
   * months, int32 days, int64 nanoseconds. */
  function intervalColumn(seconds: readonly number[]): arrow.Vector {
    const buffer = new ArrayBuffer(16 * seconds.length);
    const view = new DataView(buffer);
    seconds.forEach((s, i) => {
      view.setInt32(i * 16, 0, true);
      view.setInt32(i * 16 + 4, 0, true);
      view.setBigInt64(i * 16 + 8, BigInt(Math.round(s * 1e9)), true);
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

  const table = new arrow.Table({ duration: intervalColumn([196, 234.5]) });

  it("stays scalar and formats as a duration", () => {
    const result = buildResultFromArrow(table, [
      mapAnno([
        [
          "formatter",
          new Map([["type", "duration"]]) as unknown as AnnotationValue,
        ],
      ]),
    ]);
    const column = result.columns[0];
    expect(column.isList).toBe(false);
    expect(result.text(0, column)).toBe("3:16");
    expect(result.text(1, column)).toBe("3:55");
  });

  it("reads as interval text with no formatter", () => {
    const result = buildResultFromArrow(table, [null]);
    expect(result.text(0, result.columns[0])).toBe("00:03:16");
    expect(result.text(1, result.columns[0])).toBe("00:03:54.5");
  });
});
