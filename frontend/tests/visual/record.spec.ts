import { test, expect, type Locator } from "@playwright/test";
import { SCHEMES, openStory, snapshot } from "./harness";

// The record editor, on its own: the panel as its form lands and opened out,
// mid-edit and after a refused save; the modal picker it opens; and the
// embedded-record widget, straight from cells.

/** The red ✱ marking one field — or one embedded record — as modified. */
const star = (panel: Locator, label: string) =>
  panel.locator(`[aria-label="${label} modified"]`);

for (const colorScheme of SCHEMES) {
  // As the form lands: every field of `track`, values and counts loaded,
  // everything collapsed.
  test(`record-editor/items-collapsed - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(
      page,
      "record-editor/items-collapsed",
      colorScheme,
    );
    await expect(
      stage.getByText("Pray You Catch Me", { exact: true }),
    ).toBeVisible();
    // The `file` link's embedded record is a second request; wait for it, so
    // the widget is filled rather than caught empty.
    await expect(stage.locator(".rc-embedded").first()).toContainText(".flac");
    await expect(stage).toHaveScreenshot(
      snapshot("record-editor/items-collapsed", colorScheme),
    );
  });

  // …and opened out: a long text field expanded below its label, a
  // multi-record field listing its records, and one of those expanded into its
  // own form.
  test(`record-editor/items-expanded - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(
      page,
      "record-editor/items-expanded",
      colorScheme,
    );
    await stage.getByRole("button", { name: "Expand title" }).click();
    await stage.getByRole("button", { name: "Expand credit" }).click();
    await stage.getByRole("button", { name: /^Expand Jack White/ }).click();
    // "Featured" is on screen twice by now — in the credit's own preview and in
    // the form it just opened — so this waits on the form's copy.
    await expect(stage.getByText("Featured").last()).toBeVisible();
    await expect(stage).toHaveScreenshot(
      snapshot("record-editor/items-expanded", colorScheme),
    );
  });

  // Mid-edit: an edited field and a record being created under `credit`, each
  // starred, and the "+" buttons that put it there.
  test(`record-editor/modified - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(page, "record-editor/modified", colorScheme);
    await stage.getByText("6 Inch", { exact: true }).click();
    await stage.getByRole("textbox").fill("6 Inch (Live)");
    await page.keyboard.press("Escape");
    await stage.getByRole("button", { name: "Add credit" }).click();
    await expect(stage.locator("[data-selectable]").first()).toHaveText("New");
    await expect(stage.getByRole("textbox")).toBeFocused();
    await expect(stage).toHaveScreenshot(
      snapshot("record-editor/modified", colorScheme),
    );
  });

  // A save the database refused: what it said, above a form still holding the
  // change it couldn't write.
  test(`record-editor/save-error - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(
      page,
      "record-editor/save-error",
      colorScheme,
    );
    await stage.getByText("Pray You Catch Me", { exact: true }).click();
    await stage.getByRole("textbox").fill("Sorry");
    await page.keyboard.press("Escape");
    await expect(star(stage, "title")).toBeVisible();
    await stage.getByRole("button", { name: "Save" }).click();
    await expect(stage.getByRole("alert")).toBeVisible();
    await expect(stage).toHaveScreenshot(
      snapshot("record-editor/save-error", colorScheme),
    );
  });

  // The editor on two records at once: the same form, showing what they agree
  // on and "(varied)" where they don't, and — under the multi-record fields —
  // the bulk modification that's still to come.
  test(`record-editor/bulk - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(page, "record-editor/bulk", colorScheme);
    await expect(stage.getByRole("heading")).toHaveText("Edit 2 track records");
    // Both records' data has landed once the counts they share have — one per
    // multi-record field, of which `track` has three (`credit`, `play` and, as
    // of migration 0003, `track_tag`).
    await expect(
      stage.getByText("Bulk modification not yet supported"),
    ).toHaveCount(3);
    await expect(stage).toHaveScreenshot(
      snapshot("record-editor/bulk", colorScheme),
    );
  });

  // The modal picker on its own: the search box with its sort and display
  // buttons, and the results as the embedded records they're about to become.
  // Shot through the dialog (it portals out of the stage).
  test(`record-picker/basic - ${colorScheme}`, async ({ page }) => {
    await openStory(page, "record-picker/basic", colorScheme);
    const picker = page.getByRole("dialog");
    await expect(picker.getByTestId("picker-results")).toHaveAttribute(
      "data-rows",
      "3",
    );
    await expect(picker).toHaveScreenshot(
      snapshot("record-picker/basic", colorScheme),
    );
  });

  // One preview widget, from cells alone, in its selected state.
  test(`embedded-record/selected - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(
      page,
      "embedded-record/selected",
      colorScheme,
    );
    await expect(stage.locator(".rc-embedded")).toHaveAttribute(
      "data-selected",
      "true",
    );
    await expect(stage).toHaveScreenshot(
      snapshot("embedded-record/selected", colorScheme),
    );
  });
}
