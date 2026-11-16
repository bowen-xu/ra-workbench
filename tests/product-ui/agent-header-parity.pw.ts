import { expect, test, type Page } from "@playwright/test";

const panel = (page: Page) => page.locator(".writing-agent-panel:visible");
async function expectSharedHeader(page: Page): Promise<void> {
  const header = panel(page).locator(".writing-agent-header");
  await expect(header).toBeVisible();
  await expect(header.getByRole("button", {name: "关闭当前对话", exact: true})).toHaveCount(0);
  await expect(header.getByRole("button", {name: "新建会话", exact: true})).toBeEnabled();
  await expect(header.locator(".agent-canvas-header-divider")).toHaveCount(1);
  const order = await header.evaluate(element => {
    const controls = [...element.querySelectorAll("button, .agent-canvas-header-divider, .writing-agent-header-subtitle")];
    return controls.slice(0, 5).map(node => node.tagName === "BUTTON" ? node.getAttribute("aria-label") : node.className);
  });
  expect(order.slice(1)).toEqual(["新建会话", "对话列表", "agent-canvas-header-divider", "writing-agent-header-subtitle"]);
  const buttons = header.locator(".agent-canvas-new-conversation, .agent-canvas-list-toggle");
  for (const button of await buttons.all()) {
    await expect(button).toHaveCSS("width", "24px");
    await expect(button).toHaveCSS("height", "24px");
  }
}

for (const route of ["app", "agent-canvas", "writing-editor-diff"]) {
  test(`${route} uses the production header and new-conversation lifecycle`, async ({page}) => {
    await page.goto(`/?demo=${route}`);
    if (route === "app") await page.getByRole("button", {name: "打开对话", exact: true}).click();
    await expectSharedHeader(page);
    const header = panel(page).locator(".writing-agent-header");
    await header.getByRole("button", {name: "对话列表", exact: true}).click();
    await expect(page.getByRole("complementary", {name: "对话列表面板"})).toHaveCSS("width", "220px");
    const rows = page.locator('[aria-label="对话列表面板"] [data-conversation-id]');
    await expect(rows.first()).toBeVisible();
    const count = await rows.count();
    await page.getByRole("button", {name: "隐藏对话列表", exact: true}).click();
    await header.getByRole("button", {name: "新建会话", exact: true}).click();
    await expect(header.locator(".writing-agent-header-subtitle")).toHaveText("新对话");
    await header.getByRole("button", {name: "新建会话", exact: true}).click();
    await header.getByRole("button", {name: "对话列表", exact: true}).click();
    await expect(rows).toHaveCount(count);
    await expect(rows.locator('[aria-current="true"]')).toHaveCount(0);
    await page.getByRole("button", {name: "隐藏对话列表", exact: true}).click();
    await expect(panel(page)).toContainText("研究助手为您服务");
    await expectSharedHeader(page);
    await header.screenshot({path: `.tmp/agent-header-${route}.png`});
  });
}

test("a configured directory keeps the same App header", async ({page}) => {
  await page.goto("/?demo=app");
  await page.getByRole("button", {name: "写作", exact: true}).click();
  await page.getByRole("button", {name: "Autonomy（合成稿件） main.tex", exact: true}).click();
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await page.getByRole("button", {name: "对话列表", exact: true}).click();
  await page.locator('[aria-label="对话列表面板"] [data-conversation-id]').getByRole("button", {name: "完善摘要", exact: true}).first().click();
  await page.getByRole("button", {name: "隐藏对话列表", exact: true}).click();
  await expect(panel(page).locator(".writing-agent-header-subtitle")).toHaveText("完善摘要");
  await expectSharedHeader(page);
  await panel(page).locator(".writing-agent-header").screenshot({path: ".tmp/agent-header-directory.png"});
  await page.getByRole("button", {name: "新建会话", exact: true}).click();
  await expect(panel(page).locator(".writing-agent-header-subtitle")).toHaveText("新对话");
  await expectSharedHeader(page);
});
