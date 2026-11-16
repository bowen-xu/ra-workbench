import {expect, test} from "@playwright/test";

for (const width of [1280, 600]) test(`catalog-only version selection and modal overlay at ${width}px`, async ({page}) => {
  await page.setViewportSize({width, height: 820});
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "设置", exact: true}).waitFor();
  await page.evaluate(async () => {
    const [react, dom, settings] = await Promise.all([
      "/node_modules/.vite/deps/react.js", "/node_modules/.vite/deps/react-dom_client.js", "/ra-workbench/apps/web/src/core/features/settings/AiProviderSettings.tsx",
    ].map(path => import(path)));
    const h = (react.default ?? react).createElement;
    const host = document.createElement("div"); host.id = "version-settings-fixture";
    document.getElementById("root")!.hidden = true; document.body.append(host);
    let state = {runtimeVersion: "0.146.0", selectedVersion: "0.146.0", availableVersions: ["0.146.0"]};
    const versionGateway = {
      getVersions: async (refresh: boolean) => {
        if (refresh) {state = {...state, availableVersions: ["0.200.0", "0.146.0"]}; host.dataset.refreshes = "1";}
        return state;
      },
      selectVersion: async (version: string) => {state = {...state, selectedVersion: version}; host.dataset.selected = version; return state;},
    };
    const gateway = {
      getLlmConfiguration: async () => ({providers: [{providerId: "openai", displayName: "ChatGPT / Codex", authMode: "chatgpt-login", protocol: "codex-cli", modelKinds: ["text"], defaultBaseUrls: [], baseUrl: "", usesDefaultBaseUrl: false, enabled: true, timeoutMs: 120000, credentialConfigured: true}], defaultModels: [], useCases: []}),
      getChatGptAccountStatus: async () => ({runtimeAvailable: true, configured: true, authType: "chatgpt", email: "reader@example.com"}),
      fetchLlmProviderModels: async () => {host.dataset.modelVersion = state.selectedVersion; return {providerId: "openai", models: []};},
    };
    (dom.default ?? dom).createRoot(host).render(h(settings.AiProviderSettings, {desktopAvailable: true, gateway, versionGateway, onClose: () => {host.dataset.closed = "true";}}));
  });
  const dialog = page.getByRole("dialog", {name: "LLM API 设置"});
  await expect(dialog).toBeVisible();
  const select = dialog.getByRole("combobox", {name: "Codex CLI 版本号", exact: true});
  await expect(select).toHaveText("0.146.0");
  const refresh = dialog.getByRole("button", {name: "刷新 Codex CLI 版本号", exact: true});
  await expect(refresh).toHaveCSS("border-top-width", "0px");
  await expect(refresh).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(refresh).toHaveCSS("width", "28px");
  await expect(select).toHaveCSS("border-radius", "11px");
  await expect(dialog).toContainText("仅供获取模型列表使用，不表示更新版本或使用的实际版本");
  await expect(dialog.locator(".api-provider-panel-heading small")).toContainText("Codex app-server 0.146.0（仅用于登录与账号状态）");
  await dialog.getByRole("button", {name: "刷新 Codex CLI 版本号", exact: true}).click();
  await expect(page.locator("#version-settings-fixture")).toHaveAttribute("data-refreshes", "1");
  await expect(select).toHaveText("0.146.0");
  await select.click();
  const list = page.getByRole("listbox", {name: "可用 Codex CLI 版本"});
  await expect(list).toBeVisible();
  await expect(list).toHaveCSS("border-radius", "20px");
  const option = list.getByRole("option", {name: "0.200.0", exact: true});
  const painted = await option.evaluate(el => {const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));});
  expect(painted).toBe(true);
  const scope = await list.evaluate(el => el.closest('[data-overlay-scope="modal"]') !== null); expect(scope).toBe(true);
  const box = await list.boundingBox(); expect(box!.x).toBeGreaterThanOrEqual(0); expect(box!.x + box!.width).toBeLessThanOrEqual(width);
  await page.screenshot({path: `.tmp/codex-catalog-version-${width}.png`});
  await page.keyboard.press("Escape"); await expect(list).toBeHidden(); await expect(dialog).toBeVisible(); await expect(select).toBeFocused();
  await select.click(); await option.click();
  await expect(select).toHaveText("0.200.0");
  await expect(page.locator("#version-settings-fixture")).toHaveAttribute("data-model-version", "0.200.0");
  await expect(dialog.locator(".api-provider-panel-heading small")).toContainText("Codex app-server 0.146.0（仅用于登录与账号状态）");
  await expect(list).toBeHidden();
});

test("the actual app preview supplies versions without native IPC or external discovery", async ({page}) => {
  const external: string[] = [];
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    if (url.origin !== new URL(test.info().project.use.baseURL as string).origin) {
      external.push(url.href); await route.abort();
    } else await route.continue();
  });
  await page.addInitScript(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {value: {invoke: () => {throw new Error("Unexpected native IPC");}}, configurable: true});
  });
  await page.goto("/?preview=app&demo=text-patch");
  async function openSettings() {
    await page.getByRole("button", {name: "设置", exact: true}).click();
    await page.getByRole("group", {name: "AI 与模型", exact: true}).getByRole("button", {name: "管理", exact: true}).click();
    return page.getByRole("dialog", {name: "LLM API 设置", exact: true});
  }
  const dialog = await openSettings();
  const select = dialog.getByRole("combobox", {name: "Codex CLI 版本号", exact: true});
  await expect(select).toHaveText("0.146.0");
  await dialog.getByRole("button", {name: "刷新 Codex CLI 版本号", exact: true}).click();
  await expect(select).toHaveText("0.146.0");
  await select.click();
  await page.getByRole("option", {name: "0.200.0", exact: true}).click();
  await expect(select).toHaveText("0.200.0");
  await expect(dialog.locator(".api-provider-panel-heading small").filter({hasText: "OpenAI 官方 Codex"})).toContainText("Codex app-server 0.146.0（仅用于登录与账号状态）");
  await page.screenshot({path: ".tmp/codex-catalog-version-app-preview.png"});
  await page.reload();
  const reopened = await openSettings();
  await expect(reopened.getByRole("combobox", {name: "Codex CLI 版本号", exact: true})).toHaveText("0.200.0");
  await expect(reopened).not.toContainText("读取失败");
  expect(external).toEqual([]);
});
