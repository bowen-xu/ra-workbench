import { test, expect, type Locator, type Page } from "@playwright/test";

const original = "逻辑推理模型可以整合局部观察，但目前缺少复杂场景的验证。";
const rewritten = "逻辑推理模型通过累积局部证据更新判断；现有结果支持其可行性，复杂场景中的稳健性仍需验证。";
function noteBody(page: Page): Locator { return page.locator(".notes-content").getByLabel("笔记正文", { exact: true }); }
function block(body: Locator, id: string): Locator { return body.locator(`[data-id="${id}"] .bn-inline-content`).first(); }
async function send(page: Page, text: string) {
  const composer = page.getByRole("textbox", { name: "给研究助手发送消息", exact: true });
  await composer.fill(text);
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(page.locator(".writing-agent-message.user").last()).toContainText(text);
  await expect(composer).toBeEmpty();
}
async function finished(page: Page) {
  const chat = page.locator(".writing-agent-panel");
  await expect(chat.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/);
}
async function latestCard(page: Page): Promise<Locator> {
  await finished(page);
  const turns = page.locator(".writing-agent-panel .writing-agent-turn");
  await expect(turns.last().getByLabel("补丁结果")).toBeVisible();
  const cards = turns.filter({ has: page.getByLabel("补丁结果") }).getByLabel("补丁结果");
  return cards.nth(await cards.count() - 1);
}
async function hideChat(page: Page) { await page.getByRole("button", { name: "隐藏对话", exact: true }).click(); }
async function showChat(page: Page) { await page.getByRole("button", { name: "打开对话", exact: true }).click(); }

