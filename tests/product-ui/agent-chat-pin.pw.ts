import {expect, test} from "@playwright/test";

test("conversation list remains clickable above an overlapping chat in the App", async ({page}) => {
    await page.setViewportSize({width: 717, height: 486});
    await page.goto("/?preview=app");
    const launcher = page.getByRole("button", {name: "打开对话", exact: true});
    await launcher.click();
    await page.getByRole("button", {name: "对话列表", exact: true}).click();
    const drawer = page.getByRole("complementary", {name: "对话列表面板"});
    await expect(drawer).toBeVisible();
    await expect.poll(() => drawer.evaluate(element => {
      const panel = document.querySelector<HTMLElement>(".writing-agent-panel:not([hidden])");
      if (!panel) return false;
      const a = element.getBoundingClientRect(), b = panel.getBoundingClientRect();
      const left = Math.max(a.left, b.left), right = Math.min(a.right, b.right);
      const top = Math.max(a.top, b.top), bottom = Math.min(a.bottom, b.bottom);
      if (right <= left || bottom <= top) return false;
      const hit = document.elementFromPoint((left + right) / 2, (top + bottom) / 2);
      return !!hit && element.contains(hit);
    })).toBe(true);
    await drawer.getByRole("button", {name: "隐藏对话列表", exact: true}).click();
    await expect(drawer).toBeHidden();
  });

for (const action of ["outside", "manual"] as const) {
  test(`${action} dismissal hides the chat and conversation list together without waiting for layout transitions`, async ({page}) => {
    await page.setViewportSize({width: 1434, height: 898});
    await page.goto("/?preview=app");
    await page.getByRole("button", {name: "打开对话", exact: true}).click();
    await page.getByRole("button", {name: "对话列表", exact: true}).click();
    await expect(page.getByRole("complementary", {name: "对话列表面板", exact: true})).toBeVisible();
    const samples = await page.evaluateHandle(() => {
      const panel = document.querySelector<HTMLElement>(".writing-agent-panel:not([hidden])")!;
      const drawer = document.querySelector<HTMLElement>(".agent-canvas-conversations:not([hidden])")!;
      const state = {done: false, frames: [] as {panelHidden: boolean; listHidden: boolean}[]};
      window.addEventListener("click", () => {
        const frame = () => {
          state.frames.push({panelHidden: getComputedStyle(panel).display === "none", listHidden: getComputedStyle(drawer).display === "none"});
          if (state.frames.length < 20) requestAnimationFrame(frame);
          else state.done = true;
        };
        requestAnimationFrame(frame);
      }, {once: true, capture: true});
      return state;
    });
    if (action === "outside") await page.mouse.click(1350, 350);
    else await page.getByRole("button", {name: "隐藏对话", exact: true}).click();
    await expect.poll(() => samples.evaluate(state => state.done)).toBe(true);
    const frames = await samples.evaluate(state => state.frames);
    expect(frames.filter(frame => frame.panelHidden !== frame.listHidden)).toEqual([]);
    expect(frames.slice(0, 2).some(frame => frame.panelHidden && frame.listHidden)).toBe(true);
    await expect(page.getByRole("button", {name: "打开对话", exact: true})).toBeVisible();
    await samples.dispose();
  });
}

test("pin prevents automatic collapse, retains background interaction and still allows manual hide", async ({page}) => {
  await page.setViewportSize({width: 1434, height: 898});
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  const panel = page.locator(".writing-agent-panel:visible");
  const pin = page.getByRole("button", {name: "置顶聊天窗", exact: true});
  await expect(pin).toHaveAttribute("aria-pressed", "false");
  await pin.click();
  const unpin = page.getByRole("button", {name: "取消置顶聊天窗", exact: true});
  await expect(unpin).toHaveAttribute("aria-pressed", "true");
  const draft = panel.getByRole("textbox").last();
  await draft.fill("保留置顶草稿");
  const node = await draft.elementHandle();
  await page.getByRole("button", {name: "文献", exact: true}).click();
  await expect(panel).toBeVisible();
  await page.evaluate(() => window.dispatchEvent(new Event("research-assistant:text-document-revealed")));
  await expect(panel).toBeVisible();
  await unpin.hover();
  const hint = page.getByRole("tooltip").filter({hasText: "取消置顶，恢复自动收起"});
  await expect(hint).toBeVisible();
  expect(await hint.evaluate(el => Boolean(el.closest('[data-overlay-scope="panel"]')))).toBe(true);
  const bounds = await unpin.boundingBox();
  expect(await unpin.evaluate(el => {const r = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));})).toBe(true);
  expect(bounds!.width).toBeGreaterThan(0);
  await page.screenshot({path: ".tmp/agent-chat-pinned.png"});
  await page.getByRole("button", {name: "隐藏对话", exact: true}).click();
  await expect(panel).toHaveCount(0);
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await expect(unpin).toHaveAttribute("aria-pressed", "true");
  await expect(draft).toHaveValue("保留置顶草稿");
  expect(await node?.evaluate(el => el.isConnected)).toBe(true);
  await page.setViewportSize({width: 760, height: 650});
  await expect(unpin).toBeVisible();
  const narrow = await unpin.boundingBox();
  expect(narrow!.x).toBeGreaterThanOrEqual(0);
  expect(narrow!.x + narrow!.width).toBeLessThanOrEqual(760);
  await unpin.click();
  await page.getByRole("button", {name: "文献", exact: true}).click();
  await expect(panel).toHaveCount(0);
});

test("text-patch shares the pin control and retains it after hiding", async ({page}) => {
    await page.goto("/?demo=text-patch");
    await page.getByRole("button", {name: "置顶聊天窗", exact: true}).click();
    await page.getByRole("button", {name: /^(隐藏对话|隐藏 AI 写作助手)$/u}).click();
    await page.getByRole("button", {name: "打开对话", exact: true}).click();
    await expect(page.getByRole("button", {name: "取消置顶聊天窗", exact: true})).toHaveAttribute("aria-pressed", "true");
  });
