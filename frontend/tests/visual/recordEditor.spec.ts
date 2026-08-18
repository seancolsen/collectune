import { test, expect, type Locator, type Page } from "@playwright/test";
import { QUERIES_FIXTURE } from "../../src/dev/fixtures";
import type { AppStore } from "../../src/state/store";

// The record-editor entry path and everything the form does, behaviorally:
// right-click a result row → a DOM context menu offering one "Edit {table}" per
// table whose primary key the row carries → the record editor opens as a
// sidebar inside the query page. What the panel and its parts look like is
// covered component-by-component in `record.spec.ts`, through the harness.
//
// The rows are canvas pixels, so the menu and the sidebar are the only parts a
// locator can see; the row underneath is addressed by coordinates (rows start at
// the canvas's top edge, ~40px tall).

/** The store the app exposes under `?expose=1`. */
interface AppWindow {
  __appStore: AppStore;
}

async function mockRpc(page: Page) {
  await page.route("**/api/rpc", async (route) => {
    const body = route.request().postDataJSON() as {
      method: string;
      id: number;
    };
    const result =
      body.method === "query.list"
        ? QUERIES_FIXTURE
        : body.method === "preset.list"
          ? []
          : null;
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ jsonrpc: "2.0", result, id: body.id }),
    });
  });
  await page.route("**/api/query", (route) =>
    route.fulfill({ status: 200, contentType: "text/plain", body: "" }),
  );
}

/** The seeded grid, with rows carrying both a `track` and an `album` key, plus
 * the canned schema + record data the form is built and filled from (`?records=`
 * numbers rows from 1, and the fixture's ids match: row 0 is `track-1`). */
const SEEDED =
  "/?tabs=Lemonade&grid=lemonade&records=track,album&recordFixture=1&expose=1";

/** The same, with every record query held back — long enough to see the form
 * before its data lands. */
const slow = (ms: number) => `${SEEDED}&recordDelay=${ms}`;

async function openGrid(page: Page, url = SEEDED) {
  await mockRpc(page);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(url);
  await expect(page.locator("canvas[data-rows]")).toBeVisible();
}

/** Right-clicks a row of the results canvas (row 0 unless told otherwise). */
async function rightClickRow(page: Page, y = 20) {
  await page
    .locator("canvas")
    .click({ button: "right", position: { x: 200, y } });
}

/** The record-editor sidebar (the explorer is an `<aside>` too — match on the
 * accessible name). */
const editorPanel = (page: Page) =>
  page.getByRole("complementary", { name: /^Edit / });

/** The embedded records within the form — the previews of other rows. Only
 * those under a multi-record field are selectable; the one a scalar linked
 * record field shows is not. */
const embeddedRecords = (editor: Locator) => editor.locator(".rc-embedded");
const selectableRecords = (editor: Locator) =>
  editor.locator("[data-selectable]");

/** One form item, by the id the model tracks it under: `r` is the record the
 * editor was opened on, `r:title` its `title` field's label. */
const formItem = (editor: Locator, itemId: string) =>
  editor.locator(`[data-item-id="${itemId}"]`);

const selection = (page: Page) =>
  page.evaluate(() => {
    const store = (window as unknown as AppWindow).__appStore;
    return [...store.rowSelection(store.state.activeTabId!)];
  });

/** A y inside the nth seeded row. The grid lays rows out from its content, so
 * this is measured (~36px each) rather than declared — near enough the middle of
 * each of the five rows to be safe. */
const rowY = (index: number) => index * 36 + 18;

/** Opens the editor on a grid row through the context menu. */
async function openEditor(page: Page, table = "track", row = 0) {
  await rightClickRow(page, rowY(row));
  await page.getByRole("menuitem", { name: `Edit ${table}` }).click();
  await expect(editorPanel(page)).toBeVisible();
}

test("a row's context menu offers one entry per table it identifies", async ({
  page,
}) => {
  await openGrid(page);
  await rightClickRow(page);

  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Edit track",
    "Edit album",
  ]);
  // Right-clicking a row also selects it, so the menu's target is visible.
  expect(await selection(page)).toEqual([0]);
});

test("rows with no primary key offer no menu", async ({ page }) => {
  // Same grid, without the `records=` lineage stand-in.
  await openGrid(page, "/?tabs=Lemonade&grid=lemonade&expose=1");
  await rightClickRow(page);
  await expect(page.getByRole("menu")).toBeHidden();
});

test("choosing an entry opens the record editor on that record", async ({
  page,
}) => {
  await openGrid(page);
  await rightClickRow(page, 60); // row 1
  await page.getByRole("menuitem", { name: "Edit album" }).click();

  const editor = editorPanel(page);
  await expect(editor).toBeVisible();
  await expect(editor.getByRole("heading")).toHaveText("Edit album");
  // The seeded key value for row 1 (`?records=` numbers rows from 1).
  await expect(editor).toContainText("album-2");
  // Choosing an entry dismisses the menu.
  await expect(page.getByRole("menu")).toBeHidden();
});

test("the editor sidebar narrows the results, not the tab bar", async ({
  page,
}) => {
  await openGrid(page);
  const canvasWidth = async () =>
    (await page.locator("canvas").boundingBox())!.width;
  const tabBarWidth = async () =>
    (await page.getByTestId("tab-bar").boundingBox())!.width;

  const before = await canvasWidth();
  const tabsBefore = await tabBarWidth();
  await rightClickRow(page);
  await page.getByRole("menuitem", { name: "Edit track" }).click();
  await expect(editorPanel(page)).toBeVisible();

  const width = await page.evaluate(() =>
    (window as unknown as AppWindow).__appStore.recordSidebarWidth(),
  );
  expect(await canvasWidth()).toBeCloseTo(before - width, 0);
  // The tab bar spans the app frame, outside the query page, and keeps its width.
  expect(await tabBarWidth()).toBe(tabsBefore);
});