test("the real App reviews note changes at their blocks and keeps frozen chat Diff at the owning turn end", async ({ page }) => {
  await page.goto("/?preview=app&demo=text-patch");
  const body = noteBody(page), note = page.locator(".notes-content");
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("研究笔记");
  await expect(page.getByRole("button", { name: "更多页面操作" })).toBeVisible();
  await expect(page.getByRole("button", { name: "重置预览数据" })).toBeVisible();
  await expect(page.locator(".text-patch-demo")).toHaveCount(0);
  const recommendation = page.getByRole("group", { name: "推荐持续上下文 笔记 · 研究笔记", exact: true });
  const recommendationBox = await recommendation.boundingBox();
  if (!recommendationBox) throw new Error("Missing note recommendation");
  expect(recommendationBox.width).toBeLessThan(120);
  await expect(page.locator(".writing-agent-message.assistant").last()).toContainText("读取笔记");
  await expect(page.locator(".writing-agent-message.assistant").last()).not.toContainText("读取当前候选");
  await expect(page.locator(".writing-agent-message.assistant").last()).not.toContainText("读取已保存版本");
  await expect(block(body, "preview-argument")).toHaveText(original);
  const handle = await body.locator(".ProseMirror").elementHandle();

  await send(page, "改写研究问题");
  await expect(page.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /工作中/);
  const firstCard = await latestCard(page);
  await expect(block(body, "preview-argument")).toHaveText(rewritten);
  await expect(note.getByLabel("笔记变更审阅")).toHaveCount(0);
  await expect(note.locator(".agent-text-patch-review")).toHaveCount(0);
  await expect(body.locator('.note-diff-deleted-blocks[contenteditable="false"]')).toContainText(original);
  await send(page, "补充验证清单");
  const secondCard = await latestCard(page);
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(2);
  await expect(firstCard.locator(".writing-agent-inline-diff-counts").first()).toHaveText("+1 −1");
  await expect(secondCard.locator(".writing-agent-inline-diff-counts").first()).toContainText("+5");
  // Each card opens its tool generation, even after later candidate changes.
  await firstCard.getByRole("button", { name: /^研究笔记/ }).click();
  const details = page.getByRole("dialog", { name: "笔记 Diff 详情" });
  await details.getByRole("button", { name: "下一处变更", exact: true }).click();
  await expect(details).toContainText("证据更新判断");
  await expect(details).not.toContainText("下一步验证");
  await details.getByRole("button", { name: "关闭 Diff 详情" }).click();
  await secondCard.getByRole("button", { name: /^研究笔记/ }).click();
  await details.getByRole("button", { name: "下一处变更", exact: true }).click();
  await expect(details).toContainText("下一步验证");
  await expect(details.getByRole("button", { name: "接受整组" })).toHaveCount(0);
  await details.getByRole("button", { name: "关闭 Diff 详情" }).click();
  await expect(secondCard.getByRole("button", { name: "接受整组", exact: true })).toHaveCount(0);
  await expect(secondCard.getByRole("button", { name: "拒绝整组", exact: true })).toHaveCount(0);
  await expect(secondCard.getByRole("button", { name: "继续调整", exact: true })).toHaveCount(0);
  await expect(secondCard.locator("header").getByRole("button", { name: "Undo", exact: true })).toBeEnabled();
  await expect(secondCard.locator("xpath=ancestor::*[contains(@class,'writing-agent-turn')][1]").locator(".writing-agent-turn-process")).not.toHaveAttribute("open", "");

  await send(page, "读取笔记"); await finished(page);
  await expect(page.locator(".writing-agent-message.assistant").last()).toContainText("下一步验证");
  await expect(page.locator(".writing-agent-message.assistant").last()).toContainText(rewritten);
  await expect(page.locator(".writing-agent-message.assistant").last().locator("pre")).toHaveCount(0);

  await hideChat(page);
  await note.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(note.getByLabel("变更控制", { exact: true })).toHaveCount(0);
  await expect(note.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await expect(block(body, "preview-argument")).toHaveText(rewritten);
  await expect(body.getByRole("link", { name: "方法说明" })).toHaveAttribute("href", "https://example.org/method");
  expect(await handle?.evaluate(element => element.isConnected)).toBe(true);
  await showChat(page);
  await secondCard.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(block(body, "preview-argument")).toHaveText(original);
  await expect(block(body, "preview-check-heading")).toHaveCount(0);
  expect(await handle?.evaluate(element => element.isConnected)).toBe(true);
});

test("individual Keep advances B and Keep all history undo restores the pending Diff without changing D", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "改写研究问题"); await latestCard(page);
  await send(page, "补充验证清单"); await latestCard(page);
  await hideChat(page);
  const body = noteBody(page), note = page.locator(".notes-content"), editor = body.locator(".ProseMirror");
  const handle = await editor.elementHandle();
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(2);
  await note.getByRole("button", { name: "保留第 1 处变更", exact: true }).click();
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(body.locator(".note-diff-deleted-blocks")).toHaveCount(0);
  await expect(note.getByLabel("当前变更位置")).toHaveText("1/1");
  await note.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(note.getByLabel("变更控制", { exact: true })).toHaveCount(0);
  await editor.click(); await editor.press("ControlOrMeta+z");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(block(body, "preview-argument")).toHaveText(rewritten);
  await expect(block(body, "preview-check-heading")).toHaveText("下一步验证");
  await expect(note.getByRole("button", { name: "Undo all", exact: true })).toBeEnabled();
  await note.getByRole("button", { name: "Undo all", exact: true }).click();
  await expect(note.getByLabel("变更控制", { exact: true })).toHaveCount(0);
  await expect(block(body, "preview-check-heading")).toHaveCount(0);
  await expect(block(body, "preview-argument")).toHaveText(rewritten); // Prior Keep remains B.
  expect(await handle?.evaluate(element => element.isConnected)).toBe(true);
});

test("ordinary manual input synchronizes B and survives Undo all instead of staling the entire proposal", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "改写研究问题"); const card = await latestCard(page);
  await hideChat(page);
  const body = noteBody(page), note = page.locator(".notes-content"), source = block(body, "preview-source");
  const editor = body.locator(".ProseMirror");
  const bounds = await source.boundingBox();
  if (!bounds) throw new Error("Ordinary note paragraph is not visible");
  await source.click({ position: { x: bounds.width - 4, y: Math.min(12, bounds.height - 2) } });
  await editor.pressSequentially(" 手动保留的输入");
  await expect(source).toContainText("手动保留的输入");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(note.getByRole("button", { name: "Undo all", exact: true })).toBeEnabled();
  await showChat(page);
  await expect(card.getByRole("button", { name: "Undo", exact: true })).toBeDisabled();
  await hideChat(page);
  await note.getByRole("button", { name: "Undo all", exact: true }).click();
  await expect(block(body, "preview-argument")).toHaveText(original);
  await expect(source).toContainText("手动保留的输入");
  await expect(source.locator('a[href="https://example.org/method"]')).toContainText("方法说明");
  await expect(note.getByLabel("变更控制", { exact: true })).toHaveCount(0);
});

