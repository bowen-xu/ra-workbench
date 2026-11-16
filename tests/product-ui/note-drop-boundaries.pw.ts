import { expect, test } from "@playwright/test";

for (const precedingText of ["", "Above"]) {
  test(`one drop boundary on both sides of ${precedingText === "" ? "an empty" : "a paragraph"} block / quote gap`, async ({ page }) => {
    await page.goto("/");
    await page.evaluate(() => window.localStorage.clear());
    await page.reload();
    await page.getByRole("button", { name: "收起主导航" }).click({ force: true });
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.getByRole("button", { name: "新建页面" }).first().click();
    const editor = page.locator(".bn-editor");
    await editor.click();
    await page.keyboard.type("Top");
    await page.keyboard.press("Enter");
    if (precedingText !== "") await page.keyboard.type(precedingText);
    await page.keyboard.press("Enter");
    await page.keyboard.type('" Quote');
    await page.keyboard.press("Enter");
    await page.keyboard.type("Source");
    const blocks = page.locator(".bn-block-content");
    await expect(blocks).toHaveText(["Top", precedingText, "Quote", "Source"]);
    await expect(blocks.nth(2)).toHaveAttribute("data-content-type", "quote");
    const ids = () => page.locator(".bn-editor .bn-block[data-id]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")));
    const initialIds = await ids();
    // Separate fixture typing from the move's undo transaction.
    await page.waitForTimeout(600);

    for (const releaseSide of ["preceding", "quote"]) {
      await blocks.nth(3).hover();
      const h = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
      const preceding = await blocks.nth(1).boundingBox();
      const quote = await blocks.nth(2).boundingBox();
      if (h === null || preceding === null || quote === null) throw new Error("Missing block geometry");
      await page.mouse.move(h.x + 16, h.y + 16);
      await page.mouse.down();
      await page.mouse.move(preceding.x + 100, preceding.y + preceding.height - 2, { steps: 8 });
      const cursor = page.locator(".note-block-drop-cursor");
      await expect(cursor).toBeVisible();
      const line = await cursor.boundingBox();
      if (line === null) throw new Error("Missing drop line");
      // Cross the same boundary repeatedly from text and the handle gutter.
      for (const inset of [100, -12, 100, -12]) {
        for (const y of [quote.y + 2, preceding.y + preceding.height - 2]) {
          await page.mouse.move(quote.x + inset, y, { steps: 3 });
          await expect(cursor).toBeVisible();
          await expect.poll(() => cursor.boundingBox()).toEqual(line);
        }
      }
      const releaseY = releaseSide === "quote" ? quote.y + 2 : preceding.y + preceding.height - 2;
      await page.mouse.move(quote.x + 100, releaseY);
      await page.mouse.up();
      await expect.poll(ids).toEqual([initialIds[0], initialIds[1], initialIds[3], initialIds[2]]);
      await expect(cursor).toHaveCount(0);
      await page.keyboard.press("ControlOrMeta+z");
      await expect.poll(ids).toEqual(initialIds);
    }
  });
}