test("the menu blocks the rows beneath it", async ({ page }) => {
  await openGrid(page);
  // A 400×30 strip of canvas covering a row well below the menu's anchor.
  const strip = () =>
    page.evaluate(() => {
      const canvas = document.querySelector("canvas")!;
      const dpr = window.devicePixelRatio || 1;
      const ctx = canvas.getContext("2d")!;
      const d = ctx.getImageData(0, 88 * dpr, 400 * dpr, 30 * dpr).data;
      let sum = 0;
      for (let i = 0; i < d.length; i++) sum = (sum * 31 + d[i]) >>> 0;
      return sum;
    });

  await rightClickRow(page); // selects row 0, opens the menu
  expect(await selection(page)).toEqual([0]);
  const blocked = await strip();

  // Hovering a row while the menu is up leaves it unpainted — no hover state.
  await page.mouse.move(300, 100);
  await page.waitForTimeout(50);
  expect(await strip()).toBe(blocked);

  // Clicking that row only dismisses the menu: the blocking layer swallows the
  // click, so the row underneath is never selected.
  await page.mouse.click(300, 100);
  await expect(page.getByRole("menu")).toBeHidden();
  expect(await selection(page)).toEqual([0]);

  // With the menu gone the same hover *does* repaint — so the check above was
  // watching something real. (Wheel scrolling is gated by the same freeze.)
  await page.mouse.move(0, 300);
  await page.mouse.move(300, 100);
  await expect.poll(strip).not.toBe(blocked);
});

test("Escape closes the menu; the X closes the editor", async ({ page }) => {
  await openGrid(page);
  await rightClickRow(page);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toBeHidden();

  await rightClickRow(page);
  await page.getByRole("menuitem", { name: "Edit track" }).click();
  const editor = editorPanel(page);
  await expect(editor).toBeVisible();
  await editor.getByRole("button", { name: "Close record editor" }).click();
  await expect(editor).toBeHidden();
});

test("the sidebar is resizable, and its width persists", async ({ page }) => {
  await openGrid(page);
  await rightClickRow(page);
  await page.getByRole("menuitem", { name: "Edit track" }).click();
  const editor = editorPanel(page);
  await expect(editor).toBeVisible();
  const before = (await editor.boundingBox())!;

  // Drag the divider 120px to the left — the sidebar grows by that much.
  const divider = page.getByRole("separator", { name: "Resize record editor" });
  const grip = (await divider.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + 200);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2 - 120, grip.y + 200, {
    steps: 8,
  });
  await page.mouse.up();
  await expect
    .poll(async () => (await editor.boundingBox())!.width)
    .toBeCloseTo(before.width + 120, 0);

  // The released width is what a reload restores (it lives in localStorage).
  const stored = await page.evaluate(() =>
    localStorage.getItem("recordSidebarWidth"),
  );
  expect(Number(stored)).toBeCloseTo(before.width + 120, 0);
  await page.reload();
  await expect(page.locator("canvas[data-rows]")).toBeVisible();
  expect(
    await page.evaluate(() =>
      (window as unknown as AppWindow).__appStore.recordSidebarWidth(),
    ),
  ).toBeCloseTo(before.width + 120, 0);
});

// ── The form ────────────────────────────────────────────────────────────────
//
// Structure from introspection, data from the seeded fixture (`?recordFixture`),
// and everything below the top level fetched only once it's opened.

test("the form shows its field labels — and the id — before any data lands", async ({
  page,
}) => {
  await openGrid(page, slow(5000));
  await openEditor(page);
  const editor = editorPanel(page);

  // Every field of `track` is there: its own columns, then the tables that
  // reference it, alphabetically.
  await expect(editor.getByText("title", { exact: true })).toBeVisible();
  await expect(editor.getByText("credit", { exact: true })).toBeVisible();
  await expect(editor.getByText("play", { exact: true })).toBeVisible();
  // The key came with the row, so it renders; nothing else has arrived.
  await expect(editor.getByText("track-1")).toBeVisible();
  await expect(editor.getByText("Pray You Catch Me")).toBeHidden();
  // No pencils either — an unloaded field isn't known to be empty.
  await expect(editor.getByRole("button", { name: "Edit title" })).toBeHidden();
  // Nor anything to expand: the counts that say whether there's anything in
  // there haven't arrived.
  await expect(
    editor.getByRole("button", { name: "Expand credit" }),
  ).toBeHidden();
});

test("the form loads the record's values and its related-record counts", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await expect(
    editor.getByText("Pray You Catch Me", { exact: true }),
  ).toBeVisible();
  // The `file` link shows the record it points at, not the id it holds.
  await expect(embeddedRecords(editor).first()).toContainText(
    "Pray You Catch Me.flac",
  );
  await expect(editor.getByText("file-1")).toBeHidden();
  // `credit` and `play` show how many records reference this track.
  await expect(
    editor.getByRole("button", { name: "Expand credit" }),
  ).toBeVisible();
});

test("selecting another row rebuilds the form on that record", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);
  const title = (text: string) => editor.getByText(text, { exact: true });
  await expect(title("Pray You Catch Me")).toBeVisible();

  // The sidebar follows the selection, and the form follows the sidebar. Track
  // 3's title is the fixture's long one, matched by a fragment of its one-line
  // form rather than in full.
  await page.locator("canvas").click({ position: { x: 200, y: rowY(2) } });
  await expect(editor.getByText(/Live at the Superdome/)).toBeVisible();
  await expect(title("Pray You Catch Me")).toBeHidden();
});

// ── Several records at once ─────────────────────────────────────────────────
//
// Widening the result-row selection puts the editor on every record it covers.
// It's the same form: what the records agree on is editable as ever and the edit
// lands on all of them, what they disagree on reads "(varied)", and their child
// records wait for bulk modification proper.

/** Widens the row selection to rows 0…`through`, the sidebar following it. */
async function extendSelectionTo(page: Page, through: number) {
  await page
    .locator("canvas")
    .click({ position: { x: 200, y: rowY(through) }, modifiers: ["Shift"] });
}

