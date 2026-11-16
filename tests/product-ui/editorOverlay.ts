import type { Locator, Page } from "@playwright/test";

/** Floating controls are owned by an editor instance, outside its content DOM. */
export async function editorOverlay(page: Page, surface: Locator): Promise<Locator> {
  const owner = await surface.getAttribute("data-editor-instance");
  if (!owner) throw new Error("Missing editor instance identity");
  return page.locator(`[data-editor-overlay-owner=${JSON.stringify(owner)}]`);
}
