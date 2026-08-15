// The structured, four-part saved-query definition. A stored `query.definition`
// is a JSON string that deserializes to `QueryDefinition`; `assemble` resolves
// it — with
// preset references looked up against the loaded preset list — into either
// per-section Querydown source (the builder default) or a single hand-written
// query (full mode). This is pure string manipulation: no compiler, no schema.

import type { Preset } from "api-client";

/** A sort or display section: hand-written Querydown (`custom`), a reference to
 * a saved preset (`preset`), or a built-in parameterized preset (`builtin`).
 * Matches the stored JSON's `SectionContent` union (serde `rename_all =
 * "lowercase"`, externally tagged). The filter section instead uses
 * `FilterParts`. */
export type SectionContent =
  | { custom: string }
  | { preset: string } // preset id (UUID)
  | { builtin: BuiltinPreset };

/** A built-in sorting preset carrying its parameters inline in the stored query.
 * Serde tags it with `preset` (`#[serde(tag = "preset")]`); only `shuffle`
 * exists. */
export type BuiltinPreset = { preset: "shuffle"; seed: string };

/** Characters a shuffle seed is drawn from. */
const SEED_ALPHABET =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const SEED_LEN = 20;

/** A random 20-char alphanumeric shuffle seed. Uses the platform CSPRNG; the
 * slight modulo bias is irrelevant for a seed. */
export function generateSeed(): string {
  const bytes = new Uint8Array(SEED_LEN);
  crypto.getRandomValues(bytes);
  let out = "";
  for (const b of bytes) out += SEED_ALPHABET[b % SEED_ALPHABET.length];
  return out;
}

/** A fresh Shuffle built-in section content (a new seed reshuffles the order). */
export function shuffleContent(): SectionContent {
  return { builtin: { preset: "shuffle", seed: generateSeed() } };
}

/** The filter section: custom conditions AND-combined with any number of
 * presets (referenced by id). */
export interface FilterParts {
  custom: string;
  presets: string[];
}

/** A saved query split into its four Querydown parts. `full`, when present, is
 * the whole hand-written query and the sectioned parts are ignored. */
export interface QueryDefinition {
  base: string;
  filter: FilterParts;
  sort: SectionContent;
  display: SectionContent;
  full?: string | null;
}

/** The three query sections with builder UIs and saveable presets. */
export type Section = "filter" | "sort" | "display";

/** The short label used on a section's toolbar toggle button. */
export function sectionLabel(section: Section): string {
  return section === "filter"
    ? "Filter"
    : section === "sort"
      ? "Sort"
      : "Display";
}

/** The noun used in builder headings and menus ("Filter"/"Sorting"/"Display"). */
export function sectionNoun(section: Section): string {
  return section === "sort" ? "Sorting" : sectionLabel(section);
}

/** A fresh, empty sectioned definition (no base chosen). The default working
 * state, and the fallback when a stored definition can't be parsed. */
export function emptyDefinition(): QueryDefinition {
  return {
    base: "",
    filter: { custom: "", presets: [] },
    sort: { custom: "" },
    display: { custom: "" },
  };
}

/** A deep, independent copy of a definition — so a tab's working (`live`) copy
 * can be mutated without touching its `saved` baseline. */
export function cloneDefinition(def: QueryDefinition): QueryDefinition {
  return structuredClone(def);
}

/** Whether a sort/display section holds nothing at all — the empty custom
 * fragment {@link emptyDefinition} starts it out with. */
function isEmptySection(content: SectionContent): boolean {
  return "custom" in content && content.custom.trim() === "";
}

/** A fresh definition for `base`, seeded with that table's default presets:
 * every default filter preset (the filter combines them), and the first default
 * sort / display preset (each of those sections holds exactly one thing, so any
 * further default for it is ignored). */
export function definitionForBase(
  base: string,
  presets: readonly Preset[],
): QueryDefinition {
  const def = emptyDefinition();
  def.base = base;
  for (const preset of presets) {
    if (!preset.isDefault || preset.baseTable !== base) continue;
    if (preset.section === "filter") {
      def.filter.presets.push(preset.id);
    } else if (preset.section === "sort" && isEmptySection(def.sort)) {
      def.sort = { preset: preset.id };
    } else if (preset.section === "display" && isEmptySection(def.display)) {
      def.display = { preset: preset.id };
    }
  }
  return def;
}

