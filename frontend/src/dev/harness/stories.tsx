import { Index, type JSX } from "solid-js";
import type { CommandStore } from "../../state/commands";
import type { AppStore, RecordRef } from "../../state/store";
import { SETTINGS } from "../../state/settings";
import { emptyDefinition, type QueryDefinition } from "../../query/definition";
import { emptyCountResult, lemonadeGridResult } from "../gridFixture";
import {
  FIXTURE_SCHEMA_JSON,
  fixtureQuery,
  installRecordFixture,
} from "../recordFixture";
import { failDml, STUB_VERSION } from "./mockApi";
import {
  FILTER_DEF,
  FULL_DEF,
  QUERIES_FIXTURE,
  SHUFFLE_DEF,
  VETTED_PRESET_ID,
} from "../fixtures";

import { AboutDialog } from "../../components/AboutModal";
import { SettingDialog } from "../../components/SettingModal";
import CommandPalette from "../../components/CommandPalette";
import Explorer from "../../components/Explorer";
import NowPlaying from "../../components/NowPlaying";
import PageActionsMenu from "../../components/PageActionsMenu";
import PlaybackActionsMenu from "../../components/PlaybackActionsMenu";
import QueryResults from "../../components/QueryResults";
import QueryToolbar from "../../components/QueryToolbar";
import RecordEditorPanel from "../../components/RecordEditorPanel";
import RowActionsMenu from "../../components/RowActionsMenu";
import SettingsMenu from "../../components/SettingsMenu";
import { UpdateBar } from "../../components/UpdateBanner";
import { CaptureDialog } from "../../components/ShortcutsPage";
import RecordPicker from "../../components/RecordPicker";
import QueryBuilder from "../../components/builder/QueryBuilder";
import EmbeddedRecord from "../../components/record/EmbeddedRecord";
import { ContextMenu } from "../../components/ui/ContextMenu";
import { Menu } from "../../components/ui/Menu";
import SidebarLeft from "../../components/ui/SidebarLeft";

// The visual-test harness's catalogue: one entry per snapshot, each putting a
// single component on screen with the state or props that snapshot is about —
// and nothing else. `tests/visual/*.spec.ts` navigate to
// `/harness.html?story=<id>` and shoot the stage (or, for a component that
// portals out of it, the dialog or menu it raised).
//
// A story is meant to read as the shortest description of what the snapshot
// shows. When one can't — when reaching a state takes a chain of interactions
// against a stand-in backend — that's the sign the scenario belongs in an
// end-to-end test against a real one instead.

/** One catalogue entry. */
export interface Story {
  /** Stage width in CSS px. Omit to fill the viewport (which is then the test's
   * to size — what a component whose own layout responds to the viewport,
   * like `SidebarLeft`, needs). */
  width?: number;
  /** Stage height in CSS px. Omit to let the stage hug its contents. */
  height?: number;
  /** Extra classes for the stage — its flex direction, mostly. */
  frame?: string;
  /** Puts the store (and the command store) into the state the story is about,
   * once the canned query and preset lists have loaded. */
  setup?: (store: AppStore, commands: CommandStore) => void;
  render: () => JSX.Element;
}

const [LEMONADE, DEEP_CUTS] = QUERIES_FIXTURE;

/** Opens the "Lemonade" fixture query in a tab and returns its id. */
function openLemonade(store: AppStore): string {
  store.openTab({ id: LEMONADE.id, name: LEMONADE.name, definition: "" });
  return LEMONADE.id;
}

/** Opens "Lemonade" carrying `def` as its working query. Unless it's `saved`,
 * the tab reads as having unsaved changes (an empty baseline), which is what
 * puts the Save button in the toolbar. */
function openWithDefinition(
  store: AppStore,
  def: QueryDefinition,
  saved = false,
): string {
  const id = openLemonade(store);
  store.setTabDefinitions(id, saved ? def : emptyDefinition(), def);
  return id;
}

/** The filter builder with the "vetted" preset's inline editor open. */
function expandedVettedPreset(store: AppStore): string {
  const id = openWithDefinition(store, FILTER_DEF);
  store.toggleBuilderSection(id, "filter");
  store.toggleExpandPreset(id, VETTED_PRESET_ID);
  return id;
}

/** The track the record-editor stories edit: a row of the seeded grid, keyed as
 * `?records=` keys them (row N is `track-N`). */
const trackRecord = (n: number): RecordRef => ({
  table: "track",
  key: [{ column: "id", value: `track-${n}` }],
});

/** A record-editor story: the panel alone, on the tracks `ns` names, over the
 * canned schema and record data (`recordFixture.ts`) that stand in for a
 * backend. More than one track is the bulk case — the same form, on every record
 * the result-row selection covers. `saveFails` makes the next save come back
 * refused, with that message. */