test("widening the selection puts the editor on every record it covers", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page); // track-1
  const editor = editorPanel(page);
  await expect(
    editor.getByText("Pray You Catch Me", { exact: true }),
  ).toBeVisible();

  await extendSelectionTo(page, 1); // track-1 and track-2
  await expect(editor.getByRole("heading")).toHaveText("Edit 2 track records");

  // The titles differ, so there is no title to show — or to edit.
  await expect(editor.getByText("Pray You Catch Me")).toBeHidden();
  await expect(formItem(editor, "r:title")).toBeVisible();
  await expect(editor.getByText("(varied)").first()).toBeVisible();
  await formItem(editor, "r:title").dblclick();
  await expect(editor.getByRole("textbox")).toBeHidden();

  // Child records are not editable across records yet, so a multi-record field
  // says so in place of the ways into it.
  await expect(
    editor.getByText("Bulk modification not yet supported").first(),
  ).toBeVisible();
  await expect(editor.getByRole("button", { name: "Add credit" })).toBeHidden();
  await expect(
    editor.getByRole("button", { name: "Expand credit" }),
  ).toBeHidden();

  // Narrowing the selection back to one row is the single-record form again.
  await page.locator("canvas").click({ position: { x: 200, y: rowY(1) } });
  await expect(editor.getByRole("heading")).toHaveText("Edit track");
  await expect(editor.getByText("Hold Up", { exact: true })).toBeVisible();
});

test("editing a field the records share writes every one of them", async ({
  page,
}) => {
  await openGrid(page);
  const calls = await mockDml(page);
  await openEditor(page);
  const editor = editorPanel(page);
  await extendSelectionTo(page, 1);

  // Both tracks are on disc 1, so that field has a value to edit — and the
  // edit is made to both of them.
  await editor.getByText("1", { exact: true }).first().click();
  await editor.getByRole("textbox").fill("2");
  await page.keyboard.press("Escape");
  await expect(star(editor, "disc_number")).toBeVisible();

  await saveButton(editor).click();
  await expect(saveButton(editor)).toBeHidden();
  expect(calls[0]).toEqual([
    {
      operation: "update",
      id: "op1",
      table: "track",
      where: { id: "track-1" },
      values: { disc_number: "2" },
    },
    {
      operation: "update",
      id: "op2",
      table: "track",
      where: { id: "track-2" },
      values: { disc_number: "2" },
    },
  ]);
});

test("a NULL field offers a pencil, which activates an empty input", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 4); // track-5 has no track number
  const editor = editorPanel(page);

  const pencil = editor.getByRole("button", { name: "Edit track_number" });
  await expect(pencil).toBeVisible();
  await pencil.click();
  const input = editor.getByRole("textbox");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("");
});

test("clicking a value edits it; clicking away puts it back into view mode", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  const input = editor.getByRole("textbox");
  await expect(input).toBeFocused();
  await input.fill("Pray You Catch Me (Live)");
  // Clicking outside the box leaves edit mode, keeping what was typed.
  await editor.getByRole("heading").click();
  await expect(editor.getByRole("textbox")).toBeHidden();
  await expect(editor.getByText("Pray You Catch Me (Live)")).toBeVisible();
});

test("a text field too long for its line expands below its label", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  // Track 1's title fits on its line, so it has nothing to expand.
  await expect(
    editor.getByRole("button", { name: "Expand title" }),
  ).toBeHidden();

  // Track 3's — the long one — does.
  await openEditor(page, "track", 2);
  await editor.getByRole("button", { name: "Expand title" }).click();
  // Expanded, the value keeps its linebreaks instead of being flattened to one
  // ellipsized line.
  await expect(
    editor.getByText(/reprise of "Ring the Alarm" nobody had rehearsed/),
  ).toBeVisible();
});

test("expanding a multi-record field lists its records, each expandable in turn", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);

  await editor.getByRole("button", { name: "Expand credit" }).click();
  // Each credit is previewed by the columns that identify it at a glance, both
  // a hop away through a link: its artist's name, and — since migration 0003
  // made `role` a link to a table of unique names — its role. They're ordered by
  // `order`, which the same generation picked up.
  const credits = selectableRecords(editor);
  await expect(credits).toHaveCount(2);
  await expect(credits.first()).toHaveText("Beyoncé");
  await expect(credits.nth(1)).toContainText("Jack White");
  await expect(credits.nth(1)).toContainText("Featured");

  // Expanding one loads that record's own form, without the `track` field it was
  // reached through. "Featured" is on screen twice by then — in the preview
  // above and in the form below — so this asks for the form's own copy.
  await editor.getByRole("button", { name: /^Expand Jack White/ }).click();
  await expect(editor.getByText("role", { exact: true })).toBeVisible();
  await expect(editor.getByText("Featured").last()).toBeVisible();
  await expect(editor.getByText("track", { exact: true })).toBeHidden();
});

test("expanding a scalar linked record field loads that record's form", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await editor.getByRole("button", { name: "Expand file" }).click();
  await expect(editor.getByText("path", { exact: true })).toBeVisible();
  // Both the embedded record above and the `path` field within the sub-form now
  // show the file's path.
  await expect(editor.getByText(/Pray You Catch Me\.flac/)).toHaveCount(2);
  // The linked record's own referencing fields are there too — the tree recurses
  // as far as the data goes.
  await expect(
    editor.getByRole("button", { name: "Collapse file" }),
  ).toBeVisible();
});

test("a multi-record field skeletons its records while they load", async ({
  page,
}) => {
  await openGrid(page, slow(500));
  await openEditor(page);
  const editor = editorPanel(page);

  // The count arrived with the top-level load, so the list has its shape — two
  // empty embedded records — before any of the records themselves are here.
  await editor.getByRole("button", { name: "Expand play" }).click();
  await expect(editor.getByTestId("record-placeholder")).toHaveCount(2);
  await expect(editor.getByTestId("record-placeholder")).toHaveCount(0);
});

// ── Embedded records, focus and selection ───────────────────────────────────

test("a scalar linked record field previews the record it points at", async ({
  page,
}) => {
  await openGrid(page, slow(700));
  await openEditor(page);
  const editor = editorPanel(page);

  // The preview is a second request, made once the top-level load hands over the
  // id: the widget is there, empty, before its content is.
  const preview = embeddedRecords(editor).first();
  await expect(preview).toBeVisible();
  await expect(preview).toHaveText("");
  await expect(preview).toContainText("Pray You Catch Me.flac");
  // It isn't a selectable item — the field's label is what the user selects.
  await expect(preview).not.toHaveAttribute("data-selectable", "");
});