/** `def` moved onto a different base table. The hand-written filter carries over
 * verbatim — it's what the user typed, and re-typing it after every base change
 * would be the annoying part. Everything else is rebuilt from the new table's
 * defaults ({@link definitionForBase}): filter presets, sort and display are all
 * scoped to the table they were written against, so they can't follow the query
 * to another one. A full-mode definition drops back to sectioned mode. */
export function rebasedDefinition(
  def: QueryDefinition,
  base: string,
  presets: readonly Preset[],
): QueryDefinition {
  const next = definitionForBase(base, presets);
  next.filter.custom = def.filter.custom;
  return next;
}

/** A section's content rewritten into a canonical, fixed-key-order object, so
 * two structurally-equal sections serialize identically regardless of how they
 * were built (parsed JSON vs. mutated in place). */
function canonicalSection(c: SectionContent): unknown {
  if ("custom" in c) return { custom: c.custom };
  if ("preset" in c) return { preset: c.preset };
  return { builtin: { preset: c.builtin.preset, seed: c.builtin.seed } };
}

/** A definition rewritten into a canonical shape for structural comparison. */
function canonicalDef(def: QueryDefinition): unknown {
  return {
    base: def.base,
    filter: { custom: def.filter.custom, presets: [...def.filter.presets] },
    sort: canonicalSection(def.sort),
    display: canonicalSection(def.display),
    full: def.full ?? null,
  };
}

/** Whether two definitions are structurally equal — the "does the working copy
 * differ from the saved one?" test that drives the unsaved-changes indicator. */
export function defsEqual(a: QueryDefinition, b: QueryDefinition): boolean {
  return JSON.stringify(canonicalDef(a)) === JSON.stringify(canonicalDef(b));
}

/** Parses a stored definition into a complete working copy, filling any missing
 * fields from the empty default (so a sparse `{}` or partial JSON still yields a
 * full `filter`/`sort`/`display`). Falls back to the empty definition when the
 * string is blank or unparseable (this phase skips the legacy raw-Querydown
 * split). */
export function definitionFromStored(raw: string): QueryDefinition {
  const parsed = fromStored(raw);
  if (!parsed) return emptyDefinition();
  const base = emptyDefinition();
  return {
    base: typeof parsed.base === "string" ? parsed.base : base.base,
    filter: {
      custom: parsed.filter?.custom ?? base.filter.custom,
      presets: parsed.filter?.presets ?? base.filter.presets,
    },
    sort: parsed.sort ?? base.sort,
    display: parsed.display ?? base.display,
    full: parsed.full ?? undefined,
  };
}

/** What `assemble` resolves a definition into for compilation: independently
 * parsed sections (sectioned mode) or one whole-query string (full mode). */
export type CompileSource =
  | { kind: "full"; text: string }
  | {
      kind: "sections";
      base: string;
      filter: string;
      sort: string;
      display: string;
    };

/** Querydown definitions (RadioCrate's computed columns + custom comparisons)
 * prepended to every query before compilation. */
export const PRELUDE = `#track.firstplay = #play.timestamp%min
#track.lastplay = #play.timestamp%max
#track.artists = #credit.artist.name%list(\\\\order \\\\artist.name)
#track.year = album.year
#track.added = file.added
#track.duration = file.duration
#track.playcount = #play
#track.number = track_number
#track.__querydown_default_text_search:@x = [
  title:@x
  album.title:@x
  ++#artist{name:@x}
  ++#tag{name:@x}
]
#track.artist:@x = ++#artist{name:@x}`;

/** Serializes a working definition into the JSON string persisted in the
 * backend's `query.definition` column — the inverse of {@link fromStored}.
 * `full` is omitted when absent so a sectioned query never carries a stray
 * `"full": null`. */
export function definitionToStored(def: QueryDefinition): string {
  const out: Record<string, unknown> = {
    base: def.base,
    filter: { custom: def.filter.custom, presets: [...def.filter.presets] },
    sort: def.sort,
    display: def.display,
  };
  if (def.full != null) out.full = def.full;
  return JSON.stringify(out);
}

