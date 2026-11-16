import { expect, test } from "@playwright/test";
import { PREVIEW_STORAGE_PREFIX } from "../../apps/web/src/core/runtime/appStorage";
import { encodeMockState } from "../../apps/web/src/core/runtime/mockStateCodec";
import raw from "../fixtures/agent-events-v1.json" with {type: "json"};

test("completed multi-page history appears as a complete response, never as live output", async ({ page }) => {
  const conversationId = raw[0]!.conversationId, turnId = raw[0]!.payload.turnId;
  const itemId = raw[17]!.payload.itemId;
  const chunks = Array.from({length: 220}, (_, index) => `历史片段${index} `);
  const completeText = chunks.join("");
  const events = [...raw.slice(0, 17), ...chunks.map(text => ({...raw[17]!, payload: {...raw[17]!.payload, text}})),
    raw.find(event => event.payload.type === "itemCompleted" && event.payload.itemId === itemId)!,
    raw.find(event => event.payload.type === "turnFinished")!]
    .map((event, index) => ({...event, seq: index + 1, timestampMs: 1700000000000 + index}));
  const name = "已完成的分页历史";
  const seeded: Record<string, string> = {
    controls: JSON.stringify({delayMs: 80, nextFailure: null}),
    "independent-conversations": JSON.stringify([[conversationId, {conversationId, title: name}]]),
    "independent-metadata": JSON.stringify([[conversationId, {id: conversationId, title: name, archived: false,
      durableCount: events.length, createdAt: "2026-10-16T00:00:00.000Z", updatedAt: "2026-10-16T00:00:00.000Z"}]]),
    "independent-history": encodeMockState({events: new Map([[conversationId, events]]), admissions: new Map(), finished: new Set(), deleted: new Set()}),
  };
  await page.addInitScript(({prefix, seeded}) => {
    for (const [key, value] of Object.entries(seeded)) localStorage.setItem(prefix + key, value);
  }, {prefix: PREVIEW_STORAGE_PREFIX, seeded});
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await page.getByRole("button", {name: "对话列表", exact: true}).click();
  const messageSelector = `[data-agent-message-id="${turnId}/${itemId}"] .writing-agent-markdown`;
  await page.evaluate(selector => {
    const observations: {length: number; streaming: boolean}[] = [];
    Object.assign(window, {historyHydrationObservations: observations});
    new MutationObserver(() => {
      const body = document.querySelector(selector);
      if (body?.textContent) observations.push({length: body.textContent.length, streaming: body.getAttribute("aria-busy") === "true"});
    }).observe(document.body, {subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ["aria-busy"]});
  }, messageSelector);
  await page.getByRole("complementary", {name: "对话列表面板"}).getByRole("button", {name, exact: true}).click();
  const message = page.locator(messageSelector);
  await expect(message).toHaveText(completeText.trim());
  const observations = await page.evaluate(() => (window as typeof window & {
    historyHydrationObservations: {length: number; streaming: boolean}[];
  }).historyHydrationObservations);
  expect(observations.length).toBeGreaterThan(0);
  expect(observations.every(row => row.length === completeText.trim().length && !row.streaming)).toBe(true);
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toHaveCount(0);
  await expect(page.locator('[data-agent-activity="thinking"]')).toHaveCount(0);
  expect(await message.evaluate(element => element.getAnimations().length)).toBe(0);
  await page.getByRole("button", {name: "隐藏对话列表", exact: true}).click();
  await page.locator(".writing-agent-panel:visible").screenshot({path: ".tmp/completed-history-hydration.png"});
});
