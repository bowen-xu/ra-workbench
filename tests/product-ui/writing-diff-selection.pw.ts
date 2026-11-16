import { expect, test, type Locator } from "@playwright/test";
import type { WritingDiagnosticEvent } from "../../../plugins/ra-writing-projects/src/features/writing/persistence/WritingStateDiagnostics";

/** Measure a visible word, then exercise selection with real pointer events. */
async function wordBounds(line: Locator, word: string) {
  await line.scrollIntoViewIfNeeded();
  return line.evaluate((element, needle) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      const offset = node.textContent?.indexOf(needle) ?? -1;
      if (offset < 0) continue;
      const range = document.createRange();
      range.setStart(node, offset);
      range.setEnd(node, offset + needle.length);
      const bounds = range.getBoundingClientRect();
      return { left: bounds.left, right: bounds.right, y: bounds.top + bounds.height / 2 };
    }
    throw new Error(`Missing visible word: ${needle}`);
  }, word);
}

test("deleted Diff text can be selected and copied while edits and confirmation preserve its baseline", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const errors: string[] = [];
  const diagnosticReads: Promise<WritingDiagnosticEvent>[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.text().startsWith("[writing-state]")) diagnosticReads.push(message.args()[1]!.jsonValue());
  });
  await page.goto("/?demo=writing-editor-diff");
  const host = page.locator(".writing-editor-host");
  const editor = host.locator(".cm-content");
  const deleted = host.locator(".cm-deletedChunk");
  await expect(deleted).toHaveCount(2);
  const oldText = await deleted.allTextContents();
  const originalEditor = await editor.elementHandle();
  expect(originalEditor).not.toBeNull();
  const oldLine = deleted.first().locator(".cm-deletedLine").first();
  expect(await oldLine.evaluate(element => element.closest('[contenteditable="false"]') !== null)).toBe(true);

  const oldWord = await wordBounds(oldLine, "cover");
  await page.mouse.move(oldWord.left + 0.1, oldWord.y);
  await page.mouse.down();
  await page.mouse.move(oldWord.right + 0.1, oldWord.y, { steps: 5 });
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => getSelection()?.toString())).toBe("cover");
  await page.keyboard.press(process.platform === "darwin" ? "Meta+c" : "Control+c");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("cover");
  await page.keyboard.press("Backspace");
  expect(await deleted.allTextContents()).toEqual(oldText);

  const newLine = host.locator(".cm-line").filter({ hasText: "This paper presents" });
  const newWord = await wordBounds(newLine, "paper");
  await page.mouse.dblclick((newWord.left + newWord.right) / 2, newWord.y);
  await page.keyboard.insertText("probe");
  await expect(host.locator(".cm-line").filter({ hasText: "This probe presents" })).toBeVisible();
  expect(await deleted.allTextContents()).toEqual(oldText);
  await page.keyboard.press("Backspace");
  await expect(host.locator(".cm-line").filter({ hasText: "This prob presents" })).toBeVisible();
  expect(await deleted.allTextContents()).toEqual(oldText);
  expect(await originalEditor!.evaluate(node => node.isConnected)).toBe(true);

  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(deleted).toHaveCount(0);
  await page.getByRole("button", { name: "撤销", exact: true }).click();
  await expect(deleted).toHaveCount(2);
  expect(await deleted.allTextContents()).toEqual(oldText);
  await expect(editor).toContainText("This prob presents");
  await page.getByRole("button", { name: "重做", exact: true }).click();
  await expect(deleted).toHaveCount(0);
  expect(await originalEditor!.evaluate(node => node.isConnected)).toBe(true);
  await page.getByRole("button", { name: "模拟退出并重开", exact: true }).click();
  await expect.poll(() => originalEditor!.evaluate(node => node.isConnected)).toBe(false);
  await expect(editor).toContainText("This prob presents");
  await expect(deleted).toHaveCount(0);
  expect(errors).toEqual([]);
  if (process.env.VITE_WRITING_STATE_TRACE === "1") {
    await expect.poll(async () => (await Promise.all(diagnosticReads)).some(event => event.source === "checkpoint-restore")).toBe(true);
    const events = await Promise.all(diagnosticReads);
    for (const source of ["agent", "input", "review", "undo", "redo", "checkpoint-restore", "save-queue"] as const) {
      expect(events.some(event => event.source === source && event.status === "completed")).toBe(true);
    }
    expect(new Set(events.map(event => event.ownerId)).size).toBeGreaterThanOrEqual(2);
    expect(events.every(event => event.skipped === 0)).toBe(true);
    expect(JSON.stringify(events)).not.toContain("This prob presents");
    expect(JSON.stringify(events)).not.toContain("summary appears");
    for (const event of events) {
      expect(event.operationId).not.toBe("");
      if (event.after) expect(event.after.contentSha256).toMatch(/^[a-f0-9]{64}$/);
    }
  } else expect(diagnosticReads).toEqual([]);
  await page.screenshot({ path: ".tmp/writing-diff-selection-confirmed.png" });
});