test("embedded records select like result rows, with Shift and Ctrl", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 4); // track-5: two credits
  const editor = editorPanel(page);
  await editor.getByRole("button", { name: "Expand credit" }).click();
  const credits = selectableRecords(editor);
  await expect(credits).toHaveCount(2);
  const selected = () =>
    editor.locator('[data-selectable][data-selected="true"]');

  await credits.first().click();
  await expect(selected()).toHaveCount(1);
  // A selected record is a focused one.
  await expect(credits.first()).toBeFocused();

  await credits.nth(1).click({ modifiers: ["Shift"] });
  await expect(selected()).toHaveCount(2);

  await credits.first().click({ modifiers: ["ControlOrMeta"] });
  await expect(selected()).toHaveCount(1);
  await expect(credits.nth(1)).toHaveAttribute("data-selected", "true");

  // Clicking anything that isn't a selectable record ends the selection.
  await editor.getByRole("heading").click();
  await expect(selected()).toHaveCount(0);
});

test("the arrow keys move between form items and open them", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);

  // Clicking a field label selects it; Down walks to the next field's label.
  await formItem(editor, "r:title").click();
  await expect(formItem(editor, "r:title")).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(formItem(editor, "r:album")).toBeFocused();

  // Right opens the focused field, Down then recurses into what it opened, and
  // Left closes it again.
  await formItem(editor, "r:#credit").click();
  await page.keyboard.press("ArrowRight");
  await expect(selectableRecords(editor)).toHaveCount(2);
  await page.keyboard.press("ArrowDown");
  await expect(selectableRecords(editor).first()).toBeFocused();
  // Arrowing onto an embedded record selects it, as clicking it would.
  await expect(selectableRecords(editor).first()).toHaveAttribute(
    "data-selected",
    "true",
  );

  await formItem(editor, "r:#credit").click();
  await page.keyboard.press("ArrowLeft");
  await expect(selectableRecords(editor)).toHaveCount(0);
});

test("Enter activates the focused label: edit, pick, or add, by field kind", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 4); // track-5 has no album
  const editor = editorPanel(page);

  // A primitive field: its input, focused.
  await formItem(editor, "r:title").click();
  await page.keyboard.press("Enter");
  await expect(editor.getByRole("textbox")).toBeFocused();
  await page.keyboard.press("Escape");

  // A scalar linked record field: the picker.
  await formItem(editor, "r:album").click();
  await page.keyboard.press("Enter");
  await expect(picker(page)).toBeVisible();
  await page.keyboard.press("Escape");

  // A multi-record field: a fresh child record, open and ready to type into —
  // unlike double-click, which only expands the field.
  await formItem(editor, "r:#credit").click();
  await page.keyboard.press("Enter");
  await expect(selectableRecords(editor).first()).toHaveText("New");
  await expect(editor.getByRole("textbox")).toBeFocused();
});

test("the X on a scalar linked record field's embedded record clears it", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page); // track-1 is already filed under album-1
  const editor = editorPanel(page);
  // Its three link fields: `album`, `file` and — since migration 0003 — `rating`.
  await expect(embeddedRecords(editor)).toHaveCount(3);

  await editor.getByRole("button", { name: "Clear album" }).click();
  await expect(embeddedRecords(editor)).toHaveCount(2);
  await expect(
    editor.getByRole("button", { name: "Edit album" }),
  ).toBeVisible();
  await expect(star(editor, "album")).toBeVisible();
});

test("the X on an embedded record within a multi-record field deletes it", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);
  await editor.getByRole("button", { name: "Expand credit" }).click();
  const credits = selectableRecords(editor);
  await expect(credits).toHaveCount(2);

  await editor.getByRole("button", { name: "Delete Beyoncé" }).click();
  await expect(credits).toHaveCount(1);
  await expect(credits.first()).toContainText("Jack White");
});

test("Delete clears the focused field, and drops a selected record", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);

  // A primitive field goes to NULL — in the form only, until it's saved — which
  // is what the pencil is offering to fill back in.
  await formItem(editor, "r:title").click();
  await page.keyboard.press("Delete");
  await expect(editor.getByText(/Live at the Superdome/)).toBeHidden();
  await expect(
    editor.getByRole("button", { name: "Edit title" }),
  ).toBeVisible();

  // A selected record within a multi-record field goes away, and the field's
  // count follows it down.
  await editor.getByRole("button", { name: "Expand credit" }).click();
  await selectableRecords(editor).first().click();
  await page.keyboard.press("Delete");
  await expect(selectableRecords(editor)).toHaveCount(1);
  await expect(selectableRecords(editor).first()).toContainText("Jack White");
});

test("Ctrl+Click on a toggle opens every field beside it", async ({ page }) => {
  await openGrid(page);
  await openEditor(page, "track", 2);
  const editor = editorPanel(page);

  await editor
    .getByRole("button", { name: "Expand credit" })
    .click({ modifiers: ["ControlOrMeta"] });
  // `credit` and its siblings `file`, `play` and `track_tag` all opened at
  // once.
  await expect(
    editor.getByRole("button", { name: "Collapse play" }),
  ).toBeVisible();
  await expect(
    editor.getByRole("button", { name: "Collapse file" }),
  ).toBeVisible();
});

test("Escape and Tab leave an activated field for a label", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await editor.getByRole("textbox").fill("Renamed");
  await page.keyboard.press("Escape");
  // The edit is kept in the form, and focus lands back on the field's label.
  await expect(editor.getByRole("textbox")).toBeHidden();
  await expect(editor.getByText("Renamed", { exact: true })).toBeVisible();
  await expect(formItem(editor, "r:title")).toBeFocused();

  // Tab out of an edit and focus moves on to the next item's label instead.
  await editor.getByText("Renamed", { exact: true }).click();
  await page.keyboard.press("Tab");
  await expect(formItem(editor, "r:album")).toBeFocused();
});

// ── Form modification ───────────────────────────────────────────────────────
//
// Every change here is ephemeral: it lives in the form (and, once the form is
// closed, in the tab) until the user saves.

