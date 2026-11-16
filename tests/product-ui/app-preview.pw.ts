import { expect, test, type Page } from "@playwright/test";

async function isolate(page: Page) {
  const forbidden: string[] = [];
  page.on("request", request => {
    const url = new URL(request.url());
    if (url.protocol === "data:" || url.protocol === "blob:") return;
    // Business traffic is forbidden even when sent to the development origin.
    if (url.origin !== new URL(test.info().project.use.baseURL as string).origin || /^\/(api|v1|rpc|tauri)(\/|$)/.test(url.pathname)) forbidden.push(request.url());
  });
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    const base = new URL(test.info().project.use.baseURL as string);
    if (url.origin !== base.origin || /^\/(api|v1|rpc|tauri)(\/|$)/.test(url.pathname)) { forbidden.push(url.href); await route.abort(); return; }
    await route.continue();
  });
  await page.addInitScript(() => {
    const violations: string[] = [];
    Object.defineProperty(window, "__PREVIEW_VIOLATIONS__", { value: violations });
    const forbidden = (name: string): never => { violations.push(name); throw new Error(`Forbidden ${name}`); };
    // Simulate a present native bridge: selecting preview must still prevent IPC.
    Object.defineProperty(window, "__TAURI_INTERNALS__", { value: { invoke: () => forbidden("Tauri invocation") }, configurable: true });
    window.open = () => forbidden("external window");
    navigator.mediaDevices.getUserMedia = async () => forbidden("microphone request");
    localStorage.setItem("preview-isolation-business-marker", "unchanged");
  });
  const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
  return async () => { expect(forbidden).toEqual([]); expect(errors).toEqual([]); expect(await page.evaluate(() => Reflect.get(window, "__PREVIEW_VIOLATIONS__"))).toEqual([]); expect(await page.evaluate(() => localStorage.getItem("preview-isolation-business-marker"))).toBe("unchanged"); };
}
async function writing(page: Page) {
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true }).click();
  await expect(page.locator(".writing-editor-host .cm-content")).toBeVisible();
  await expect(page.getByRole("button", { name: "Keep all", exact: true })).toBeEnabled();
  await expect.poll(() => page.evaluate(() => {
    const prefix="research-assistant.app-preview.v3:", id=localStorage.getItem(prefix+"initial-manuscript");
    const raw=localStorage.getItem(prefix+"agent:"+id);
    return raw ? JSON.parse(raw).agent.completions.entries[0]?.[1].acknowledged : false;
  })).toBe(true);
}
async function openWritingChat(page: Page) {
  await expect(page.locator(".writing-editor-host .cm-content")).toBeVisible();
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await page.getByRole("button", { name: "对话列表", exact: true }).click();
  await page.locator('[aria-label="对话列表面板"] [data-conversation-id]').getByRole("button", {name: "完善摘要", exact: true}).first().click();
  await page.getByRole("button", { name: "隐藏对话列表", exact: true }).click();
}

test("focus adds no frame and preserves selected rows, input borders and keyboard actions", async ({ page }) => {
  const verify = await isolate(page);
  await page.goto("/?demo=app");
  const row = page.getByRole("row", { name: /Mock Methods and Reproducibility/ });
  await row.click();
  await expect(row).toHaveAttribute("aria-selected", "true");
  await row.press("Tab");
  const selected = await row.evaluate(element => {
    const style = getComputedStyle(element);
    return { border: style.border, shadow: style.boxShadow, background: style.backgroundColor };
  });
  await page.keyboard.press("Shift+Tab");
  await expect(row).toBeFocused();
  expect(await row.evaluate(element => element.matches(":focus-visible"))).toBe(true);
  await expect(row).toHaveCSS("outline-style", "none");
  expect(await row.evaluate(element => {
    const style = getComputedStyle(element);
    return { border: style.border, shadow: style.boxShadow, background: style.backgroundColor };
  })).toEqual(selected);

  const home = page.getByRole("tab", {name: "返回文献列表", exact: true});
  const homeFrame = home.locator("..");
  const homeBorder = await homeFrame.evaluate(element => getComputedStyle(element).borderColor);
  await home.focus();
  await expect(home).toHaveCSS("outline-style", "none");
  await expect(homeFrame).toHaveCSS("border-color", homeBorder);
  await home.press("Enter");
  await expect(home).toHaveAttribute("aria-selected", "true");
  await expect(row).toHaveAttribute("aria-selected", "true");

  const search = page.getByRole("textbox", { name: "搜索文献", exact: true });
  const shell = page.locator(".library-search");
  const border = await shell.evaluate(element => getComputedStyle(element).borderColor);
  await search.focus();
  await expect(search).toHaveCSS("outline-style", "none");
  await expect(search).toHaveCSS("box-shadow", "none");
  await expect(shell).toHaveCSS("border-color", border);
  await expect(shell).toHaveCSS("box-shadow", "none");

  const launcher = page.getByRole("button", { name: "打开对话", exact: true });
  const shadow = await launcher.evaluate(element => getComputedStyle(element).boxShadow);
  await launcher.focus();
  await expect(launcher).toHaveCSS("outline-style", "none");
  await expect(launcher).toHaveCSS("box-shadow", shadow);
  await launcher.press("Enter");
  await expect(page.getByRole("button", { name: /^隐藏(?: AI 写作助手|对话)$/ })).toBeVisible();
  await verify();
});