test("manual input inside a pending block preserves its old side and remains reviewable", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "改写研究问题"); await latestCard(page); await hideChat(page);
  const body = noteBody(page), note = page.locator(".notes-content"), argument = block(body, "preview-argument");
  await argument.click(); await argument.press("End"); await argument.pressSequentially(" 手动补充");
  await expect(argument).toContainText("手动补充");
  await expect(body.locator(".note-diff-deleted-blocks")).toContainText(original);
  await expect(body.locator(".note-diff-deleted-blocks")).not.toContainText("手动补充");
  await expect(note.getByRole("button", { name: "Keep all", exact: true })).toBeEnabled();
  await note.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(note.getByLabel("变更控制", { exact: true })).toHaveCount(0);
  await expect(argument).toContainText("手动补充");
  await expect(note.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
});

test("full App keeps the production responsive note and chat layout", async ({ page }) => {
  await page.setViewportSize({ width: 650, height: 900 });
  await page.goto("/?demo=text-patch");
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("研究笔记");
  await expect(page.getByRole("textbox", { name: "给研究助手发送消息", exact: true })).toBeVisible();
  await send(page, "改写研究问题"); const card = await latestCard(page);
  await expect(card.getByRole("button", { name: "Undo", exact: true })).toBeEnabled();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await card.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(card.locator("header strong")).toContainText("Edited file");
  await expect(block(noteBody(page), "preview-argument")).toHaveText(original);
  await hideChat(page);
  await expect(page.getByLabel("变更控制", { exact: true })).toHaveCount(0);
});

test("stopping a staged tool turn prevents its patch and allows a fresh message", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("研究笔记");
  await send(page, "改写研究问题");
  const chat = page.locator(".writing-agent-panel");
  await expect(chat.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /工作中/);
  await chat.getByRole("button", { name: "停止生成", exact: true }).click();
  await expect(chat).toContainText("已中止");
  await expect(page.locator(".notes-content").getByLabel("变更控制", { exact: true })).toHaveCount(0);
  await expect(block(noteBody(page), "preview-argument")).toHaveText(original);
  await send(page, "补充验证清单"); await latestCard(page);
  await expect(block(noteBody(page), "preview-check-heading")).toHaveText("下一步验证");
  await expect(block(noteBody(page), "preview-argument")).toHaveText(original);
  await expect(page.locator(".notes-content").getByLabel("笔记变更审阅")).toHaveCount(0);
});

