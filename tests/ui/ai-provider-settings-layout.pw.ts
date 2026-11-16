import {projectPath} from "./projectPaths";
import { expect, test, type Page } from "@playwright/test";

async function mountSettingsLayoutFixture(page: Page): Promise<void> {
  await page.setContent(`
    <main class="api-settings-dialog settings-api-dialog" style="width: 1100px; max-width: none; max-height: none; overflow: visible">
      <section class="api-provider-panel" style="width: 760px">
        <footer class="api-provider-actions">
          <button class="danger-primary" type="button">退出登录</button>
          <button type="button">刷新模型列表</button>
        </footer>
      </section>
      <section class="api-routing-panel">
        <div class="api-route-group">
          <div class="api-route-row" data-route="regular">
            <div><strong>解释所选内容</strong><small>默认继承“文本模型”</small></div>
            <select aria-label="普通 Provider"><option>继承默认</option></select>
            <input aria-label="普通模型" placeholder="先选择 Provider" />
          </div>
          <div class="api-route-row api-route-row-reasoning" data-route="openai">
            <div><strong>生成论文概览</strong><small>默认继承“文本模型”</small></div>
            <select aria-label="OpenAI Provider"><option>ChatGPT</option></select>
            <input aria-label="OpenAI 模型" value="gpt-5.6-luna" />
            <select aria-label="OpenAI 推理强度"><option>medium</option></select>
          </div>
        </div>
      </section>
    </main>
  `);
  await page.addStyleTag({ path: projectPath("apps/web/src/core/styles.css") });
  await page.addStyleTag({ path: projectPath("apps/web/src/core/features/settings/SettingsWorkspace.css") });
  await page.addStyleTag({ path: projectPath("apps/web/src/core/features/settings/AiProviderSettings.css") });
}

async function x(page: Page, selector: string): Promise<number> {
  const box = await page.locator(selector).boundingBox();
  if (box === null) throw new Error(`missing layout box for ${selector}`);
  return box.x;
}

test("keeps provider routes on one aligned desktop row", async ({ page }) => {
  await mountSettingsLayoutFixture(page);

  const regularRow = page.locator('[data-route="regular"]');
  const openAiRow = page.locator('[data-route="openai"]');
  const regularBox = await regularRow.boundingBox();
  const openAiBox = await openAiRow.boundingBox();
  expect(regularBox?.height).toBe(openAiBox?.height);

  expect(await x(page, '[aria-label="普通 Provider"]'))
    .toBeCloseTo(await x(page, '[aria-label="OpenAI Provider"]'), 1);
  expect(await x(page, '[aria-label="普通模型"]'))
    .toBeCloseTo(await x(page, '[aria-label="OpenAI 模型"]'), 1);

  const openAiControls = openAiRow.locator(":scope > select, :scope > input");
  const yPositions = await openAiControls.evaluateAll((elements) =>
    elements.map((element) => element.getBoundingClientRect().y));
  expect(new Set(yPositions.map((value) => Math.round(value))).size).toBe(1);
});

test("keeps ChatGPT action order and gives logout a muted red surface", async ({ page }) => {
  await mountSettingsLayoutFixture(page);

  const logout = page.getByRole("button", { name: "退出登录" });
  const refresh = page.getByRole("button", { name: "刷新模型列表" });
  const logoutBox = await logout.boundingBox();
  const refreshBox = await refresh.boundingBox();
  expect(logoutBox).not.toBeNull();
  expect(refreshBox).not.toBeNull();
  expect(logoutBox!.x).toBeLessThan(refreshBox!.x);

  const styles = await logout.evaluate((element) => {
    const computed = getComputedStyle(element);
    return { backgroundColor: computed.backgroundColor, color: computed.color };
  });
  expect(styles).toEqual({ backgroundColor: "rgb(246, 233, 227)", color: "rgb(162, 93, 80)" });
});

test("stacks route controls only at the narrow-screen breakpoint", async ({ page }) => {
  await page.setViewportSize({ width: 650, height: 720 });
  await mountSettingsLayoutFixture(page);

  const openAiControls = page.locator('[data-route="openai"] > select, [data-route="openai"] > input');
  const boxes = await openAiControls.evaluateAll((elements) => elements.map((element) => {
    const rect = element.getBoundingClientRect();
    return { x: rect.x, y: rect.y };
  }));
  expect(new Set(boxes.map(({ x: value }) => Math.round(value))).size).toBe(1);
  expect(new Set(boxes.map(({ y: value }) => Math.round(value))).size).toBe(boxes.length);
});