/** The red ✱ marking one field — or one embedded record — as modified. */
const star = (editor: Locator, label: string) =>
  editor.locator(`[aria-label="${label} modified"]`);

test("typing into a field marks it modified as it's typed", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await expect(star(editor, "title")).toBeHidden();
  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await page.keyboard.type(" (Live)");
  // The star is there before the edit is committed — the form takes each
  // keystroke as it happens.
  await expect(star(editor, "title")).toBeVisible();
  await expect(editor.getByRole("textbox")).toBeFocused();

  // Typing it back to what it was is no longer a modification.
  for (let i = 0; i < " (Live)".length; i++)
    await page.keyboard.press("Backspace");
  await expect(star(editor, "title")).toBeHidden();
});

test("a change deep in the tree stars the collapsed field above it", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);

  await editor.getByRole("button", { name: "Expand credit" }).click();
  await editor.getByRole("button", { name: /^Expand Jack White/ }).click();
  // The primitive edited deep in the tree is the credit's `order`: migration
  // 0003 made `role` a link, whose label offers the record picker rather than a
  // text box. Enter on a focused label is the field's "do the thing" key.
  await formItem(editor, "r##credit[1]:order").click();
  await page.keyboard.press("Enter");
  await editor.getByRole("textbox").fill("42");
  await page.keyboard.press("Escape");

  // The edited record wears a star, and so does the field holding it.
  await expect(star(editor, "order")).toBeVisible();
  await expect(star(editor, "credit")).toBeVisible();

  // Collapsing hides the field it was made in, not the fact that it was made —
  // and the change itself survives being closed and opened again.
  await editor.getByRole("button", { name: "Collapse credit" }).click();
  await expect(star(editor, "credit")).toBeVisible();
  await expect(editor.getByText("42", { exact: true })).toBeHidden();
  // Reopening finds everything as it was left — the record still expanded, the
  // edit still in it.
  await editor.getByRole("button", { name: "Expand credit" }).click();
  await expect(editor.getByText("42", { exact: true })).toBeVisible();
});

test("unsaved changes stay with the record when another is edited", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await editor.getByRole("textbox").fill("Renamed");
  await page.keyboard.press("Escape");

  // Off to another record, and back: the form picks up where it was left.
  await page.locator("canvas").click({ position: { x: 200, y: rowY(2) } });
  await expect(editor.getByText(/Live at the Superdome/)).toBeVisible();
  await page.locator("canvas").click({ position: { x: 200, y: rowY(0) } });
  await expect(editor.getByText("Renamed", { exact: true })).toBeVisible();
  await expect(star(editor, "title")).toBeVisible();
});

test("a record with unsaved changes is marked in the results", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  // The left margin of row 0, where the ✱ is painted — no cell text reaches it.
  const gutter = () =>
    page.evaluate(() => {
      const canvas = document.querySelector("canvas")!;
      const dpr = window.devicePixelRatio || 1;
      const ctx = canvas.getContext("2d")!;
      const d = ctx.getImageData(0, 4 * dpr, 8 * dpr, 28 * dpr).data;
      let sum = 0;
      for (let i = 0; i < d.length; i++) sum = (sum * 31 + d[i]) >>> 0;
      return sum;
    });
  const clean = await gutter();

  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await editor.getByRole("textbox").fill("Renamed");
  await page.keyboard.press("Escape");
  await expect.poll(gutter).not.toBe(clean);
});

test("the + button scaffolds a new record, open and ready to type into", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);

  await editor.getByRole("button", { name: "Add credit" }).click();
  const credits = selectableRecords(editor);
  await expect(credits).toHaveCount(3);
  // It goes in at the top, previews as "New" (it has no values to preview yet),
  // and the caret is already in its first editable field.
  await expect(credits.first()).toHaveText("New");
  await expect(editor.getByRole("textbox")).toBeFocused();
  await page.keyboard.type("3");

  // Its own form is open below it, and the field it was added to is starred and
  // counts it.
  await expect(editor.getByText("role", { exact: true })).toBeVisible();
  await expect(star(editor, "credit")).toBeVisible();
  await expect(editor.getByText("3", { exact: true }).first()).toBeVisible();
});

// ── Context menus ───────────────────────────────────────────────────────────

/** Right-clicks one of the form's items. */
async function rightClick(target: Locator) {
  await target.click({ button: "right" });
}

test("a primitive field's menu edits, clears and copies it", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await rightClick(formItem(editor, "r:title"));
  const menu = page.getByRole("menu");
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Edit",
    "Clear",
    "Copy",
  ]);
  // The label says what the menu is about to act on, as if it were focused: it
  // wears the same blue border a focused label would, instead of its usual
  // transparent one.
  await expect(formItem(editor, "r:title")).not.toHaveCSS(
    "border-top-color",
    "rgba(0, 0, 0, 0)",
  );

  await menu.getByRole("menuitem", { name: "Clear" }).click();
  await expect(
    editor.getByText("Pray You Catch Me", { exact: true }),
  ).toBeHidden();
  await expect(star(editor, "title")).toBeVisible();
  // Clearing a field is exactly what the pencil offers to undo.
  const pencil = editor.getByRole("button", { name: "Edit title" });
  await expect(pencil).toBeVisible();

  // The value itself carries the same menu as the label.
  await pencil.click();
  await editor.getByRole("textbox").fill("Retitled");
  await page.keyboard.press("Escape");
  await rightClick(editor.getByText("Retitled", { exact: true }));
  await expect(page.getByRole("menu").getByRole("menuitem")).toHaveText([
    "Edit",
    "Clear",
    "Copy",
  ]);
  await page.getByRole("menuitem", { name: "Edit" }).click();
  await expect(editor.getByRole("textbox")).toBeFocused();
});

