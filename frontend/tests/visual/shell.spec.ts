import { test, expect } from "@playwright/test";
import { SCHEMES, openStory, snapshot } from "./harness";

// The app's furniture, each piece on its own: the left sidebar in both of its
// layouts, the explorer that fills it, the settings menu with the prelude and
// rebind dialogs it raises,
// the client-update bar and the About dialog, the now-playing bar and its menu,
// and the command palette.

for (const colorScheme of SCHEMES) {
  // Wide enough for a persistent column beside the content.
  test(`sidebar-left/open-persistent - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(
      page,
      "sidebar-left/open-persistent",
      colorScheme,
      { width: 1280, height: 800 },
    );
    await expect(stage).toHaveScreenshot(
      snapshot("sidebar-left/open-persistent", colorScheme),
    );
  });

  // Narrower than PERSISTENT_MIN_WIDTH: a modal drawer over the content, behind
  // a dimming scrim.
  test(`sidebar-left/open-ephemeral - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(
      page,
      "sidebar-left/open-ephemeral",
      colorScheme,
      { width: 390, height: 844 },
    );
    await expect(stage).toHaveScreenshot(
      snapshot("sidebar-left/open-ephemeral", colorScheme),
    );
  });

  // The explorer's own contents: the "Opened" list (first tab active), the
  // saved-query list under its filter, and the Settings footer.
  test(`explorer/basic - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(page, "explorer/basic", colorScheme);
    await expect(stage.getByText("Workout Mix")).toBeVisible();
    await expect(stage).toHaveScreenshot(
      snapshot("explorer/basic", colorScheme),
    );
  });

  test(`settings/menu - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(page, "settings/menu", colorScheme);
    await expect(
      page.getByRole("menuitem", { name: "Keyboard shortcuts" }),
    ).toBeVisible();
    await expect(stage).toHaveScreenshot(
      snapshot("settings/menu", colorScheme),
    );
  });

  // The Querydown prelude in its setting editor, at the default value: the
  // monospace field and the disabled "Reset to default" it opens with. Shot
  // through the dialog — it portals out of the stage.
  test(`settings/prelude - ${colorScheme}`, async ({ page }) => {
    await openStory(page, "settings/prelude", colorScheme);
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("button", { name: "Save" })).toBeDisabled();
    await expect(dialog).toHaveScreenshot(
      snapshot("settings/prelude", colorScheme),
    );
  });

  // The rebind dialog, holding a chord another command already owns — so the
  // "currently bound to" warning shows too. Shot through the dialog: it renders
  // itself through a Portal, outside the stage.
  test(`settings/keyboard-shortcuts/modal-assign - ${colorScheme}`, async ({
    page,
  }) => {
    await openStory(
      page,
      "settings/keyboard-shortcuts/modal-assign",
      colorScheme,
    );
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByText(/Currently bound to/)).toBeVisible();
    await expect(dialog).toHaveScreenshot(
      snapshot("settings/keyboard-shortcuts/modal-assign", colorScheme),
    );
  });

  // The client-update bar in both of its notices: the dismissible "ready" one
  // over the persistent "stale" one.
  test(`update/banner - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(page, "update/banner", colorScheme);
    await expect(
      page.getByRole("button", { name: "Reload", exact: true }),
    ).toBeVisible();
    await expect(stage).toHaveScreenshot(
      snapshot("update/banner", colorScheme),
    );
  });

  // The About dialog on a mismatched pair of build ids, so the "didn't come
  // from the server that's running" verdict shows. Shot through the dialog: it
  // portals out of the stage.
  test(`about/modal - ${colorScheme}`, async ({ page }) => {
    await openStory(page, "about/modal", colorScheme);
    const dialog = page.getByRole("dialog");
    await expect(dialog.getByText(/didn’t come from the server/)).toBeVisible();
    await expect(dialog).toHaveScreenshot(snapshot("about/modal", colorScheme));
  });

  // The now-playing bar: title + artists on the left, play/pause and the
  // overflow trigger on the right, progress timeline across the bottom.
  test(`now-playing/playing - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(page, "now-playing/playing", colorScheme);
    await expect(page.getByTestId("now-playing")).toBeVisible();
    await expect(stage).toHaveScreenshot(
      snapshot("now-playing/playing", colorScheme),
    );
  });

  // Its overflow menu on its own — Next / Close / Locate, the last disabled
  // because the seeded track isn't located in any result set.
  test(`now-playing/menu - ${colorScheme}`, async ({ page }) => {
    const stage = await openStory(page, "now-playing/menu", colorScheme);
    await expect(page.getByRole("menuitem", { name: "Locate" })).toBeVisible();
    await expect(stage).toHaveScreenshot(
      snapshot("now-playing/menu", colorScheme),
    );
  });

  // The palette filtered by a typed query, which also drops the non-matching
  // commands. Shot through the dialog (it portals out of the stage).
  test(`command-palette/filtered - ${colorScheme}`, async ({ page }) => {
    await openStory(page, "command-palette/filtered", colorScheme);
    await expect(
      page.getByRole("option", { name: "Tabs: Save active tab" }),
    ).toBeVisible();
    await expect(page.getByTestId("command-palette")).toHaveScreenshot(
      snapshot("command-palette/filtered", colorScheme),
    );
  });
}
