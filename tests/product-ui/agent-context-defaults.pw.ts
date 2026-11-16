import {expect, test} from "@playwright/test";

test("text-patch restores automatic draft defaults but preserves manual read-only access", async ({page}) => {
  await page.goto("/?preview=app&demo=text-patch");
  await page.getByRole("button", {name: "文献", exact: true}).click();
  await page.getByRole("row", {name: /Vision by Logic/}).dblclick();
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await page.getByRole("button", {name: "新建会话", exact: true}).click();
  const settings = page.getByRole("button", {name: "上下文管理", exact: true});
  const dialog = page.getByRole("dialog", {name: "上下文管理", exact: true});
  const notes = dialog.getByRole("switch", {name: /Vision by Logic.*阅读笔记 权限$/});
  const portrait = dialog.getByRole("switch", {name: /Vision by Logic.*文献画像 权限$/});
  await settings.click();
  await expect(notes).toBeChecked();
  await expect(portrait).toBeChecked();
  await expect(dialog.getByRole("switch", {name: /vision-by-logic.pdf 权限$/})).toBeDisabled();
  // Reproduce the old automatically saved draft in this isolated preview store.
  await page.evaluate(() => {
    const prefix = "research-assistant.text-patch-preview.v5:";
    const id = localStorage.getItem(`${prefix}research-assistant:conversation-pending`)!;
    const key = `${prefix}research-assistant:agent-context:v1:${id}`;
    const state = JSON.parse(localStorage.getItem(key)!);
    const paper = state.persistent.find((item: {resource: {kind: string}}) => item.resource.kind === "paperGroup");
    paper.resource.parts = [];
    paper.resource.newMemberAccess = "readOnly";
    localStorage.setItem(key, JSON.stringify(state));
    localStorage.setItem(`${prefix}research-assistant:context-auto:${id}`, JSON.stringify(["paperGroup", paper.resource.resourceId]));
  });
  await page.reload();
  await page.getByRole("button", {name: "文献", exact: true}).click();
  await page.getByRole("tab", {name: /Vision by Logic/}).click();
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await settings.click();
  await expect(notes).toBeChecked();
  await expect(portrait).toBeChecked();
  await dialog.screenshot({path: ".tmp/context-defaults-restored.png"});
  await notes.uncheck();
  await expect.poll(() => page.evaluate(() => {
    const prefix = "research-assistant.text-patch-preview.v5:";
    const id = localStorage.getItem(`${prefix}research-assistant:independent-selected`);
    return localStorage.getItem(`${prefix}research-assistant:context-auto:${id}`);
  })).toBe("manual");
  await expect(notes).not.toBeChecked();
  await page.reload();
  await page.getByRole("button", {name: "文献", exact: true}).click();
  await page.getByRole("tab", {name: /Vision by Logic/}).click();
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await settings.click();
  await expect(notes).not.toBeChecked();
  await expect(portrait).toBeChecked();
});
