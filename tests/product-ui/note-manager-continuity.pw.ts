import {expect, test} from "@playwright/test";

for (const lifecycle of ["hidden", "unmounted"] as const) test(`Core note survives real manager ${lifecycle}/reopen with the same owner`, async ({page}) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.route("**/__note_manager_continuity.html", route => route.fulfill({contentType: "text/html", body: "<!doctype html><html><body></body></html>"}));
  await page.goto("/__note_manager_continuity.html");
  await page.evaluate(() => import("/@id/__x00__@vitejs/plugin-react/preamble"));
  const result = await page.evaluate(async lifecycle => {
    const {verifyNoteManagerContinuity} = await import("/integration/web/src/app/notes/NoteManagerContinuity.browser.tsx");
    return verifyNoteManagerContinuity(lifecycle);
  }, lifecycle);
  expect(result).toMatchObject({lifecycle, managerRetained: lifecycle === "hidden", editorRetained: true, ownerRetained: true});
  expect(result.historyCount).toBeGreaterThan(0);
  expect(result.revision).toBeGreaterThan(1);
  expect(errors).toEqual([]);
});