test("production library, PDF annotations and note editor work with isolated persistent data", async ({ page }) => {
  const verify = await isolate(page);
  await page.goto("/?preview=app");
  await expect(page.getByRole("row", { name: /Synthetic Research Preview/ })).toBeVisible();
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toBeVisible();
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.", { exact: true })).toBeVisible();
  await page.getByRole("tab", {name:"高亮与笔记",exact:true}).click();
  await page.getByRole("button", { name: "写文献笔记", exact: true }).click();
  const readingPanel = page.getByRole("dialog", { name: "文献笔记编辑面板", exact: true });
  await readingPanel.locator(".bn-editor").click();
  await readingPanel.locator(".bn-editor").pressSequentially("persisted preview annotation");
  await expect(readingPanel.getByRole("status", { name: "已保存", exact: true })).toBeVisible({ timeout: 15000 });
  await readingPanel.getByRole("button", { name: "关闭阅读笔记", exact: true }).click();
  await expect(page.getByText("persisted preview annotation", { exact: true }).first()).toBeVisible();
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("button", { name: "新建页面", exact: true }).first().click();
  await page.getByRole("textbox", { name: "笔记标题", exact: true }).fill("persistent preview note");
  await page.locator(".bn-editor").click(); await page.locator(".bn-editor").pressSequentially("synthetic note body");
  await expect(page.locator(".notes-content .note-save-state")).toHaveAttribute("aria-label", "已保存", {timeout:15000});
  await page.reload();
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("button", { name: /persistent preview note/ }).click();
  await expect(page.locator(".bn-editor")).toContainText("synthetic note body");
  await verify();
});

test("the three supplied papers appear in the library and open their distinct original PDFs", async ({ page }) => {
  test.setTimeout(60000);
  const verify = await isolate(page);
  await page.goto("/?preview=app");
  const papers = [
    { title: "Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic", pages: 15 },
    { title: "Automated construction of cognitive maps with visual predictive coding", pages: 16 },
    { title: "Life-inspired interoceptive artificial intelligence for autonomous and adaptive agents", pages: 12 },
  ];
  for (const paper of papers) await expect(page.getByRole("row", { name: new RegExp(paper.title) })).toBeVisible();
  for (const paper of papers) {
    await page.getByRole("row", { name: new RegExp(paper.title) }).dblclick();
    const reader = page.getByLabel(`${paper.title} PDF 阅读器`, { exact: true });
    await expect(reader).toBeVisible();
    await expect(reader.locator(".pdf-page-count")).toHaveText(`/ ${paper.pages}`, { timeout: 15000 });
    await expect(reader.locator(".textLayer")).not.toHaveCount(0);
    await page.getByRole("tab", { name: "返回文献列表", exact: true }).click();
  }
  await page.reload();
  for (const paper of papers) await expect(page.getByRole("row", { name: new RegExp(paper.title) })).toHaveCount(1);
  await verify();
});

