import { expect, test, type Locator } from "@playwright/test";

async function expectReachable(target: Locator) {
  await expect(target).toBeVisible();
  await expect.poll(() => target.evaluate(element => {
    const box = element.getBoundingClientRect();
    const hit = element.ownerDocument.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
    return hit !== null && element.contains(hit) ? "target" : `${hit?.outerHTML.slice(0, 220)}; inert=${element.closest('[inert]')?.outerHTML.slice(0, 220)}`;
  })).toBe("target");
}

// Hints deliberately ignore pointer events. Temporarily enable hit testing to
// verify paint order; undo it before checking actual interaction behavior.
async function expectHintAboveOwner(hint: Locator) {
  await expect(hint).toBeVisible();
  await hint.evaluate(node => (node as HTMLElement).style.pointerEvents = "auto");
  await expectReachable(hint);
  await hint.evaluate(node => (node as HTMLElement).style.pointerEvents = "none");
}

test.beforeEach(async ({ page }) => {
  await page.goto("/@fs" + process.cwd() + "/tests/product-ui/fixtures/overlay.html");
});

test("notification hints own a scope above the toast and remain actionable with modal isolation", async ({page}) => {
  await page.getByRole("button", {name: "显示测试消息", exact: true}).click();
  await page.getByRole("button", {name: "打开弹窗", exact: true}).click();
  const action = page.getByRole("button", {name: "通知操作", exact: true});
  await expectReachable(action);
  await action.hover();
  await expectHintAboveOwner(page.getByRole("tooltip"));
  await action.click();
  // The notification is persistent and the parent modal stays open.
  await expect(page.getByLabel("通知操作次数")).toHaveText("1");
  await expect(page.getByRole("dialog", {name: "父弹窗", exact: true})).toBeVisible();
  await expect(action).toBeEnabled();
});

test("global notifications follow native top-layer ownership without resetting task actions", async ({page}) => {
  await page.getByRole("button", {name: "显示测试消息", exact: true}).click();
  await page.getByRole("button", {name: "打开原生弹窗", exact: true}).click();
  const action = page.getByRole("button", {name: "通知操作", exact: true});
  await expect.poll(() => action.evaluate(element => Boolean(element.closest("dialog:modal")))).toBe(true);
  await expectReachable(action);
  await action.hover();
  await expectHintAboveOwner(page.getByRole("tooltip"));
  await action.click();
  await page.getByRole("button", {name: "关闭原生弹窗", exact: true}).click();
  await expect.poll(() => action.evaluate(element => Boolean(element.closest("dialog")))).toBe(false);
  await expectReachable(action);
  await action.click();
  await expect(page.getByLabel("通知操作次数")).toHaveText("2");
  await page.getByRole("button", {name: "打开原生弹窗", exact: true}).click();
  await expect.poll(() => action.evaluate(element => Boolean(element.closest("dialog:modal")))).toBe(true);
  await page.getByRole("button", {name: "移除原生弹窗", exact: true}).click();
  await expectReachable(action);
  await expect.poll(() => action.evaluate(element => Boolean(element.closest("dialog")))).toBe(false);
  await action.click();
  await expect(page.getByLabel("通知操作次数")).toHaveText("3");
});