function recordEditor(ns: readonly number[], saveFails?: string): Story {
  return {
    width: 340,
    height: 640,
    frame: "flex",
    setup: (store) => {
      if (saveFails !== undefined) failDml(saveFails);
      store.setSchemaJson(FIXTURE_SCHEMA_JSON);
      installRecordFixture(0);
      openLemonade(store);
    },
    render: () => (
      <RecordEditorPanel
        tabId={LEMONADE.id}
        target={{ table: "track", records: ns.map(trackRecord) }}
      />
    ),
  };
}

/** Filler for the stories about a component's own layout rather than about
 * anything it holds. */
function Lorem(props: { heading: string; lines: number }): JSX.Element {
  const text =
    "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do " +
    "eiusmod tempor incididunt ut labore et dolore magna aliqua.";
  return (
    <div class="text-ink flex flex-col gap-2 p-3 text-sm">
      <h2 class="font-semibold">{props.heading}</h2>
      <Index each={Array.from({ length: props.lines })}>
        {() => <p class="text-ink-weak">{text}</p>}
      </Index>
    </div>
  );
}

export const STORIES: Record<string, Story> = {
  // ── The app's left sidebar, as a general-purpose panel ────────────────────
  // Both stories are about `SidebarLeft`'s two layouts and nothing else, so
  // what it holds is filler. The layout it picks comes from the viewport, so
  // these stories take their size from the test rather than declaring one.
  "sidebar-left/open-persistent": {
    frame: "flex",
    render: () => (
      <SidebarLeft
        open
        onClose={() => {}}
        main={<Lorem heading="Main content" lines={3} />}
      >
        <Lorem heading="Sidebar" lines={2} />
      </SidebarLeft>
    ),
  },
  "sidebar-left/open-ephemeral": {
    frame: "flex",
    render: () => (
      <SidebarLeft
        open
        onClose={() => {}}
        main={<Lorem heading="Main content" lines={3} />}
      >
        <Lorem heading="Sidebar" lines={2} />
      </SidebarLeft>
    ),
  },

  // ── The explorer, without the panel it fills ─────────────────────────────
  "explorer/basic": {
    width: 200,
    height: 600,
    frame: "flex flex-col",
    setup: (store) => {
      const active = openLemonade(store);
      store.openTab({ id: DEEP_CUTS.id, name: DEEP_CUTS.name, definition: "" });
      store.selectTab(active);
    },
    render: () => <Explorer />,
  },

  // ── Settings ─────────────────────────────────────────────────────────────
  "settings/menu": {
    width: 200,
    // Tall enough to clear the menu's bottom edge (its last row is "About
    // RadioCrate"), so the snapshot isn't cropping the panel.
    height: 350,
    render: () => (
      <Menu defaultOpen width="180px" trigger={() => null}>
        <SettingsMenu />
      </Menu>
    ),
  },
  // The Querydown prelude setting in its editor, holding the default value —
  // the state the dialog opens in for anyone who hasn't customized it, and the
  // one that shows both the monospace field and the disabled "Reset to default".
  "settings/prelude": {
    render: () => (
      <SettingDialog
        name={SETTINGS.querydown_prelude.name}
        description={SETTINGS.querydown_prelude.description}
        value={SETTINGS.querydown_prelude.default}
        defaultValue={SETTINGS.querydown_prelude.default}
        onSave={() => {}}
        onClose={() => {}}
      />
    ),
  },
  // The rebind dialog holding a chord another command already owns, so the
  // "currently bound to" warning shows too (⌘/Ctrl+S is "Tabs: Save active
  // tab").
  "settings/keyboard-shortcuts/modal-assign": {
    render: () => (
      <CaptureDialog
        cmd="explorer.toggle"
        pending={{ mod: true, ctrl: false, shift: false, alt: false, key: "S" }}
        onAssign={() => {}}
        onUnbind={() => {}}
        onReset={() => {}}
        onCancel={() => {}}
      />
    ),
  },

  // ── Client updates ───────────────────────────────────────────────────────
  // Both notices at once, since the bar is the same component either way and
  // the difference is entirely wording: "ready" (a downloaded update waiting,
  // dismissible) above "stale" (the server has moved on from this client — the
  // stronger, non-dismissible warning, whose button is the fresh-copy reload).
  // Rendered from props rather than from the update controller, which needs a
  // service worker the harness deliberately doesn't have.
  "update/banner": {
    width: 720,
    frame: "flex flex-col gap-2",
    render: () => (
      <>
        <UpdateBar notice="ready" onReload={() => {}} onDismiss={() => {}} />
        <UpdateBar notice="stale" onReload={() => {}} onDismiss={() => {}} />
      </>
    ),
  },
  // The About dialog on a *mismatched* pair of build ids — the state it exists
  // to explain. Both ids are pinned here (the client's by prop, the server's by
  // `STUB_VERSION`) rather than read from `__BUILD_ID__`, so the baseline
  // doesn't churn with every commit.
  "about/modal": {
    render: () => (
      <AboutDialog
        clientBuildId="3c7ae10"
        version={STUB_VERSION}
        onCheck={() => {}}
        onReloadFresh={() => {}}
        onClose={() => {}}
      />
    ),
  },

  // ── The query toolbar ────────────────────────────────────────────────────
  // Saved (clean) query, no builder open: no Save button, the section toggles
  // inactive, "12 results" at the far right.
  "query-builder/collapsed": {
    width: 1280,
    setup: (store) => {
      const id = openWithDefinition(store, FILTER_DEF, true);
      store.setResults(id, emptyCountResult(12));
    },
    render: () => <QueryToolbar tabId={LEMONADE.id} />,
  },
  // Filter section open + unsaved: the Save button, the active split button
  // with its ⋮, and the builder line below it.
  "query-builder/filter-open": {
    width: 1280,
    setup: (store) => {
      const id = openWithDefinition(store, FILTER_DEF);
      store.setResults(id, emptyCountResult(12));
      store.toggleBuilderSection(id, "filter");
    },
    render: () => <QueryToolbar tabId={LEMONADE.id} />,
  },
  // Compact (≤ 500px): the section buttons drop their labels and the
  // run/filter separator is hidden.
  "query-builder/filter-open-narrow": {
    width: 460,
    setup: (store) => {
      const id = openWithDefinition(store, FILTER_DEF);
      store.setResults(id, emptyCountResult(12));
      store.toggleBuilderSection(id, "filter");
    },
    render: () => <QueryToolbar tabId={LEMONADE.id} />,
  },
  // Full-Querydown mode: the three section toggles collapse into one
  // "Querydown" toggle (no ⋮ — there are no sections to configure) over the
  // whole-query editor.
  "query-builder/querydown": {
    width: 1280,
    setup: (store) => {
      const id = openWithDefinition(store, FULL_DEF, true);
      store.setResults(id, emptyCountResult(12));
      store.toggleFullEditor(id);
    },
    render: () => <QueryToolbar tabId={LEMONADE.id} />,
  },
  // The wrench menu, with its Base submenu open (the test opens it): the
  // schema's tables as an exclusive choice over the "Full Querydown" escape
  // hatch.
  "query-builder/actions-menu": {
    width: 440,
    height: 300,
    setup: (store) => {
      store.setSchemaJson(FIXTURE_SCHEMA_JSON);
      openWithDefinition(store, FILTER_DEF, true);
    },
    render: () => (
      <Menu defaultOpen width="210px" trigger={() => null}>
        <PageActionsMenu tabId={LEMONADE.id} />
      </Menu>
    ),
  },

  // ── The builders, without the toolbar above them ─────────────────────────
  // Preset expanded, no unsaved edits: the inline editor (name + apply-by-
  // default + definition), no star/revert/save.
  "filter-builder/preset-expanded": {
    width: 1280,
    setup: expandedVettedPreset,
    render: () => <QueryBuilder tabId={LEMONADE.id} />,
  },
  // Narrow: the "Apply by default" checkbox wraps onto its own line below the
  // name row.
  "filter-builder/preset-expanded-narrow": {
    width: 560,
    setup: expandedVettedPreset,
    render: () => <QueryBuilder tabId={LEMONADE.id} />,
  },
  // The same preset made dirty by toggling "Apply by default" on: red ✱ plus
  // revert and save appear, and the checkbox is checked.
  "filter-builder/modified-preset": {
    width: 1280,
    setup: (store) => {
      expandedVettedPreset(store);
      store.patchPresetEdit(VETTED_PRESET_ID, { isDefault: true });
    },
    render: () => <QueryBuilder tabId={LEMONADE.id} />,
  },
  // Sort built-in: the Shuffle preset tab beside its Reshuffle button.
  "sort-builder/shuffle": {
    width: 1280,
    setup: (store) => {
      const id = openWithDefinition(store, SHUFFLE_DEF);
      store.toggleBuilderSection(id, "sort");
    },
    render: () => <QueryBuilder tabId={LEMONADE.id} />,
  },

  // ── Results ──────────────────────────────────────────────────────────────
  // The canned Lemonade rows, so the grid's columns, artist pills, per-column
  // fonts/colors/alignment, formatters and separators are all captured.
  "results/basic": {
    width: 1280,
    height: 200,
    frame: "flex flex-col",
    setup: (store) => {
      const id = openLemonade(store);
      const { result, lineage } = lemonadeGridResult();
      store.setResults(id, result, lineage);
    },
    render: () => <QueryResults tabId={LEMONADE.id} />,
  },
  // A row's context menu: one entry per table whose primary key the row
  // carries.
  "result-row/context-menu": {
    render: () => (
      <ContextMenu x={8} y={8} onClose={() => {}}>
        <RowActionsMenu
          records={[
            trackRecord(1),
            { table: "album", key: [{ column: "id", value: "album-1" }] },
          ]}
          onEdit={() => {}}
        />
      </ContextMenu>
    ),
  },

  // ── The now-playing bar ──────────────────────────────────────────────────
  "now-playing/playing": {
    width: 1280,
    frame: "flex flex-col",
    setup: (store) => seedPlayback(store),
    render: () => <NowPlaying />,
  },
  // Its overflow menu on its own. "Locate" is disabled: the seeded track is
  // deliberately left unlocated in the results (see `seedPlayback`).
  "now-playing/menu": {
    width: 160,
    height: 120,
    setup: (store) => seedPlayback(store),
    render: () => (
      <Menu defaultOpen width="130px" trigger={() => null}>
        <PlaybackActionsMenu />
      </Menu>
    ),
  },

  // ── The command palette ──────────────────────────────────────────────────
  // Filtered by a typed query, which also drops the non-matching commands.
  // Which commands are *available* comes from the store, so the story opens a
  // query tab with results the way the palette's `When` predicates expect.
  "command-palette/filtered": {
    setup: (store, commands) => {
      const id = openLemonade(store);
      store.setResults(id, lemonadeGridResult().result);
      commands.togglePalette();
      commands.setPaletteQuery("tab");
    },
    render: () => <CommandPalette />,
  },

  // ── The record editor ────────────────────────────────────────────────────
  // As the form lands: every field of `track`, values and counts loaded,
  // everything collapsed.
  "record-editor/items-collapsed": recordEditor([1]),
  // …and opened out (the test does the opening): a long text field expanded
  // below its label, a multi-record field listing its records, and one of those
  // expanded into its own form.
  "record-editor/items-expanded": recordEditor([3]),
  // Mid-edit: an edited field and a record being created under `credit`, each
  // starred. On track 5, whose title is short — track 3's is the long one the
  // expansion story is about, and this story wants a plain single-line edit.
  "record-editor/modified": recordEditor([5]),
  // A save the database refused: what it said, above a form still holding the
  // change it couldn't write.
  "record-editor/save-error": recordEditor(
    [1],
    'Duplicate key "title: Sorry" violates unique constraint.',
  ),
  // Two records at once: the fields they agree on (`disc_number`) editable as
  // ever, the ones they don't reading "(varied)", the `credit` count they
  // happen to share still shown beside the message that says child records
  // aren't editable in bulk yet, and the `play` count they don't share varied
  // like any other value.
  "record-editor/bulk": recordEditor([3, 5]),
  // The modal record picker on its own: the search box with its sort and
  // display buttons, the results as the embedded records they're about to
  // become, and the "New record" way out. `initialSort`/`initialDisplay` are
  // hard-coded to what `embedSpec` picks for `artist` (see
  // `query/embeddedRecord.test.ts`) — deriving them here would make this story
  // about the generator rather than about the picker.
  "record-picker/basic": {
    render: () => (
      <RecordPicker
        table="artist"
        keyColumn="id"
        initialSort="\\\\name \\\\id"
        initialDisplay="$name"
        runQuery={(q) => Promise.resolve(fixtureQuery(q))}
        onPick={() => {}}
        onCancel={() => {}}
        onCreate={() => {}}
      />
    ),
  },

  // ── Embedded records ─────────────────────────────────────────────────────
  // One preview widget, from cells alone: a selected member of a multi-record
  // field, wearing the results grid's selected-row fill under its sheen.
  "embedded-record/selected": {
    width: 320,
    // The widget takes the whole width of the row it's given, and sizes itself
    // from it.
    frame: "flex p-2",
    render: () => <EmbeddedRecord cells={["Beyoncé"]} focusable selected />,
  },
};

/** Fills the now-playing bar with a frozen transport — no audio element, no
 * stream request. The track is deliberately left unlocated (`rowIndex: null`,
 * so "Locate" reads disabled): where the playing row's marker is painted is the
 * results grid's business, and `playback.spec.ts` asserts it against canvas
 * pixels there. */
function seedPlayback(store: AppStore): void {
  const id = openLemonade(store);
  store.seedNowPlaying(
    {
      sourceTabId: id,
      id: "seeded-track",
      rowIndex: null,
      title: "Uncatena",
      artists: ["Sylvan Esso", "Nick Sanborn"],
    },
    { playing: true, position: 74, duration: 255, hasNext: true },
  );
}
