import { expect, test } from "@playwright/test";

test("chat close, reopen and conversation switches preserve a scrolled source editor and its history", async ({ page }) => {
  await page.goto("/?demo=writing-editor-diff");
  const pane = page.getByRole("region", { name: "编辑器 Diff 预览", exact: true });
  const editor = pane.locator(".cm-content");
  const scroller = pane.locator(".cm-scroller");
  const navigation = pane.locator(".writing-diff-navigation-group");
  await expect(navigation).toContainText("1/2");
  const originalEditor = await editor.elementHandle();
  const originalScroller = await scroller.elementHandle();
  expect(originalEditor).not.toBeNull();
  expect(originalScroller).not.toBeNull();
  await editor.click();
  await page.keyboard.press(process.platform === "darwin" ? "Meta+ArrowDown" : "Control+End");
  const tail = Array.from({ length: 100 }, (_, index) => `% viewport marker ${String(index).padStart(3, "0")}`).join("\n");
  await page.keyboard.insertText(`\n${tail}`);
  await expect(editor).toContainText("viewport marker 099");
  // Keep a known caret near the bottom, then move the viewport independently.
  await page.keyboard.press("ArrowLeft");
  const cursorStatus = await pane.locator(".writing-editor-status").innerText();
  await scroller.hover();
  await page.mouse.wheel(0, -240);
  await expect.poll(() => scroller.evaluate(element => element.scrollTop)).toBeGreaterThan(200);
  const viewport = await scroller.evaluate(element => ({ top: element.scrollTop, left: element.scrollLeft }));
  expect(viewport.top).toBeGreaterThan(200);
  await test.info().attach("editor-viewport", {
    body: JSON.stringify({ viewport, cursorStatus }),
    contentType: "application/json",
  });
  const visibleText = await editor.innerText();

  const unchanged = async () => {
    await expect.poll(() => scroller.evaluate(element => ({ top: element.scrollTop, left: element.scrollLeft }))).toEqual(viewport);
    expect(await originalEditor!.evaluate(element => element.isConnected)).toBe(true);
    expect(await originalScroller!.evaluate(element => element.isConnected)).toBe(true);
    expect(await editor.evaluate((element, original) => element === original, originalEditor!)).toBe(true);
    expect(await editor.innerText()).toBe(visibleText);
    expect(await pane.locator(".writing-editor-status").innerText()).toBe(cursorStatus);
    await expect(navigation).toContainText("1/2");
  };
  await page.getByRole("button", { name: "隐藏 AI 写作助手", exact: true }).click();
  await unchanged();
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await unchanged();
  await page.getByRole("button", {name:"对话列表",exact:true}).click();
  await page.getByRole("complementary", {name:"对话列表面板"}).getByRole("button", {name:"第二个对话",exact:true}).click();
  await page.getByRole("button", {name:"隐藏对话列表",exact:true}).click();
  await expect(page.getByRole("textbox", { name: "给 AI 写作助手发送消息", exact: true })).toBeEnabled();
  await unchanged();
  await page.getByRole("button", {name:"对话列表",exact:true}).click();
  await page.getByRole("complementary", {name:"对话列表面板"}).getByRole("button", {name:"完善摘要",exact:true}).click();
  await page.getByRole("button", {name:"隐藏对话列表",exact:true}).click();
  await unchanged();
  await pane.getByRole("button", { name: "撤销", exact: true }).click();
  await expect(editor).not.toContainText("viewport marker");
  await expect(navigation).toContainText("1/2");
  await pane.getByRole("button", { name: "重做", exact: true }).click();
  await expect(editor).toContainText("viewport marker");
  expect(await originalEditor!.evaluate(element => element.isConnected)).toBe(true);
  await page.screenshot({ path: ".tmp/writing-diff-chat-scroll.png" });
});