test("page and modal popups escape clipped transforms, remain clickable and restore focus", async ({ page }) => {
  for (const name of ["页面", "父"]) {
    if (name === "父") await page.getByRole("button", {name: "打开弹窗", exact: true}).click();
    const trigger = page.getByRole("button", {name: `${name}菜单`, exact: true});
    await trigger.click();
    const action = page.getByRole("menuitem", {name: "执行", exact: true});
    await expectReachable(action);
    await action.hover();
    await expectHintAboveOwner(page.getByRole("tooltip").filter({hasText: `${name}菜单项提示`}));
    await action.click();
    await expect(page.getByLabel(`${name}结果`)).toHaveText("完成");
    await expect(trigger).toBeFocused();
    const source = page.getByRole("button", {name: `${name}来源`, exact: true});
    await source.hover();
    await expectHintAboveOwner(page.getByRole("tooltip").filter({hasText: `${name}来源提示`}));
    const select = page.getByRole("combobox", {name: `${name}选择`});
    await select.click();
    await expectReachable(page.getByRole("option", {name: "乙", exact: true}));
    await page.getByRole("option", {name: "乙", exact: true}).click();
    await expect(select).toHaveText("乙");
  }
  await page.getByRole("button", {name: "更多阅读审阅操作"}).click();
  const keep = page.getByRole("menuitem", {name: "Keep all", exact: true});
  await expectReachable(keep);
  await keep.click();
  await expect(page.getByLabel("审阅结果")).toHaveText("keep:all");
  await page.screenshot({path: ".tmp/overlay-modal.png"});
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", {name: "父弹窗"})).toHaveCount(0);
  await expect(page.getByRole("button", {name: "打开弹窗", exact: true})).toBeFocused();
  await page.getByRole("button", {name: "打开弹窗", exact: true}).click();
  await page.getByRole("button", {name: "父菜单", exact: true}).click();
  await expectReachable(page.getByRole("menuitem", {name: "执行", exact: true}));
});

test("nested modal owns its popups and Escape dismisses only the active layer", async ({ page }) => {
  await page.getByRole("button", {name: "打开弹窗", exact: true}).click();
  await page.getByRole("button", {name: "父菜单", exact: true}).click();
  await page.getByRole("menuitem", {name: "打开子弹窗", exact: true}).click();
  await expectReachable(page.getByRole("button", {name: "子菜单", exact: true}));
  await page.getByRole("button", {name: "子菜单", exact: true}).click();
  await expectReachable(page.getByRole("menuitem", {name: "执行", exact: true}));
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(page.getByRole("dialog", {name: "子弹窗", exact: true})).toBeVisible();
  await page.getByRole("button", {name: "关闭子弹窗", exact: true}).focus();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", {name: "子弹窗", exact: true})).toHaveCount(0);
  await expect(page.getByRole("dialog", {name: "父弹窗", exact: true})).toBeVisible();
  await expectReachable(page.getByRole("button", {name: "父菜单", exact: true}));
});

test("nested menus stack above their parent and stay within the resized viewport", async ({page}) => {
  await page.getByRole("button", {name: "打开弹窗", exact: true}).click();
  await page.getByRole("button", {name: "父菜单", exact: true}).click();
  await page.getByRole("menuitem", {name: "父次级菜单", exact: true}).click();
  const action = page.getByRole("menuitem", {name: "执行次级", exact: true});
  await expectReachable(action);
  await page.setViewportSize({width: 640, height: 600});
  await expectReachable(action);
  const box = await action.boundingBox();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(640);
  await action.click();
  await expect(page.getByLabel("父结果")).toHaveText("次级完成");
});