test("rich note formats render old and new content in place with independent decisions and sticky controls", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "修改格式示例"); await latestCard(page); await hideChat(page);
  const body = noteBody(page), note = page.locator(".notes-content");
  const toolbar = note.locator(".note-page-topline .writing-diff-control-row");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(7);
  await expect(body.locator(".note-diff-deleted-blocks")).toHaveCount(5);
  await expect(body.locator(".note-diff-deleted-content")).toHaveCount(2);
  await expect(toolbar.getByLabel("当前变更位置")).toHaveText("1/7");
  await expect(note.getByLabel("笔记变更审阅")).toHaveCount(0);
  await expect(body).not.toContainText("<block-format");
  await expect(body).not.toContainText("<reference");

  const added = (id: string) => body.locator(`[data-id="${id}"]`).first();
  const oldFor = async (id: string) => {
    const chunkId = await added(id).getAttribute("data-note-diff-chunk");
    if (!chunkId) throw new Error(`Missing rich note Diff chunk for ${id}`);
    return body.locator(`.note-diff-deleted-blocks[data-note-diff-chunk="${chunkId}"]`);
  };
  const oldInline = await oldFor("preview-format-inline"), newInline = added("preview-format-inline");
  await expect(oldInline.locator('.note-formula-preview[aria-label="E = mc^2"] .katex')).toHaveCount(1);
  await expect(newInline.locator('.note-formula-preview[aria-label="E^2 = (mc^2)^2 + (pc)^2"] .katex')).toHaveCount(1);
  await expect(oldInline.locator("strong")).toContainText("能量模型");
  await expect(newInline.locator("strong")).toContainText("修订能量模型");
  await expect(newInline.locator("u")).toContainText("修订能量模型");
  await expect(oldInline.locator("em")).toContainText("这些数值仅用于界面预览");
  await expect(newInline.locator("em")).toContainText("保留普通链接和对象引用");
  const metrics = async (item: Locator) => item.locator(".bn-inline-content").first().evaluate(element => {
    const style = getComputedStyle(element), bounds = element.getBoundingClientRect();
    return { x: bounds.x, font: style.fontSize, lineHeight: style.lineHeight };
  });
  const oldMetrics = await metrics(oldInline), newMetrics = await metrics(newInline);
  expect(Math.abs(oldMetrics.x - newMetrics.x)).toBeLessThanOrEqual(1);
  expect(oldMetrics.font).toBe(newMetrics.font);
  expect(oldMetrics.lineHeight).toBe(newMetrics.lineHeight);

  const oldEquation = await oldFor("preview-format-equation"), newEquation = added("preview-format-equation");
  await expect(oldEquation.locator('.note-formula-preview[aria-label="f(x) = x^2"] .katex-display')).toHaveCount(1);
  await expect(newEquation.getByLabel("f(x) = x^2 + \\lambda x", { exact: true }).locator(".katex-display")).toHaveCount(1);
  const oldTable = await oldFor("preview-format-table"), newTable = added("preview-format-table");
  await expect(oldTable.locator("table")).toContainText("0.65");
  await expect(newTable.locator("table")).toContainText("0.72（模拟）");
  await expect(oldTable.locator("table tr")).toHaveCount(3);
  await expect(newTable.locator("table tr")).toHaveCount(3);
  await expect(newTable.locator("table tr").nth(2).locator("td").nth(1).locator(".note-diff-added-text")).toHaveText("72（模拟）");
  await expect(newTable.locator("table tr").nth(1).locator(".note-diff-added-text")).toHaveCount(0);
  const code = added("preview-format-code"), quote = added("preview-format-quote");
  await expect(code.locator("pre")).toHaveCount(1);
  await expect(code.locator("pre > code .note-diff-deleted-content")).toHaveText("score = evaluate(model)\nprint(score)");
  await expect(code.locator("pre > code")).toContainText("noise=True");
  await expect(code.locator(".note-diff-deleted-content")).toHaveCSS("white-space", "pre-wrap");
  await expect(code.locator("pre > code .tok-number").last()).toHaveText("3");
  await expect(code.locator("pre")).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(quote.locator("blockquote")).toHaveCount(1);
  await expect(quote.locator("blockquote .note-diff-deleted-content")).toContainText("先明确假设，再检查证据。");
  await expect(quote).toHaveCSS("border-left-color", "rgba(0, 0, 0, 0)");
  const quoteMarker = await quote.locator("blockquote").evaluate(element => {
    const old = element.querySelector(".note-diff-deleted-quote"), style = getComputedStyle(element);
    if (!old) throw new Error("Missing readonly quote content");
    return { image: style.backgroundImage, oldX: old.getBoundingClientRect().x,
      greenX: element.getBoundingClientRect().x + Number.parseFloat(style.borderLeftWidth) + Number.parseFloat(style.backgroundPositionX) };
  });
  expect(quoteMarker.image).toContain("rgb(107, 175, 117)");
  expect(Math.abs(quoteMarker.greenX - quoteMarker.oldX)).toBeLessThanOrEqual(1);
  const codeMarker = await code.locator("pre > code").evaluate(element => {
    const old = element.querySelector(".note-diff-deleted-codeBlock"), style = getComputedStyle(element);
    if (!old) throw new Error("Missing readonly code content");
    return { image: style.backgroundImage, display: style.display, oldX: old.getBoundingClientRect().x,
      greenX: element.getBoundingClientRect().x + Number.parseFloat(style.backgroundPositionX) };
  });
  expect(codeMarker.image).toContain("rgb(107, 175, 117)");
  expect(codeMarker.display).toBe("block");
  expect(Math.abs(codeMarker.greenX - codeMarker.oldX)).toBeLessThanOrEqual(1);
  for (const id of ["preview-format-heading", "preview-format-inline", "preview-format-equation", "preview-format-table", "preview-format-list"]) {
    await expect(added(id)).toHaveCSS("border-left-color", "rgb(107, 175, 117)");
  }

  await newTable.scrollIntoViewIfNeeded();
  const top = await toolbar.boundingBox(), content = await note.boundingBox();
  if (!top || !content) throw new Error("Rich note review toolbar is not visible");
  expect(top.y).toBeGreaterThanOrEqual(content.y - 1);
  expect(top.y).toBeLessThanOrEqual(content.y + 3);
  await page.screenshot({ path: ".tmp/note-inline-diff-rich-formats.png" });
  await code.scrollIntoViewIfNeeded();
  await page.screenshot({ path: ".tmp/note-inline-diff-code-highlight.png" });
  await note.getByRole("button", { name: "保留第 4 处变更", exact: true }).click();
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(6);
  await expect(newTable.locator("table")).toContainText("0.72（模拟）");
  await expect(oldTable).toHaveCount(0);
  await note.getByRole("button", { name: "撤销第 1 处变更", exact: true }).click();
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(5);
  await expect(block(body, "preview-format-heading")).toHaveText("实验指标");
  await note.getByRole("button", { name: "Undo all", exact: true }).click();
  await expect(toolbar).toHaveCount(0);
  await expect(newTable.locator("table")).toContainText("0.72（模拟）");
  await expect(block(body, "preview-format-inline")).toContainText("这些数值仅用于界面预览");
  await expect(added("preview-format-code")).toContainText("score = evaluate(model)");
});