test("one production editor survives chat and page switches; Keep, save, compile and reload agree", async ({ page }) => {
  test.setTimeout(60000);
  const verify = await isolate(page); await page.goto("/?preview=app"); await writing(page);
  const toolbar = await page.getByRole("banner", { name: "窗口工具栏", exact: true }).boundingBox();
  expect(toolbar!.height).toBe(36);
  const navigationButton = await page.getByRole("button", { name: "后退", exact: true }).boundingBox();
  expect(navigationButton!.height).toBe(24);
  expect(navigationButton!.y + navigationButton!.height / 2).toBe(toolbar!.y + toolbar!.height / 2);
  const editor = page.locator(".writing-editor-host .cm-content");
  const handle = await editor.elementHandle();
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await expect(page.getByRole("complementary", { name: "对话列表面板" })).toBeHidden();
  await page.getByRole("button", { name: "对话列表", exact: true }).click();
  await expect(page.getByRole("complementary", { name: "对话列表面板" })).toBeVisible();
  const conversationBounds = await page.getByRole("complementary", { name: "对话列表面板" }).boundingBox();
  expect(conversationBounds).not.toBeNull();
  expect(conversationBounds!.x).toBeCloseTo(18, 0);
  expect(conversationBounds!.y).toBeCloseTo(54, 0);
  const listHeader = await page.locator(".agent-canvas-conversations > header").boundingBox();
  expect(listHeader!.height).toBeCloseTo(31, 0);
  const listHide = await page.getByRole("button", { name: "隐藏对话列表", exact: true }).boundingBox();
  const chatHide = await page.getByRole("button", { name: /^隐藏(?: AI 写作助手|对话)$/ }).boundingBox();
  expect(listHide!.width).toBe(chatHide!.width);
  expect(listHide!.height).toBe(chatHide!.height);
  const listIcon = await page.getByRole("button", {name:"隐藏对话列表",exact:true}).locator("svg").boundingBox();
  const chatIcon = await page.locator(".writing-agent-header-actions button svg").boundingBox();
  expect(listIcon!.width).toBe(chatIcon!.width);
  expect(listIcon!.height).toBe(chatIcon!.height);
  await page.getByRole("button", { name: "隐藏对话列表", exact: true }).click();
  await page.getByRole("button", { name: /^隐藏(?: AI 写作助手|对话)$/ }).click();
  expect(await editor.evaluate((element, previous) => element === previous, handle)).toBe(true);
  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(page.locator(".cm-deletedChunk")).toHaveCount(0);
  await page.getByRole("button", { name: "文献", exact: true }).click();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  expect(await page.locator(".writing-editor-host .cm-content").evaluate((element, previous) => element === previous, handle)).toBe(true);
  const current = page.locator(".writing-editor-host .cm-content"); await current.click(); await current.press("ControlOrMeta+Home"); await current.pressSequentially("% preview saved text\n");
  await current.press("ControlOrMeta+s"); await expect(page.getByRole("status").filter({ hasText: "已保存" })).toBeVisible({ timeout: 15000 });
  await page.getByRole("button", { name: /^(编译 PDF|重新编译)$/ }).click();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.", { exact: true })).toBeVisible({ timeout: 15000 });
  await page.reload(); await page.getByRole("button", { name: "写作", exact: true }).click();
  const manuscript = page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true }); await manuscript.click();
  await expect(page.locator(".writing-editor-host .cm-content")).toContainText("preview saved text");
  await expect(page.locator(".cm-deletedChunk")).toHaveCount(0); await verify();
});

for (const entry of ["/?preview=app", "/?demo=app"]) {
test(`a newly created manuscript uses mock recovery, fills the app and retains one chat launcher at ${entry}`, async ({ page }) => {
  const verify = await isolate(page);
  await page.goto(entry);
  await expect(page.getByRole("row", { name: /Synthetic Research Preview/ })).toBeVisible();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "新建稿件", exact: true }).click();
  await page.getByRole("textbox", { name: "论文标题", exact: true }).fill("Layout regression manuscript");
  await page.getByRole("button", { name: "创建稿件", exact: true }).click();
  await expect(page.locator(".writing-editor-host .cm-content")).toBeVisible();
  await expect(page.locator(".writing-editor-host .cm-content")).toContainText("Layout regression manuscript");
  await expect(page.getByRole("status", { name: "读取文件操作记录", exact: true })).toHaveCount(0);
  await expect(page.getByRole("status", { name: "读取文件撤销记录", exact: true })).toHaveCount(0);
  await expect(page.getByText("尚未完成的 Agent 文件修改读取失败，当前编辑状态已保留。", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "编译 PDF", exact: true })).toBeEnabled();
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toHaveCount(1);
  await expect(page.locator(".writing-agent-trigger")).toHaveCount(1);
  const workspace = page.locator(".writing-workspace");
  await expect.poll(async () => {
    const bounds = await workspace.boundingBox();
    return Math.abs(bounds!.y + bounds!.height - page.viewportSize()!.height);
  }).toBeLessThanOrEqual(1);
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toHaveCount(0);
  await expect(page.locator(".writing-agent-trigger")).toHaveCount(0);
  await page.getByRole("button", { name: /^隐藏(?: AI 写作助手|对话)$/ }).click();
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toHaveCount(1);
  await page.getByRole("button", { name: "文献", exact: true }).click();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toHaveCount(1);
  const restored = await workspace.boundingBox();
  expect(Math.abs(restored!.y + restored!.height - page.viewportSize()!.height)).toBeLessThanOrEqual(1);
  await page.reload();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  const manuscript = page.getByRole("button", { name: "Layout regression manuscript main.tex", exact: true });
  await manuscript.click();
  await expect(page.locator(".writing-editor-host .cm-content")).toContainText("Layout regression manuscript");
  await expect(page.getByRole("status", { name: "读取文件操作记录", exact: true })).toHaveCount(0);
  await expect(page.getByRole("status", { name: "读取文件撤销记录", exact: true })).toHaveCount(0);
  await expect(page.getByText("尚未完成的 Agent 文件修改读取失败，当前编辑状态已保留。", { exact: true })).toHaveCount(0);
  await verify();
});
}

