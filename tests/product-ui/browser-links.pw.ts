import { expect, test } from "@playwright/test";
import {openLiveAgent, sendLiveAgentMessage} from "./live-agent";

for (const route of ["/", "/?preview=app"]) {
  test(`note anchors and the production link toolbar open browser pages without replacing the app on ${route}`, async ({page, context}) => {
    const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      const open = window.open.bind(window); const calls: unknown[][] = [];
      Object.assign(window, {__browserOpenCalls: calls});
      window.open = (...args) => { calls.push(args); return open(...args); };
    });
    await context.route("https://example.org/research", request => request.fulfill({contentType: "text/html", body: "<title>Synthetic web destination</title>Browser adapter fixture"}));
    await page.goto(route);
    await expect(page.getByRole("button", {name: "笔记", exact: true})).toBeVisible();
    const noteId = await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).createBrowserLinkNote());
    await page.reload(); await page.getByRole("button", {name: "笔记", exact: true}).click();
    await page.evaluate(id => window.dispatchEvent(new CustomEvent("research-assistant:open-note-page", {detail: id})), noteId);
    const body = page.getByLabel("笔记正文", {exact: true});
    const external = body.getByRole("link", {name: "External web", exact: true});
    await expect(external).toBeVisible(); const appUrl = page.url();
    for (const open of [() => external.click(), async () => {
      await external.hover();
      const button = page.locator(".bn-link-toolbar").getByRole("button", {name: "在浏览器中打开", exact: true});
      await expect(button).toBeVisible();
      if (route.includes("preview")) await page.screenshot({path: ".tmp/browser-adapter-note.png"});
      await button.click();
    }]) {
      const popup = page.waitForEvent("popup"); await open(); const destination = await popup;
      await expect(destination).toHaveURL("https://example.org/research");
      await destination.waitForLoadState("domcontentloaded");
      expect(await destination.evaluate(() => window.opener === null)).toBe(true);
      await destination.close(); expect(page.url()).toBe(appUrl);
    }
    await body.getByRole("link", {name: "Internal note", exact: true}).click();
    await expect(page.getByRole("tab", {name: "Browser adapter sample", exact: true})).toHaveAttribute("aria-selected", "true");
    await expect(external).toBeVisible();
    expect(context.pages()).toHaveLength(1);
    expect(await page.evaluate(() => Reflect.get(window, "__browserOpenCalls"))).toEqual([
      ["https://example.org/research", "_blank", "noopener,noreferrer"],
      ["https://example.org/research", "_blank", "noopener,noreferrer"],
    ]);
    expect(errors).toEqual([]);
  });
}

test("the production chat renderer sends web links through an injected browser and reports failures", async ({page}) => {
  await openLiveAgent(page, {text: "[Markdown 格式说明](https://example.org/markdown)"});
  await sendLiveAgentMessage(page, "显示合成网页链接", 1);
  const web = page.getByRole("link", {name: "Markdown 格式说明", exact: true});
  await expect(web).toBeVisible({timeout: 30_000});
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).configureBrowserOpenFailure());
  const appUrl = page.url(); await web.click();
  await expect(page.getByText("[BROWSER_OPEN_FAILED] 无法打开网页：Synthetic OS error 5", {exact: true})).toBeVisible();
  expect(page.url()).toBe(appUrl);
  await page.screenshot({path: ".tmp/browser-adapter-failure.png"});
});

test("writing note links and their shared toolbar use the same browser port", async ({page}) => {
  await page.goto("/");
  await page.getByRole("button", {name: "写作", exact: true}).click();
  await page.getByRole("button", {name: "新建稿件", exact: true}).click();
  await page.getByRole("dialog", {name: "新建 LaTeX 稿件"}).getByRole("textbox").fill("Browser writing fixture");
  await page.getByRole("button", {name: "创建稿件", exact: true}).click();
  await page.getByRole("tab", {name: "笔记", exact: true}).click();
  await page.getByRole("button", {name: "新建笔记", exact: true}).click();
  const input = page.getByRole("region", {name: "写作笔记"}).getByRole("textbox", {name: "笔记名称"});
  await input.fill("Web sources"); await input.press("Enter");
  const editor = page.getByRole("dialog", {name: "写作笔记编辑面板"}).locator(".bn-editor");
  await editor.click(); await page.keyboard.press("ControlOrMeta+End"); await page.keyboard.press("Enter");
  await page.keyboard.insertText("Writing web source"); await page.keyboard.press("Shift+Home");
  await page.locator('.bn-formatting-toolbar [data-test="createLink"]').click();
  const url = page.locator('input[name="url"]'); await url.fill("https://example.net/writing"); await url.press("Enter");
  await page.keyboard.press("ArrowRight");
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).recordBrowserOpenings());
  const link = editor.locator('a[href="https://example.net/writing"]').last();
  await link.click(); await link.hover();
  await page.locator(".bn-link-toolbar").getByRole("button", {name: "在浏览器中打开", exact: true}).click();
  expect(await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).readBrowserOpenings())).toEqual(["https://example.net/writing", "https://example.net/writing"]);
  expect(page.context().pages()).toHaveLength(1);
});