test("manual root creation in a pending Diff stays outside Agent differences and survives Undo all", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "修改格式示例"); await latestCard(page); await hideChat(page);
  const body = noteBody(page), note = page.locator(".notes-content");
  const changed = block(body, "preview-format-list");
  await changed.click();
  // Establish the block-end caret directly; Meta+ArrowRight is not a portable
  // end-of-line shortcut across the browser test platforms.
  await changed.evaluate(element => {
    const range = document.createRange(); range.selectNodeContents(element); range.collapse(false);
    const selection = element.ownerDocument.getSelection(); selection?.removeAllRanges(); selection?.addRange(range);
  });
  await page.keyboard.press("Enter"); await page.keyboard.type("用户新建一");
  await page.keyboard.press("Enter"); await page.keyboard.type("用户新建二");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(7);
  const old = body.locator(".note-diff-deleted-blocks");
  await expect(old.filter({ hasText: "用户新建" })).toHaveCount(0);
  const newManual = body.locator('.bn-block-outer[data-id]').filter({ hasText: /^用户新建[一二]$/u });
  await expect(newManual).toHaveCount(2);
  await expect(newManual.filter({ has: page.locator(".note-diff-added-text") })).toHaveCount(0);
  await expect(newManual.and(body.locator(".note-diff-added-block"))).toHaveCount(0);
  await note.getByRole("button", { name: "Undo all", exact: true }).click();
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(0);
  await expect(block(body, "preview-format-list")).toHaveText("比较噪声条件");
  await expect(newManual).toHaveCount(2);
  await expect(body).toContainText("用户新建一"); await expect(body).toContainText("用户新建二");
});

test("note recommendation follows its name width and a read turn returns a summary without source code", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  const short = page.getByRole("group", { name: "推荐持续上下文 笔记 · 研究笔记", exact: true });
  await expect(short).toBeVisible();
  const shortBox = await short.boundingBox();
  if (!shortBox) throw new Error("Missing short recommendation");
  expect(shortBox.width).toBeLessThan(120);
  await send(page, "读取笔记"); await finished(page);
  const answer = page.locator(".writing-agent-message.assistant").last();
  await expect(answer).toContainText("研究问题");
  await expect(answer).toContainText("表格数值为模拟数据");
  await expect(answer.locator("pre")).toHaveCount(0);
  await expect(answer).not.toContainText("score = evaluate");
  await expect(answer).not.toContainText("<reference");
  const longTitle = "逻辑视觉识别研究与复杂场景稳健性验证笔记".repeat(3);
  await page.getByRole("textbox", { name: "笔记标题", exact: true }).fill(longTitle);
  await page.getByRole("textbox", { name: "笔记标题", exact: true }).press("Tab");
  const long = page.getByRole("group", { name: `推荐持续上下文 笔记 · ${longTitle}`, exact: true });
  await expect(long).toBeVisible();
  const longBox = await long.boundingBox();
  if (!longBox) throw new Error("Missing long recommendation");
  expect(longBox.width).toBeGreaterThan(shortBox.width);
  expect(longBox.width).toBeLessThanOrEqual(24 * Number.parseFloat(await long.evaluate(element => getComputedStyle(element).fontSize)));
  await page.setViewportSize({ width: 650, height: 720 });
  const header = await page.locator(".writing-agent-header").boundingBox(), chip = await long.boundingBox();
  if (!header || !chip) throw new Error("Missing responsive header geometry");
  expect(chip.x).toBeGreaterThanOrEqual(header.x);
  expect(chip.x + chip.width).toBeLessThanOrEqual(header.x + header.width);
});

