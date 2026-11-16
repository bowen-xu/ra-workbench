import { expect, test } from "@playwright/test";

for (const route of ["/?demo=app", "/?preview=app"]) {
  test(`conversation directory synchronizes without a refresh control on ${route}`, async ({ page }) => {
    await page.goto(route);
    await page.getByRole("button", { name: "打开对话", exact: true }).click();
    await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("Catalog synchronization");
    await page.getByRole("button", { name: "发送消息", exact: true }).click();
    await expect(page.locator(".writing-agent-panel:visible")).toContainText("本轮已读取");
    await expect(page.getByRole("button", { name: "停止生成", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "对话列表", exact: true }).click();
    const drawer = page.getByRole("complementary", { name: "对话列表面板", exact: true });
    await expect(drawer.getByRole("button", { name: "刷新列表", exact: true })).toHaveCount(0);
    const selected = drawer.locator('[data-conversation-id]:has([aria-current="true"])');
    await expect(selected).toHaveCount(1);
    const id = (await selected.getAttribute("data-conversation-id"))!;
    const title = (await selected.locator("strong").textContent())!;
    // Change through a separate caller of the production gateway, not the list's callbacks.
    await page.evaluate(async ({ id, title }) => {
      const url = "/ra-workbench/apps/web/src/core/services/agentGateway.ts";
      const { agentCatalogGateway } = await import(/* @vite-ignore */ url);
      await agentCatalogGateway.rename({ id, expectedTitle: title, title: "Renamed elsewhere" });
    }, { id, title });
    await expect(selected.locator("strong")).toHaveText("Renamed elsewhere");
    await page.evaluate(async id => {
      const url = "/ra-workbench/apps/web/src/core/services/agentGateway.ts";
      const { agentArchiveGateway } = await import(/* @vite-ignore */ url);
      await agentArchiveGateway.setArchived({ id, archived: true });
    }, id);
    await expect(drawer.locator(`[data-conversation-id="${id}"]`)).toHaveCount(0);
    await drawer.getByRole("button", { name: "查看归档对话", exact: true }).click();
    const archived = drawer.locator(`[data-conversation-id="${id}"]`);
    await expect(archived.locator("strong")).toHaveText("Renamed elsewhere");
    // Returning to the foreground retains the archived view and reconciles metadata.
    await page.evaluate(async id => {
      const url = "/ra-workbench/apps/web/src/core/services/agentGateway.ts";
      const { agentCatalogGateway } = await import(/* @vite-ignore */ url);
      await agentCatalogGateway.rename({ id, expectedTitle: "Renamed elsewhere", title: "Updated while away" });
      window.dispatchEvent(new Event("focus"));
    }, id);
    await expect(archived.locator("strong")).toHaveText("Updated while away");
    await expect(drawer.getByRole("button", { name: "返回当前对话", exact: true })).toBeVisible();
    await page.screenshot({ path: `.tmp/conversation-catalog-sync-${route.includes("preview") ? "preview" : "app"}.png` });
  });
}