test("a field menu traps focus and Up/Down/Enter drive it, not the form underneath", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await rightClick(formItem(editor, "r:title"));
  const menu = page.getByRole("menu");
  const items = menu.getByRole("menuitem");
  await expect(items).toHaveText(["Edit", "Clear", "Copy"]);

  // Opening moves real focus into the menu — off the label it was raised on,
  // even though the label keeps its focus-styled border (spec: the label says
  // what the menu is about to act on).
  await expect(items.first()).toBeFocused();
  await expect(formItem(editor, "r:title")).not.toBeFocused();

  // Down highlights the next row — and does *not* also walk the form to the
  // next field, which is what bare Down does everywhere else in the editor.
  await page.keyboard.press("ArrowDown");
  await expect(items.nth(1)).toBeFocused();
  await expect(formItem(editor, "r:album")).not.toBeFocused();

  // Up from the top wraps to the bottom.
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("ArrowUp");
  await expect(items.last()).toBeFocused();

  // The focus trap: Tab cycles within the menu instead of leaving it.
  await page.keyboard.press("Tab");
  await expect(items.first()).toBeFocused();

  // Enter picks the highlighted row, just as clicking it would.
  await page.keyboard.press("ArrowDown");
  await expect(items.nth(1)).toBeFocused(); // "Clear"
  await page.keyboard.press("Enter");
  await expect(menu).toBeHidden();
  await expect(
    editor.getByText("Pray You Catch Me", { exact: true }),
  ).toBeHidden();
  // Closing hands focus back to the label the menu was raised on.
  await expect(formItem(editor, "r:title")).toBeFocused();
});

test("a primary key's menu only offers to copy it, never to edit or clear it", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  // `id` is `track`'s primary key — issued by the database, not the user.
  await rightClick(formItem(editor, "r:id"));
  await expect(page.getByRole("menu").getByRole("menuitem")).toHaveText([
    "Copy",
  ]);
  await page.keyboard.press("Escape");

  // Nor does clicking the value itself, or double-clicking its label, open an
  // editor for it — unlike an ordinary field.
  await editor.getByText("track-1", { exact: true }).click();
  await expect(editor.getByRole("textbox")).toBeHidden();
  await formItem(editor, "r:id").dblclick();
  await expect(editor.getByRole("textbox")).toBeHidden();
});

test("a multi-record field's menu adds and deletes records", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);

  await rightClick(formItem(editor, "r:#credit"));
  await expect(page.getByRole("menu").getByRole("menuitem")).toHaveText([
    "New record",
    "Delete all records",
  ]);
  await page.getByRole("menuitem", { name: "New record" }).click();
  await expect(selectableRecords(editor).first()).toHaveText("New");

  await rightClick(formItem(editor, "r:#credit"));
  await page.getByRole("menuitem", { name: "Delete all records" }).click();
  await expect(selectableRecords(editor)).toHaveCount(0);
  await expect(star(editor, "credit")).toBeVisible();
  // With nothing left in it, the field can't be expanded any more.
  await expect(
    editor.getByRole("button", { name: "Expand credit" }),
  ).toBeHidden();
});

test("a scalar linked record field's menu enters a new record", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);

  await rightClick(formItem(editor, "r:album"));
  const menu = page.getByRole("menu");
  await expect(menu.getByRole("menuitem")).toHaveText([
    "Pick a record",
    "Enter a new record",
    "Clear",
  ]);
  await menu.getByRole("menuitem", { name: "Enter a new record" }).click();

  // The field points at a record being created: its preview says so, and its
  // form is open below with the caret in the first field the user fills in.
  await expect(embeddedRecords(editor).nth(1)).toHaveText("New");
  await expect(editor.getByText("year", { exact: true })).toBeVisible();
  await expect(editor.getByRole("textbox")).toBeFocused();
  await expect(star(editor, "album")).toBeVisible();
});

test("an embedded record's menu deletes it — or the whole selection", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);
  await editor.getByRole("button", { name: "Expand credit" }).click();
  const credits = selectableRecords(editor);
  await expect(credits).toHaveCount(2);

  // Right-clicking outside the selection moves it there first, as the result
  // rows do.
  await rightClick(credits.nth(1));
  await expect(credits.nth(1)).toHaveAttribute("data-selected", "true");
  await expect(page.getByRole("menu").getByRole("menuitem")).toHaveText([
    "Delete",
  ]);
  await page.getByRole("menuitem", { name: "Delete" }).click();
  await expect(credits).toHaveCount(1);
  await expect(credits.first()).toHaveText("Beyoncé");
});

// ── The modal record picker ─────────────────────────────────────────────────
//
// How a scalar linked record field is pointed at a record that already exists:
// a dialog with a Querydown search box that runs as it's typed, and results
// rendered as the embedded records they're about to become.

/** The open record picker. */
const picker = (page: Page) => page.getByRole("dialog");
/** Its results, painted to a canvas — the query page's own grid engine, reused
 * (see `RecordPicker.tsx`). Not queryable DOM, so tests read the grid's
 * readiness/row-count marker (`data-rows`) instead of row text, and click or
 * keyboard it the same way the main grid's own tests do. */
const pickerCanvas = (page: Page) => picker(page).getByTestId("picker-results");

/** Types into the picker's search box, which queries as it goes. */
async function searchPicker(page: Page, text: string) {
  await picker(page)
    .getByPlaceholder(/^Filter /)
    .fill(text);
}

test("the pencil on a NULL linked field opens the picker", async ({ page }) => {
  await openGrid(page);
  await openEditor(page, "track", 4); // track-5 has no album
  const editor = editorPanel(page);

  await editor.getByRole("button", { name: "Edit album" }).click();
  await expect(picker(page)).toBeVisible();
  await expect(picker(page).getByRole("heading")).toHaveText("Pick album");
  // Every album, previewed by the columns the points-based generator picked.
  await expect(pickerCanvas(page)).toHaveAttribute("data-rows", "5");
  // The first result starts highlighted; picking it straight from the search
  // box proves it's the album the generator ordered first.
  await page.keyboard.press("Enter");
  await expect(embeddedRecords(editor).nth(1)).toContainText("Lemonade");
});