for (const action of ["保留", "撤销"] as const) test(`${action} a lower note Diff does not return the reading viewport to the initial caret`, async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "修改格式示例"); await latestCard(page); await hideChat(page);
  const body = noteBody(page), note = page.locator(".notes-content");
  const control = note.getByRole("button", { name: `${action}第 4 处变更`, exact: true });
  await body.getByRole("heading", { name: "研究问题", exact: true }).click();
  await control.scrollIntoViewIfNeeded();
  const before = await note.evaluate(element => element.scrollTop);
  expect(before).toBeGreaterThan(300);
  await control.click();
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(6);
  await expect.poll(() => note.evaluate(element => element.scrollTop)).toBeGreaterThan(before - 220);
  await page.screenshot({ path: `.tmp/note-review-${action === "保留" ? "keep" : "undo"}-scroll.png` });
  // The last decision also enters the formal-save lease; it must not reset
  // the reading viewport when the editor is locked and unlocked again.
  const remaining = body.getByRole("button", { name: new RegExp(`^${action}第 `) });
  while (await remaining.count() > 1) {
    const count = await remaining.count(); await remaining.first().click();
    await expect(remaining).toHaveCount(count - 1);
  }
  await body.getByRole("heading", { name: "研究问题", exact: true }).click();
  await remaining.last().scrollIntoViewIfNeeded();
  const finalPosition = await note.evaluate(element => element.scrollTop);
  await remaining.last().click();
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(0);
  await expect(note.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await expect.poll(() => note.evaluate(element => element.scrollTop)).toBeGreaterThan(finalPosition - 220);
  await expect(body.locator(".bn-editor")).toBeFocused();
});

test("Keep returns keyboard focus and review undo/redo preserves the lower reading viewport", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "修改格式示例"); await latestCard(page); await hideChat(page);
  const body = noteBody(page), editor = body.locator(".bn-editor"), note = page.locator(".notes-content");
  await body.getByRole("heading", { name: "研究问题", exact: true }).click();
  const oldView = await editor.elementHandle();
  const keep = body.getByRole("button", { name: "保留第 4 处变更", exact: true });
  await keep.scrollIntoViewIfNeeded();
  await keep.click(); await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(6);
  await expect(editor).toBeFocused();
  const keptPosition = await note.evaluate(element => element.scrollTop);
  expect(keptPosition).toBeGreaterThan(300);
  // Send to the page: no click/focus helper may mask lost keyboard ownership.
  await page.keyboard.press("ControlOrMeta+z");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(7);
  await expect(editor).toBeFocused();
  await expect.poll(() => note.evaluate(element => element.scrollTop)).toBeGreaterThan(keptPosition - 220);
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(6);
  await expect(editor).toBeFocused();
  await expect.poll(() => note.evaluate(element => element.scrollTop)).toBeGreaterThan(keptPosition - 220);
  expect(await oldView?.evaluate(element => element.isConnected)).toBe(true);
  await page.screenshot({ path: ".tmp/note-review-keyboard-focus.png" });
});

test("undo and redo of an Agent installation restores its pending Diff without keeping it", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await send(page, "修改格式示例"); await latestCard(page); await hideChat(page);
  const body = noteBody(page), editor = body.locator(".bn-editor");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(7);
  await body.getByRole("heading", { name: "研究问题", exact: true }).click();
  await page.keyboard.press("ControlOrMeta+z");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(0);
  await expect(block(body, "preview-format-heading")).toHaveText("实验指标");
  await expect(editor).toBeFocused();
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(body.locator(".note-diff-chunk-actions")).toHaveCount(7);
  await expect(block(body, "preview-format-heading")).toHaveText("实验指标与稳健性验证");
  await expect(page.locator(".notes-content").getByLabel("当前变更位置")).toHaveText("1/7");
  await expect(editor).toBeFocused();
  await page.screenshot({ path: ".tmp/note-agent-redo-pending-diff.png" });
});