test("preview reset preserves business storage and restores only mock fixtures", async ({ page }) => {
  const verify = await isolate(page); await page.goto("/?preview=app");
  const reset = page.locator(".preview-reset-control");
  const bounds = await reset.boundingBox();
  expect(bounds!.width).toBe(48);
  expect(bounds!.height).toBe(24);
  expect(bounds!.y + bounds!.height).toBe(page.viewportSize()!.height);
  expect(bounds!.x + bounds!.width / 2).toBe(page.viewportSize()!.width / 2);
  await expect(reset).toHaveCSS("background-color", "rgba(95, 117, 103, 0.12)");
  await reset.hover();
  await expect(reset).toHaveCSS("background-color", "rgb(216, 226, 218)");
  await expect(page.getByRole("tooltip", { name: "重置预览数据，清除模拟编辑" })).toBeVisible();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "高级", exact: true }).click();
  const settings = page.getByRole("region", { name: "预览数据", exact: true });
  await expect(settings).toBeVisible();
  await settings.getByRole("spinbutton").fill("0"); await settings.getByRole("button", { name: "保存模拟设置" }).click();
  await page.reload();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "高级", exact: true }).click();
  await expect(settings.getByRole("spinbutton")).toHaveValue("0");
  await Promise.all([page.waitForEvent("load"), reset.click()]);
  // Reset performs a full reload, including the lazy literature workspace.
  await expect(page.getByRole("row", { name: /Synthetic Research Preview/ })).toBeVisible({ timeout: 15000 });
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "高级", exact: true }).click();
  await expect(settings.getByRole("spinbutton")).toHaveValue("100");
  await verify();
});

test("Agent uses the production submission and history controllers without moving earlier cards", async ({ page }) => {
  test.setTimeout(60000);
  const verify = await isolate(page); await page.goto("/?preview=app"); await writing(page);
  await openWritingChat(page);
  const input = page.getByRole("textbox", { name: "给 AI 写作助手发送消息", exact: true });
  await expect(input).toBeEnabled();
  const card = page.locator(".writing-agent-inline-diff-card").first();
  await expect(card).toBeVisible(); const original = await card.textContent();
  await input.fill("帮我继续修改当前文本");
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(page.getByRole("button", { name: "停止生成", exact: true })).toBeVisible({ timeout: 15000 });
  await expect(card).toContainText(original!);
  await expect(page.getByRole("button", { name: "停止生成", exact: true })).toBeHidden({ timeout: 20000 });
  await expect(page.locator(".writing-agent-inline-diff-card")).toHaveCount(2);
  await page.reload(); await writing(page);
  await openWritingChat(page);
  await expect(page.locator(".writing-agent-inline-diff-card")).toHaveCount(2);
  await verify();
});

test("refresh cancels a running mock turn without duplicating its message or file delivery", async ({ page }) => {
  const verify = await isolate(page); await page.goto("/?preview=app"); await writing(page);
  await openWritingChat(page);
  await page.clock.install(); await page.clock.pauseAt(new Date(await page.evaluate(() => Date.now()+100)));
  await page.getByRole("textbox", { name: "给 AI 写作助手发送消息", exact: true }).fill("refresh interruption fixture");
  await page.getByRole("button", { name: "发送消息", exact: true }).click({force:true});
  await page.clock.runFor(600);
  await expect(page.getByRole("button", { name: "停止生成", exact: true })).toBeVisible();
  await page.reload(); await page.clock.resume(); await writing(page);
  await openWritingChat(page);
  await expect(page.getByText("refresh interruption fixture", { exact: true })).toHaveCount(1);
  await expect(page.getByRole("button", { name: "停止生成", exact: true })).toBeHidden();
  await expect(page.locator(".writing-agent-inline-diff-card")).toHaveCount(1);
  await expect(page.getByRole("textbox", { name: "给 AI 写作助手发送消息", exact: true })).toBeEnabled();
  await verify();
});
