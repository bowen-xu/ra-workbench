import { expect, test } from "@playwright/test";

for (const route of ["?demo=app", "?preview=app"]) test(`${route} automatically recovers from an obsolete selected conversation without a legacy creation gate`, async ({page}) => {
  const prefix = "research-assistant.app-preview.v3:";
  const oldId = "11111111-1111-4111-8111-111111111111";
  await page.addInitScript(({prefix, oldId}) => {
    const marker = "startup-regression-seeded";
    if (sessionStorage.getItem(marker)) return;
    sessionStorage.setItem(marker, "true");
    localStorage.setItem(prefix + "research-assistant:independent-selected", oldId);
  }, {prefix, oldId});
  await page.goto(`/${route}`);
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  const panel = page.locator(".writing-agent-panel:visible");
  const input = page.getByRole("textbox", {name: "给研究助手发送消息", exact: true});
  await expect(input).toBeEnabled();
  await expect.poll(() => page.evaluate(prefix => localStorage.getItem(prefix + "research-assistant:independent-selected"), prefix)).not.toBe(oldId);
  await expect(panel).toContainText("研究助手为您服务");
  await expect(panel.getByRole("button", {name: "新建对话", exact: true})).toHaveCount(0);
  await expect(panel).not.toContainText("无法核对对话及上下文");
  await input.fill("Unsent startup draft");
  await page.reload();
  const opener = page.getByRole("button", {name: "打开对话", exact: true});
  await opener.click();
  await expect(input).toBeEnabled();
  await expect(input).toHaveValue("Unsent startup draft");
  await page.screenshot({path: `.tmp/context-startup-${route.includes("demo") ? "demo" : "preview"}.png`});
});
