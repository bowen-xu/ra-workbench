import {expect, test} from "@playwright/test";

test("shared native link toolbar handles one-character and formatted links in production note and writing editors", async ({page}) => {
  const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
  await page.goto("/");
  await page.evaluate(async () => {
    const {mountLinkToolbarFixture} = await import("/integration/web/src/app/previews/fixtures/LinkToolbar.fixture.tsx");
    mountLinkToolbarFixture();
  });
  for (const name of ["普通笔记", "写作笔记"]) {
    const host = page.getByRole("region", {name, exact: true});
    await expect(host.locator('.bn-editor')).toBeVisible();
    const outside = host.getByText("Outside link", {exact: true});
    const toolbar = page.locator(".bn-link-toolbar");
    const samples = [host.locator('[data-id="body"] a').first(), host.locator('[data-id="body"] a').last(), host.locator("table a").first(), host.locator('a[href="https://example.org/format"]').first()];
    for (const anchor of samples) {
      await outside.click(); await host.getByRole("heading", {name, exact: true}).hover();
      await expect(toolbar).toHaveCount(0);
      // A real pointer hover, with no click/selection/drag over the link.
      await anchor.hover();
      const edit = toolbar.getByRole("button", {name: "编辑链接", exact: true});
      await expect(edit).toBeVisible();
      expect(await edit.evaluate(button => {const r = button.getBoundingClientRect(); return button.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));})).toBe(true);
      if (name === "普通笔记" && await anchor.innerText() === "3") await page.screenshot({path: test.info().outputPath("single-character-link-toolbar.png")});
      await edit.click();
      const address = page.getByPlaceholder("编辑链接地址", {exact: true});
      await expect(address).toBeVisible();
      await expect(address).toHaveValue(await anchor.getAttribute("href") ?? "");
      await expect(page.getByPlaceholder("编辑标题", {exact: true})).toHaveValue(await anchor.getAttribute("href") === "https://example.org/format" ? "Boldplain" : await anchor.innerText());
      await page.keyboard.press("Escape");
      await expect(address).toBeHidden();
    }
    await outside.click();
    await host.getByRole("button", {name: "切换只读", exact: true}).click();
    await expect(host.locator(".bn-editor")).toHaveAttribute("contenteditable", "false");
    await host.locator("table a").first().hover();
    await expect(toolbar).toHaveCount(0);
  }
  expect(errors).toEqual([]);
});

test("dragging inside note links selects native text without opening or moving content", async ({page}) => {
  await page.goto("/");
  await page.evaluate(async () => {
    const {getAppRuntime} = await import("/ra-workbench/apps/web/src/core/runtime/appRuntime.ts");
    const calls: string[] = [];
    Object.assign(window, {__linkSelectionOpenCalls: calls});
    Object.assign(getAppRuntime(), {browser: {open: async (url: string) => {calls.push(url);}}});
    const {mountLinkToolbarFixture} = await import("/integration/web/src/app/previews/fixtures/LinkToolbar.fixture.tsx");
    mountLinkToolbarFixture();
  });
  let opened = 0;
  for (const name of ["普通笔记", "写作笔记"]) {
    const host = page.getByRole("region", {name, exact: true});
    const editor = host.locator(".bn-editor"), original = await editor.textContent();
    const samples = [host.locator('[data-id="body"] a').last(), host.locator('table a[href="https://example.org/format"]').first()];
    for (const anchor of samples) {
      await host.getByText("Outside link", {exact: true}).click();
      await expect(anchor).toHaveAttribute("draggable", "false");
      const rect = await anchor.boundingBox(); if (!rect) throw new Error("Link geometry missing");
      expect(await anchor.evaluate(element => {const r = element.getBoundingClientRect(); return element.contains(document.elementFromPoint(r.x + r.width * .2, r.y + r.height / 2));})).toBe(true);
      await page.mouse.move(rect.x + rect.width * .2, rect.y + rect.height / 2);
      await expect(anchor).toHaveCSS("cursor", "pointer");
      await page.mouse.down();
      await expect(anchor).toHaveCSS("cursor", "pointer");
      await page.mouse.move(rect.x + rect.width * .8, rect.y + rect.height / 2, {steps: 8});
      await expect(anchor).toHaveCSS("cursor", "text");
      await page.mouse.up();
      await expect(anchor).toHaveCSS("cursor", "pointer");
      await expect.poll(() => page.evaluate(() => document.getSelection()?.toString().length ?? 0)).toBeGreaterThan(0);
      expect(await page.evaluate(() => (Reflect.get(window, "__linkSelectionOpenCalls") as string[]).length)).toBe(opened);
      await expect(editor).toHaveText(original ?? "");
      await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
      if (name === "普通笔记") await page.screenshot({path: test.info().outputPath("link-text-selection.png")});
      // The next deliberate click is independent of the completed selection.
      await host.getByText("Outside link", {exact: true}).click();
      await anchor.click(); opened++;
      expect(await page.evaluate(() => (Reflect.get(window, "__linkSelectionOpenCalls") as string[]).length)).toBe(opened);
    }
    // Existing toolbar keyboard activation stays independent of text selection.
    await host.getByText("Outside link", {exact: true}).click();
    await host.getByRole("heading", {name, exact: true}).hover();
    await expect(page.locator(".bn-link-toolbar")).toHaveCount(0);
    await samples[0]!.hover();
    const open = page.locator(".bn-link-toolbar").getByRole("button", {name: "在浏览器中打开", exact: true});
    await expect(open).toBeVisible(); await open.focus(); await open.press("Enter"); opened++;
    expect(await page.evaluate(() => (Reflect.get(window, "__linkSelectionOpenCalls") as string[]).length)).toBe(opened);
    await host.getByRole("button", {name: "切换只读", exact: true}).click();
    await expect(editor).toHaveAttribute("contenteditable", "false");
    for (const anchor of samples) {
      await host.getByText("Outside link", {exact: true}).click();
      const rect = await anchor.boundingBox(); if (!rect) throw new Error("Read-only link geometry missing");
      await page.mouse.move(rect.x + rect.width * .2, rect.y + rect.height / 2);
      await expect(anchor).toHaveCSS("cursor", "pointer"); await page.mouse.down();
      await page.mouse.move(rect.x + rect.width * .8, rect.y + rect.height / 2, {steps: 8});
      await expect(anchor).toHaveCSS("cursor", "text"); await page.mouse.up();
      await expect(anchor).toHaveCSS("cursor", "pointer");
      expect(await page.evaluate(() => document.getSelection()?.toString().length ?? 0)).toBeGreaterThan(0);
      expect(await page.evaluate(() => (Reflect.get(window, "__linkSelectionOpenCalls") as string[]).length)).toBe(opened);
    }
  }
});
