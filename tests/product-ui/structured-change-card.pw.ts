import { expect, test } from "@playwright/test";
import {openToolJournal, structuredFixture} from "./tool-journal";

for (const count of [1, 2]) test(`${count} structured receipts share one production card, frozen counts and original-source actions`, async ({ page }) => {
  const saved = structuredFixture();
  const resources = [saved, structuredFixture({id: "synthetic-template", paperId: null, kind: "readingTemplate", title: "合成阅读模板", sourceUri: "source://agents/skills/edit-structured-resources/assets/reading/synthetic-template.json"})].slice(0, count);
  const fixture = await openToolJournal(page, {resources, steps: resources.map(resource => ({name: "research_apply_patch", arguments: {},
    output: {targets: [{path: resource.sourceUri, name: resource.title, status: "applied", message: "Saved with pending review.",
      changeId: `synthetic-change-${resource.id}`, structuredResource: {resourceId: resource.id, revision: 1}}]}}))});
  const card = fixture.getByRole("status", { name: "资源变更审阅", exact: true });
  await expect(card).toHaveCount(1);
  await expect(card).toContainText(count === 1 ? "Edited file 合成文献画像" : "Edited 2 files");
  await expect(card.locator('.writing-agent-inline-diff-list > button')).toHaveCount(count === 1 ? 0 : 2);
  await expect(card.getByRole("button", { name: "Undo", exact: true })).toHaveCount(1);
  await expect(card.getByRole("button", { name: "Undo", exact: true })).toBeEnabled();
  const file = card.getByRole("button", { name: /^合成文献画像/ });
  expect(await file.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
  })).toBe(true);
  const counts = await card.locator(".writing-agent-inline-diff-counts").innerText();
  await card.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(card.getByRole("button", { name: "Undo", exact: true })).toBeDisabled();
  await expect(card.locator(".writing-agent-inline-diff-counts")).toHaveText(counts);
  await page.getByRole("button", {name: "置顶聊天窗", exact: true}).click();
  const navigation = await page.evaluateHandle(() => {
    const state = {path: [] as string[]};
    window.addEventListener("research-assistant:navigate-source", event => {
      state.path = (event as CustomEvent).detail.link.path;
    }, {once: true});
    return state;
  });
  // v0.7.6 already opens the original source. ff3106e77 removed the old modal
  // and updated the component contract; this browser test had retained it.
  await file.click();
  await expect.poll(() => navigation.evaluate(state => state.path)).toEqual(["library", "private-paper-id", "synthetic-portrait"]);
  await navigation.dispose();
  await page.screenshot({ path: `.tmp/structured-change-card-${count}.png` });
});