test("portrait review menu and source hints remain above the actual modal", async ({page}) => {
  const url = "/?preview=app";
  await page.goto(url);
  await expect(page.getByRole("row", {name: /Synthetic Research Preview/})).toBeVisible();
  const paperTitle = await page.evaluate(async () => {
    const module = "/ra-workbench/apps/web/src/core/services/structuredResourceGateway.ts";
    const {getStructuredResourceGateway} = await import(/* @vite-ignore */ module);
    const aiModule = "/ra-workbench/apps/web/src/core/services/aiAssistanceGateway.ts";
    const {getAiAssistanceGateway} = await import(/* @vite-ignore */ aiModule);
    const candidate = (await getAiAssistanceGateway().listCandidates()).find(value => value.candidateKind === "paper_portrait_v1");
    if (!candidate) throw new Error("Missing portrait fixture");
    const evidence = candidate.evidenceBundle.items[0];
    const gateway = getStructuredResourceGateway();
    const resources = await gateway.list(evidence.paperId);
    let portrait = resources.find(resource => resource.kind === "portrait");
    if (!portrait) throw new Error("Missing synthetic portrait");
    if (!portrait.content) {
      const payload = candidate.revisions.find(value => value.revision === candidate.currentRevision).payload;
      portrait = (await gateway.write({resourceId: portrait.id, expectedRevision: portrait.revision,
        content: JSON.stringify(payload), requestId: crypto.randomUUID(), editorKind: "agent"})).resource;
    }
    await gateway.getSources(portrait.id);
    const content = structuredClone(portrait.content);
    content.claims[0].description += " 浮层回归验证。";
    content.claims[0].evidence = [{evidenceItemId: evidence.citationKey, quote: evidence.quoteExact}];
    const result = await gateway.write({resourceId: portrait.id, expectedRevision: portrait.revision,
      content: JSON.stringify(content), requestId: crypto.randomUUID(), editorKind: "agent"});
    if (result.status !== "applied") throw new Error(JSON.stringify(result.errors));
    return evidence.paperTitle;
  });
  await page.getByRole("row").filter({hasText: paperTitle}).click();
  await page.getByRole("button", {name: "查看画像详情", exact: true}).click();
  const dialog = page.getByRole("dialog", {name: "文献画像详情", exact: true});
  const source = dialog.locator(".reading-field-sources button").first();
  await source.scrollIntoViewIfNeeded();
  await source.hover();
  await expectHintAboveOwner(page.getByRole("tooltip"));
  const review = dialog.getByRole("region", {name: "文献画像", exact: true});
  await review.getByRole("button", {name: "更多阅读审阅操作", exact: true}).click();
  const keep = page.getByRole("menuitem", {name: "Keep all", exact: true});
  await expectReachable(keep);
  await keep.hover();
  await expectHintAboveOwner(page.getByRole("tooltip"));
  await page.screenshot({path: ".tmp/overlay-portrait-preview.png"});
  await keep.click();
  await expect(review.getByRole("button", {name: "更多阅读审阅操作", exact: true})).toHaveCount(0);
});

test("native dialog top layer retains its popups and nested modal portals", async ({ page }) => {
  await page.getByRole("button", {name: "打开原生弹窗", exact: true}).click();
  await page.getByRole("button", {name: "原生菜单", exact: true}).click();
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  expect(await menu.evaluate(node => Boolean(node.closest("dialog")))).toBe(true);
  await expectReachable(page.getByRole("menuitem", {name: "执行", exact: true}));
  await page.getByRole("menuitem", {name: "执行", exact: true}).click();
  await page.getByRole("button", {name: "原生来源", exact: true}).hover();
  await expectHintAboveOwner(page.getByRole("tooltip"));
  await page.getByRole("button", {name: "原生内打开子弹窗", exact: true}).click();
  await page.getByRole("button", {name: "原生子菜单", exact: true}).click();
  await expectReachable(page.getByRole("menuitem", {name: "执行", exact: true}));
});

test("command palette keeps keyboard execution, backdrop dismissal and focus restoration", async ({page}) => {
  const trigger = page.getByRole("button", {name: "打开命令", exact: true});
  await trigger.click();
  await expect(page.getByRole("textbox", {name: "搜索写作命令"})).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", {name: "LaTeX 写作命令"})).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.mouse.click(10, 10);
  await expect(page.getByRole("dialog", {name: "LaTeX 写作命令"})).toHaveCount(0);
});

test("API settings use the shared modal, restore focus on Escape and keep hints above the surface", async ({page}) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "设置", exact: true}).click();
  const trigger = page.getByRole("button", {name: "管理", exact: true});
  await trigger.click();
  const dialog = page.getByRole("dialog", {name: "LLM API 设置", exact: true});
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", {name: "关闭 API 设置", exact: true}).focus();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await dialog.getByRole("button", {name: "API 配置与隐私说明", exact: true}).hover();
  await expectHintAboveOwner(page.getByRole("tooltip"));
});