test("double-clicking a linked field's label opens the picker, filled or not", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page); // track-1 is already filed under album-1
  const editor = editorPanel(page);

  await formItem(editor, "r:album").dblclick();
  await expect(picker(page)).toBeVisible();
  await expect(picker(page).getByRole("heading")).toHaveText("Pick album");
  await page.keyboard.press("Escape");

  // The field's own context menu is the third way in, and takes the keyboard
  // from the menu it was chosen in.
  await rightClick(formItem(editor, "r:album"));
  await page.getByRole("menuitem", { name: "Pick a record" }).click();
  await expect(page.getByRole("menu")).toBeHidden();
  await expect(picker(page).getByRole("heading")).toHaveText("Pick album");
});

test("the picker searches as the user types, and picking links the record", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2); // track-3: credits by Beyoncé and Jack White
  const editor = editorPanel(page);
  await editor.getByRole("button", { name: "Expand credit" }).click();
  await editor.getByRole("button", { name: "Expand Jack White" }).click();

  // The `artist` field of the second credit.
  await formItem(editor, "r##credit[1]:artist").dblclick();
  await expect(picker(page).getByRole("heading")).toHaveText("Pick artist");
  await expect(pickerCanvas(page)).toHaveAttribute("data-rows", "3");

  // No Ctrl+Enter: the query re-runs off the keystrokes themselves.
  await searchPicker(page, "name:Weeknd");
  await expect(pickerCanvas(page)).toHaveAttribute("data-rows", "1");

  // Clicking a result closes the picker and hands the whole loaded record to
  // the form — the embedded record is filled in without another request.
  await pickerCanvas(page).click({ position: { x: 100, y: 15 } });
  await expect(picker(page)).toBeHidden();
  await expect(editor.getByText("The Weeknd")).toBeVisible();
  await expect(star(editor, "artist")).toBeVisible();
  // Focus goes back to the field the picker was opened from.
  await expect(formItem(editor, "r##credit[1]:artist")).toBeFocused();
});

test("Up/Down and Enter drive the results without leaving the search box", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 2);
  const editor = editorPanel(page);
  await editor.getByRole("button", { name: "Expand credit" }).click();
  await editor.getByRole("button", { name: "Expand Beyoncé" }).click();
  await formItem(editor, "r##credit[0]:artist").dblclick();

  const search = picker(page).getByPlaceholder(/^Filter /);
  await expect(search).toBeFocused();
  await expect(pickerCanvas(page)).toHaveAttribute("data-rows", "3");
  // The first result starts highlighted; Down walks the list without the caret
  // ever leaving the search box. Landing two presses later on the third (and
  // last) result is what the Enter below proves — the highlight itself is
  // canvas paint, not DOM a locator can read.
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(search).toBeFocused();

  await page.keyboard.press("Enter");
  await expect(picker(page)).toBeHidden();
  await expect(editor.getByText("The Weeknd")).toBeVisible();
});

test("closing the picker leaves the field as it was", async ({ page }) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);
  await expect(embeddedRecords(editor).nth(1)).toContainText("Lemonade");

  await formItem(editor, "r:album").dblclick();
  await picker(page)
    .getByRole("button", { name: "Close record picker" })
    .click();
  await expect(picker(page)).toBeHidden();
  await expect(embeddedRecords(editor).nth(1)).toContainText("Lemonade");
  await expect(star(editor, "album")).toBeHidden();

  // Escape cancels it too.
  await formItem(editor, "r:album").dblclick();
  await expect(picker(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(picker(page)).toBeHidden();
  await expect(star(editor, "album")).toBeHidden();
});

test("the picker's sort and display builders re-run the query", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 4);
  const editor = editorPanel(page);
  await editor.getByRole("button", { name: "Edit album" }).click();

  // Both builders open pre-filled with what the generator chose for `album`.
  await picker(page).getByRole("button", { name: "Display" }).click();
  const display = picker(page).getByPlaceholder("Display");
  await expect(display).toHaveValue("$id $title");
  await picker(page).getByRole("button", { name: "Sort" }).click();
  await expect(picker(page).getByPlaceholder("Sort")).toHaveValue(
    "\\\\id \\\\title",
  );
  // Only one section is open at a time, as in the toolbar.
  await expect(display).toBeHidden();

  // Overruling the display changes what the results show — read off the
  // canvas's own pixels (there's no row text a locator can read) rather than
  // asserted directly, the same way `reload.spec.ts` proves a repaint.
  await picker(page).getByRole("button", { name: "Display" }).click();
  const before = await pickerCanvas(page).screenshot();
  await display.fill("$year");
  await expect
    .poll(async () =>
      Buffer.compare(before, await pickerCanvas(page).screenshot()),
    )
    .not.toBe(0);
});

test("the picker's New record button scaffolds one, seeded with the search", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page, "track", 4); // track-5 has no album
  const editor = editorPanel(page);

  await editor.getByRole("button", { name: "Edit album" }).click();
  await searchPicker(page, "Renaissance");
  await picker(page).getByRole("button", { name: "New record" }).click();
  await expect(picker(page)).toBeHidden();

  // The field points at a record being created, its form open below with the
  // search text already in the first text field and the caret after it.
  await expect(embeddedRecords(editor).nth(1)).toHaveText("New");
  const input = editor.getByRole("textbox");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("Renaissance");
  await expect(star(editor, "album")).toBeVisible();
});

// ── Saving ──────────────────────────────────────────────────────────────────
//
// One DML request per Save, carrying everything the user changed; the answer
// becomes the form's new baseline.

/** A DML operation as the form sends it (the fields these tests read). */
interface DmlOp {
  operation: "insert" | "update" | "delete";
  id: string;
  table: string;
  where?: Record<string, unknown>;
  values?: Record<string, unknown>;
}

/** Answers the `dml` RPC — as the database would, or with `error` — and collects
 * every request the form makes. Registered after `mockRpc`, so it takes
 * precedence and hands everything else back to it. */