/** Parses a stored `query.definition` JSON string into a `QueryDefinition`.
 * A parse failure yields `null` — for this crude phase that simply means no
 * results (the legacy raw-Querydown split is skipped). */
export function fromStored(raw: string): QueryDefinition | null {
  try {
    return JSON.parse(raw) as QueryDefinition;
  } catch {
    return null;
  }
}

/** The Querydown fragment a built-in preset resolves to: Shuffle orders by a
 * salted hash of each row's id. */
function builtinQuerydown(b: BuiltinPreset): string {
  // `\\id|concat('<seed>')|md5` — the two leading backslashes are literal.
  return `\\\\id|concat('${b.seed}')|md5`;
}

function presetDefinition(presets: Preset[], id: string): string {
  const found = presets.find((p) => p.id === id);
  if (!found) {
    throw new Error("This query references a preset that no longer exists.");
  }
  return found.definition;
}

/** Resolves a sort/display `SectionContent` to its Querydown fragment. */
function resolveSection(content: SectionContent, presets: Preset[]): string {
  if ("custom" in content) return content.custom.trim();
  if ("preset" in content)
    return presetDefinition(presets, content.preset).trim();
  return builtinQuerydown(content.builtin);
}

/** The Querydown text a sort/display section resolves to, used to seed the
 * custom editor when switching a preset/built-in back to "Custom …" so nothing
 * is lost. A dangling preset reference resolves to empty rather than
 * throwing. */
export function sectionSeedText(
  content: SectionContent,
  presets: readonly Preset[],
): string {
  if ("custom" in content) return content.custom;
  if ("preset" in content)
    return presets.find((p) => p.id === content.preset)?.definition ?? "";
  return builtinQuerydown(content.builtin);
}

/** Resolves a definition into the shape the matching compiler entry wants.
 * Full mode returns the hand-written text; sectioned mode returns the base
 * plus the filter (custom + each referenced preset's fragment,
 * newline-joined), sort, and display. Throws on an empty/unrunnable
 * definition or a dangling preset reference. */
export function assemble(
  def: QueryDefinition,
  presets: Preset[],
): CompileSource {
  if (def.full != null) {
    const text = def.full.trim();
    if (text === "") throw new Error("The query is empty.");
    return { kind: "full", text };
  }
  const base = def.base.trim();
  if (base === "") throw new Error("No base table selected.");

  const filterParts: string[] = [];
  const custom = def.filter.custom.trim();
  if (custom !== "") filterParts.push(custom);
  for (const id of def.filter.presets) {
    const fragment = presetDefinition(presets, id).trim();
    if (fragment !== "") filterParts.push(fragment);
  }

  return {
    kind: "sections",
    base,
    filter: filterParts.join("\n"),
    sort: resolveSection(def.sort, presets),
    display: resolveSection(def.display, presets),
  };
}

/** A definition flattened into one hand-written Querydown query — what the
 * "Full Querydown" conversion seeds its editor with: the base table with its `#`
 * sigil, then the filter, sort and display fragments, newline-separated. Preset
 * references resolve to their text (a dangling one is simply omitted, so the
 * conversion always yields editable text rather than throwing), which is exactly
 * what makes this a one-way door: the parts stop being separately editable.
 * An already-full definition returns its text unchanged. */
export function toFullQuery(
  def: QueryDefinition,
  presets: readonly Preset[],
): string {
  if (def.full != null) return def.full;

  const filterParts: string[] = [];
  const custom = def.filter.custom.trim();
  if (custom !== "") filterParts.push(custom);
  for (const id of def.filter.presets) {
    const fragment = presets.find((p) => p.id === id)?.definition.trim();
    if (fragment) filterParts.push(fragment);
  }

  const parts = [`#${def.base.trim()}`];
  for (const fragment of [
    filterParts.join("\n"),
    sectionSeedText(def.sort, presets),
    sectionSeedText(def.display, presets),
  ]) {
    if (fragment.trim() !== "") parts.push(fragment.trim());
  }
  return parts.join("\n");
}
