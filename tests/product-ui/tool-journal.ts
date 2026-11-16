import {expect, type Page} from "@playwright/test";
import type {StructuredResource} from "@research-assistant/core/structured-resources";
import type {ToolJournalFixture} from "../../../integration/web/src/app/previews/agent/AgentPreview.browser";

/** Seed only synthetic data. The normal App owns all rendering and interaction. */
export async function openToolJournal(page: Page, fixture: ToolJournalFixture) {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "设置", exact: true}).waitFor();
  const hide = page.getByRole("button", {name: /^(隐藏对话|隐藏 AI 写作助手)$/u});
  if (await hide.isVisible()) await hide.click();
  await page.getByRole("button", {name: "打开对话", exact: true}).waitFor();
  const conversationId = await page.evaluate(async data =>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).seedToolJournal(data), fixture);
  await page.reload();
  await page.getByRole("button", {name: "打开对话", exact: true}).waitFor();
  if (fixture.pendingStep || fixture.waitingForModel) await page.evaluate(async waiting =>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).preparePendingToolJournal(waiting), !!fixture.waitingForModel);
  if (fixture.resources?.length || fixture.sourceTitles) await page.evaluate(async data =>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).configureToolResources(data), fixture);
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await page.getByRole("button", {name: "对话列表", exact: true}).click();
  const drawer = page.getByRole("complementary", {name: "对话列表面板", exact: true});
  await drawer.getByRole("button", {name: `工具执行记录 ${conversationId.slice(0, 8)}`, exact: true}).click();
  if (await drawer.isVisible()) await drawer.getByRole("button", {name: "隐藏对话列表", exact: true}).click();
  await expect(page.locator(".writing-agent-turn")).toHaveCount(1);
  return page.locator(".writing-agent-panel:visible");
}

/** Logical indexes stay stable while the production tool list windows its DOM. */
export async function toolHistoryRow(page: Page, index: number) {
  const calls = await toolHistoryCallIds(page);
  const id = calls[index];
  if (!id) throw new Error(`Synthetic tool ${index} unavailable`);
  const body = page.locator(".writing-agent-activity-group-body");
  const row = body.locator(`[data-transcript-row="${id}"] .tool-history-row`);
  await body.evaluate((element, fraction) => {
    element.scrollTop = fraction * (element.scrollHeight - element.clientHeight);
  }, index / Math.max(1, calls.length - 1));
  for (let attempt = 0; attempt < 8; attempt++) {
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    if (await row.count()) {
      await row.scrollIntoViewIfNeeded();
      return row;
    }
    await body.evaluate((element, value) => {
      const mounted = [...element.querySelectorAll<HTMLElement>("[data-transcript-row]")]
        .map(node => value.calls.indexOf(node.dataset.transcriptRow!)).filter(position => position >= 0);
      const nearest = value.index < Math.min(...mounted) ? Math.min(...mounted) : Math.max(...mounted);
      element.scrollTop += (value.index - nearest) * 24;
    }, {calls, index});
  }
  await expect(row).toHaveCount(1);
  return row;
}

export async function toolHistoryCallIds(page: Page): Promise<readonly string[]> {
  return page.evaluate(async () =>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).readToolHistoryCallIds());
}

/** Select the actual App Writing adapter, including its frozen-data reader. */
export async function openWritingJournal(page: Page, developerMode = false, fresh = false) {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "设置", exact: true}).waitFor();
  const hide = page.getByRole("button", {name: /^(隐藏对话|隐藏 AI 写作助手)$/u});
  if (await hide.isVisible()) await hide.click();
  await page.evaluate(async options =>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).selectWritingJournal(options), {developerMode, fresh});
  await page.reload();
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await expect(page.locator(".writing-agent-turn")).toHaveCount(fresh ? 0 : 1);
  const input = page.getByRole("textbox", {name: /给(?:研究助手| AI 写作助手)发送消息/u});
  await expect(input).toBeEnabled();
  return input;
}

export async function setToolDeveloperMode(page: Page, enabled: boolean) {
  const hide = page.getByRole("button", {name: /^(隐藏对话|隐藏 AI 写作助手)$/u});
  if (await hide.isVisible()) await hide.click();
  await page.getByRole("button", {name: "设置", exact: true}).click();
  await page.getByRole("button", {name: "高级", exact: true}).click();
  await page.getByRole("switch", {name: "开发者模式", exact: true}).setChecked(enabled);
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
}

export async function openToolHistory(page: Page, developerMode = false, pending = false, indexes?: readonly number[]) {
  await page.goto("/?preview=app");
  const steps = await page.evaluate(async () =>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).originalToolHistorySteps());
  return openToolJournal(page, {steps: indexes ? indexes.map(index => steps[index]!) : steps, developerMode,
    ...(pending ? {pendingStep: {name: "research_apply_patch", arguments: {path: "当前正在处理.md"}}} : {}),
    response: "工具执行记录已保存。\n\n" + "完成本轮合成资料检查。\n\n".repeat(12)});
}

export function structuredFixture(overrides: Partial<StructuredResource> = {}): StructuredResource {
  return {id: "synthetic-portrait", paperId: "private-paper-id", kind: "portrait", title: "合成文献画像", sourceUri: "source://library/private-paper-id/synthetic-portrait", revision: 1,
    content: {claims: [{text: "Synthetic saved claim"}]}, reviewBaseline: null,
    pendingFieldIds: ["claim"], reviewedFieldIds: [], ...overrides} as StructuredResource;
}
