import {expect, test, type Locator} from "@playwright/test";
import {enqueueCanvasMessages} from "./agent-canvas-queue";
import {holdLiveAgentOutput, openLiveAgent, sendLiveAgentMessage} from "./live-agent";
import {setToolDeveloperMode} from "./tool-journal";

async function expectNoRing(control: Locator) {
  await expect(control).toBeFocused();
  await expect(control).toHaveCSS("outline-style", "none");
  await expect(control).toHaveCSS("box-shadow", "none");
}

for (const width of [685, 1172]) {
  test(`chat focus has no ring and menus preserve keyboard navigation at ${width}px`, async ({page}) => {
    await page.setViewportSize({width, height: 808});
    const input = await openLiveAgent(page);
    await sendLiveAgentMessage(page, "保留消息与菜单焦点", 1);
    await holdLiveAgentOutput(page);
    await input.fill("保持运行");
    await page.getByRole("button", {name: "发送消息", exact: true}).click();
    await enqueueCanvasMessages(page, ["队列消息"]);
    const panel = page.locator(".writing-agent-panel:visible");
    await page.keyboard.press("Tab");
    for (const control of await panel.locator("button:enabled, summary, input:enabled").all()) {
      if (!await control.isVisible()) continue;
      await control.focus();
      await expectNoRing(control);
    }
    const menu = panel.getByRole("button", {name: "第 1 条消息菜单", exact: true});
    await menu.focus();
    await page.keyboard.press("Enter");
    const popup = page.getByRole("menu", {name: "第 1 条消息菜单", exact: true, includeHidden: true});
    await expect(popup).toBeVisible();
    await expect(popup.getByRole("menuitem", {name: "编辑消息", exact: true})).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(popup).toBeHidden();
    await expectNoRing(menu);

    await setToolDeveloperMode(page, true);
    await panel.locator(".writing-agent-message.user").first().hover();
    await panel.getByRole("button", {name: "查看原始数据", exact: true}).first().click();
    const close = page.getByRole("dialog", {name: "消息原始数据", exact: true})
      .getByRole("button", {name: "关闭", exact: true});
    await close.focus();
    await expectNoRing(close);
    await page.keyboard.press("Escape");

    const add = panel.getByRole("button", {name: "添加上下文和工具", exact: true});
    await add.focus();
    await page.keyboard.press("Enter");
    const addPopup = page.getByRole("menu", {name: "添加上下文和工具", exact: true});
    await expect(addPopup).toBeVisible();
    await page.keyboard.press("ArrowDown");
    await expectNoRing(addPopup.locator(":focus"));
    const file = page.getByRole("menuitem", {name: "文件", exact: true});
    await file.click();
    const resourceSearch = page.getByRole("searchbox", {name: "搜索文件", exact: true});
    await resourceSearch.focus();
    await expectNoRing(resourceSearch);
    await page.keyboard.press("Escape");
    await expectNoRing(add);
    const remove = panel.getByRole("button", {name: "移除第 1 条消息", exact: true});
    await remove.focus();
    await expectNoRing(remove);
    const settings = page.getByRole("button", {name: "设置", exact: true});
    await settings.focus();
    await expect(settings).toBeFocused();
    await expect(settings).toHaveCSS("outline-style", "none");
    await remove.focus();
    await panel.screenshot({path: `.tmp/agent-chat-focus-${width}.png`});

    await page.getByRole("button", {name: "停止生成", exact: true}).click();
    await expect(page.getByRole("button", {name: "停止生成", exact: true})).toHaveCount(0);
    await page.evaluate(async () => {
      await (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).selectLauncherOpenAiModel();
    });
    const model = page.getByRole("button", {name: "选择研究助手模型", exact: true});
    await expect(model).toContainText("GPT-5.6 Terra");
    await model.click();
    const effort = page.getByRole("slider", {name: "选择推理强度", exact: true});
    await effort.focus();
    await expectNoRing(effort);
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("Escape");
    await expectNoRing(model);
  });
}