async function mockDml(page: Page, error?: string): Promise<DmlOp[][]> {
  const calls: DmlOp[][] = [];
  await page.route("**/api/rpc", async (route) => {
    const body = route.request().postDataJSON() as {
      method: string;
      id: number;
      params: { operations: DmlOp[] };
    };
    if (body.method !== "dml") return route.fallback();
    calls.push(body.params.operations);
    // The row an operation returns: what it wrote, plus the identity it wrote
    // it under (an insert's id is the database's to issue).
    const result = Object.fromEntries(
      body.params.operations
        .filter((op) => op.operation !== "delete")
        .map((op) => [
          op.id,
          { id: `saved-${op.id}`, ...op.where, ...op.values },
        ]),
    );
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify(
        error
          ? { jsonrpc: "2.0", error: { code: 1, message: error }, id: body.id }
          : { jsonrpc: "2.0", result, id: body.id },
      ),
    });
  });
  return calls;
}

const saveButton = (editor: Locator) =>
  editor.getByRole("button", { name: "Save" });

test("Save writes the form's changes, and the form stops being modified", async ({
  page,
}) => {
  await openGrid(page);
  const calls = await mockDml(page);
  await openEditor(page);
  const editor = editorPanel(page);

  // Nothing to save until something has changed.
  await expect(saveButton(editor)).toBeHidden();
  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await editor.getByRole("textbox").fill("Renamed");
  await page.keyboard.press("Escape");
  await expect(saveButton(editor)).toBeEnabled();

  await saveButton(editor).click();
  // Saved: nothing modified any more, so the button goes with it.
  await expect(saveButton(editor)).toBeHidden();
  // One request, carrying only what changed, keyed on the record being edited.
  expect(calls).toHaveLength(1);
  expect(calls[0]).toEqual([
    {
      operation: "update",
      id: "op1",
      table: "track",
      where: { id: "track-1" },
      values: { title: "Renamed" },
    },
  ]);
  // The form keeps the record open on what it just saved — no longer modified.
  await expect(editor.getByText("Renamed", { exact: true })).toBeVisible();
  await expect(star(editor, "title")).toBeHidden();
});

test("Reset discards unsaved changes and reloads the record", async ({
  page,
}) => {
  await openGrid(page);
  await openEditor(page);
  const editor = editorPanel(page);
  const resetButton = () => editor.getByRole("button", { name: "Reset" });

  // Live only alongside Save — nothing to reset until something has changed.
  await expect(resetButton()).toBeHidden();
  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await editor.getByRole("textbox").fill("Renamed");
  await page.keyboard.press("Escape");
  await expect(resetButton()).toBeVisible();

  await resetButton().click();
  await expect(
    editor.getByText("Pray You Catch Me", { exact: true }),
  ).toBeVisible();
  await expect(editor.getByText("Renamed", { exact: true })).toBeHidden();
  await expect(star(editor, "title")).toBeHidden();
  await expect(resetButton()).toBeHidden();
  await expect(saveButton(editor)).toBeHidden();
});

test("one request carries the deletes, then the inserts", async ({ page }) => {
  await openGrid(page);
  const calls = await mockDml(page);
  await openEditor(page, "track", 2); // track-3: two credits
  const editor = editorPanel(page);

  await editor.getByRole("button", { name: "Expand credit" }).click();
  await selectableRecords(editor).first().click(); // Beyoncé
  await page.keyboard.press("Delete");
  await editor.getByRole("button", { name: "Add credit" }).click();
  await expect(editor.getByRole("textbox")).toBeFocused();
  await page.keyboard.type("3");
  await page.keyboard.press("Escape");

  await saveButton(editor).click();
  await expect(saveButton(editor)).toBeHidden();
  // The record on its way out goes first — the API never cascades, and a new
  // record could otherwise collide with the one it replaces.
  expect(calls[0]).toEqual([
    {
      operation: "delete",
      id: "op1",
      table: "credit",
      where: { track: "track-3", artist: "artist-1" },
    },
    {
      operation: "insert",
      id: "op2",
      table: "credit",
      // `track` is the field the list is filtered on: hidden in the form,
      // supplied by the save.
      values: { order: "3", track: "track-3" },
    },
  ]);
});

test("clearing a field the user never opened still deletes its records", async ({
  page,
}) => {
  await openGrid(page);
  const calls = await mockDml(page);
  await openEditor(page, "track", 2); // track-3: two credits, never expanded
  const editor = editorPanel(page);

  await rightClick(formItem(editor, "r:#credit"));
  await page.getByRole("menuitem", { name: "Delete all records" }).click();
  await saveButton(editor).click();
  await expect(saveButton(editor)).toBeHidden();

  // The form fetched the keys of the records it was told to delete, and the
  // save waited for them.
  expect(calls[0]).toEqual([
    {
      operation: "delete",
      id: "op1",
      table: "credit",
      where: { track: "track-3", artist: "artist-1" },
    },
    {
      operation: "delete",
      id: "op2",
      table: "credit",
      where: { track: "track-3", artist: "artist-2" },
    },
  ]);
});

test("a failed save says why, and keeps the changes", async ({ page }) => {
  await openGrid(page);
  await mockDml(page, 'Duplicate key "title" violates unique constraint.');
  await openEditor(page);
  const editor = editorPanel(page);

  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await editor.getByRole("textbox").fill("Renamed");
  await page.keyboard.press("Escape");
  await saveButton(editor).click();

  await expect(editor.getByRole("alert")).toContainText(
    "violates unique constraint",
  );
  // Nothing was written, so nothing is cleared: the edit — and the way to try
  // again — are both still there.
  await expect(editor.getByText("Renamed", { exact: true })).toBeVisible();
  await expect(star(editor, "title")).toBeVisible();
  await expect(saveButton(editor)).toBeEnabled();
});

test("the X on a failed save's error dismisses it, keeping the changes", async ({
  page,
}) => {
  await openGrid(page);
  await mockDml(page, 'Duplicate key "title" violates unique constraint.');
  await openEditor(page);
  const editor = editorPanel(page);

  await editor.getByText("Pray You Catch Me", { exact: true }).click();
  await editor.getByRole("textbox").fill("Renamed");
  await page.keyboard.press("Escape");
  await saveButton(editor).click();
  await expect(editor.getByRole("alert")).toBeVisible();

  await editor.getByRole("button", { name: "Dismiss error" }).click();
  await expect(editor.getByRole("alert")).toBeHidden();
  await expect(editor.getByText("Renamed", { exact: true })).toBeVisible();
  await expect(saveButton(editor)).toBeEnabled();
});
