import { describe, expect, it } from "vitest";
import {
  childRecordsQuery,
  embeddedRecordQuery,
  embedSpec,
  recordPickerQuery,
} from "./embeddedRecord";
import { buildFormFields, type MultiRecordField } from "./recordForm";
import type { SchemaTable } from "./schema";

const col = (name: string, type: string, nullable = false) => ({
  name,
  type,
  nullable,
});

/** The spec's running example: previewing the credits of one track, where the
 * informative column lives a hop away (`artist.name`) rather than in the table
 * being listed. */
const TABLES: SchemaTable[] = [
  {
    name: "artist",
    columns: [col("id", "UUID"), col("name", "VARCHAR")],
    uniqueConstraints: [["id"], ["name"]],
  },
  {
    name: "credit",
    columns: [
      col("track", "UUID"),
      col("artist", "UUID"),
      col("order", "FLOAT", true),
      col("role", "VARCHAR", true),
    ],
    uniqueConstraints: [["track", "artist"]],
  },
  {
    name: "play",
    columns: [col("track", "UUID"), col("timestamp", "TIMESTAMP_S")],
    uniqueConstraints: [["track", "timestamp"]],
  },
  {
    name: "track",
    columns: [col("id", "UUID"), col("title", "VARCHAR")],
    uniqueConstraints: [["id"]],
  },
];

describe("embedSpec", () => {
  it("displays the top-scoring column, reaching through a foreign key for it", () => {
    // The spec's worked example: artist.name (3) beats role (2), which beats
    // order and artist.id (1 each) — and the foreign keys themselves are never
    // candidates. Only the top score shows, so this is one column.
    expect(embedSpec(TABLES, "credit", "track").display).toEqual([
      "$artist.name",
    ]);
  });

  it("leads the sort with `order`, then the display columns, then the key", () => {
    // The contextual `track` is skipped throughout: it's the same value on
    // every row of the list.
    expect(embedSpec(TABLES, "credit", "track").sort).toBe(
      "\\\\order \\\\artist.name \\\\artist",
    );
  });

  it("sorts a temporal column descending, newest first", () => {
    const spec = embedSpec(TABLES, "play", "track");
    expect(spec.display).toEqual(["$timestamp"]);
    expect(spec.sort).toBe("\\\\timestamp \\d");
  });

  it("previews a record with no contextual column at all", () => {
    // A scalar linked record field: the whole table is in play, and its own
    // required, unique, textual `name` wins.
    expect(embedSpec(TABLES, "artist")).toEqual({
      display: ["$name"],
      sort: "\\\\name \\\\id",
    });
  });

  it("counts a column as unique when it pairs with the contextual column", () => {
    // `credit (track, "order")` makes `order` unique within one track: it gains the
    // uniqueness point and beats `role`. Without that context the two tie, and
    // both show.
    const tables: SchemaTable[] = [
      {
        name: "credit",
        columns: [
          col("track", "UUID"),
          col("order", "FLOAT"),
          col("role", "VARCHAR", true),
        ],
        uniqueConstraints: [["track", "order"]],
      },
    ];
    expect(embedSpec(tables, "credit", "track").display).toEqual(["$order"]);
    expect(embedSpec(tables, "credit").display).toEqual(["$order", "$role"]);
  });

  it("shows at most two columns, in assembly order, when several tie", () => {
    const tables: SchemaTable[] = [
      {
        name: "t",
        columns: [
          col("a", "VARCHAR"),
          col("b", "VARCHAR"),
          col("c", "VARCHAR"),
        ],
        uniqueConstraints: [],
      },
    ];
    expect(embedSpec(tables, "t").display).toEqual(["$a", "$b"]);
  });

  it("has nothing to say about a table the schema doesn't know", () => {
    expect(embedSpec(TABLES, "nope")).toEqual({ display: [], sort: "" });
  });
});

describe("preview queries", () => {
  const creditField = () =>
    buildFormFields(TABLES, "track").find(
      (f) => f.key === "#credit",
    ) as MultiRecordField;

  it("fetches a multi-record field's records and their previews at once", () => {
    const field = creditField();
    expect(
      childRecordsQuery(field, "t1", embedSpec(TABLES, "credit", "track")),
    ).toEqual({
      base: "credit",
      filter: `track:="t1"`,
      sort: "\\\\order \\\\artist.name \\\\artist",
      // The identifying columns first — that's how each child is addressed —
      // then the preview.
      display: "$track $artist $artist.name",
    });
  });

  it("falls back to ordering a list by its keys when there's no preview", () => {
    const field = creditField();
    expect(childRecordsQuery(field, "t1", { display: [], sort: "" })).toEqual({
      base: "credit",
      filter: `track:="t1"`,
      sort: "\\\\track \\\\artist",
      display: "$track $artist",
    });
  });

  it("searches a table for the record picker, keys first", () => {
    const spec = embedSpec(TABLES, "artist");
    expect(
      recordPickerQuery(
        "artist",
        ["id"],
        "name:Wee",
        spec.sort,
        spec.display.join(" "),
      ),
    ).toEqual({
      base: "artist",
      // The user's filter, untouched: it's Querydown they typed themselves.
      filter: "name:Wee",
      sort: "\\\\name \\\\id",
      // The key the picked record is addressed by, then its preview.
      display: "$id $name",
    });
  });

  it("searches with no filter and no display at all", () => {
    expect(recordPickerQuery("artist", ["id"], "", "", "")).toEqual({
      base: "artist",
      filter: "",
      sort: "",
      display: "$id",
    });
  });

  it("fetches one record's preview by key", () => {
    expect(
      embeddedRecordQuery(
        "artist",
        [{ column: "id", value: "a1" }],
        embedSpec(TABLES, "artist"),
      ),
    ).toEqual({
      base: "artist",
      filter: `id:="a1"`,
      sort: "",
      display: "$name",
    });
  });
});
