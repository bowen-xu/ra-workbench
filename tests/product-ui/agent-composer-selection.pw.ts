import { expect, test } from "@playwright/test";

test("reference labels share the surrounding text baseline without enlarging the line", async ({ page }) => {
  for (const width of [1172, 632]) {
    await page.setViewportSize({ width, height: 808 });
    await page.goto('/?demo=writing-editor-diff');
    const input = page.getByRole('textbox', { name: '给 AI 写作助手发送消息' });
    await input.fill('保存到文献笔记 ');
    const originalHeight = await input.evaluate(element => parseFloat(getComputedStyle(element).lineHeight));
    await page.getByRole('button', { name: '添加上下文和工具', exact: true }).click();
    await page.getByRole('menuitem', {name:'文件',exact:true}).click();
    await page.getByRole('menuitem', { name: 'main.tex 当前稿件', exact: true }).click();
    await input.press('End');
    await input.pressSequentially(' 保存');
    const geometry = await input.locator('.agent-composer-reference').evaluate(tag => {
      const line = tag.closest('.cm-line')!;
      const label = tag.querySelector('.agent-composer-reference-label')!;
      const text = [...line.childNodes].find(node => node.nodeType === Node.TEXT_NODE && node.textContent?.trim())!;
      const rect = (node: Node) => {
        const range = document.createRange(); range.selectNodeContents(node);
        return range.getBoundingClientRect();
      };
      const labelRect = rect(label), textRect = rect(text);
      const icon = tag.querySelector('svg')!.getBoundingClientRect(), bounds = tag.getBoundingClientRect();
      return { topOffset: labelRect.top - textRect.top, bottomOffset: labelRect.bottom - textRect.bottom,
        iconOffset: (icon.top + icon.bottom - bounds.top - bounds.bottom) / 2,
        lineHeight: line.getBoundingClientRect().height };
    });
    expect(Math.abs(geometry.topOffset)).toBeLessThanOrEqual(.5);
    expect(Math.abs(geometry.bottomOffset)).toBeLessThanOrEqual(.5);
    expect(Math.abs(geometry.iconOffset)).toBeLessThanOrEqual(.5);
    expect(Math.abs(geometry.lineHeight - originalHeight)).toBeLessThan(.5);
  }
});

test("reference selection includes its icon for keyboard and pointer selection", async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  for (const width of [1172, 632]) {
    await page.setViewportSize({ width, height: 808 });
    await page.goto('/?demo=writing-editor-diff');
    await page.getByRole('textbox', { name: '给 AI 写作助手发送消息' }).fill('啊 ');
    await page.getByRole('button', { name: '添加上下文和工具', exact: true }).click();
    await page.getByRole('menuitem', {name:'文件',exact:true}).click();
    await page.getByRole('menuitem', { name: 'main.tex 当前稿件', exact: true }).click();
    const editor = page.locator('.agent-composer-inline-input .cm-content');
    await editor.press('End');
    await editor.pressSequentially(' hi');
    await editor.press('ControlOrMeta+A');
    const reference = editor.locator('.agent-composer-reference');
    const covered = async () => reference.evaluate(element => {
      const icon = element.querySelector('svg')!.getBoundingClientRect();
      const label = element.getBoundingClientRect();
      return [...element.closest('.cm-editor')!.querySelectorAll('.cm-selectionBackground')].some(marker => {
        const rect = marker.getBoundingClientRect();
        return rect.left <= icon.left + 1 && rect.right >= label.right - 1
          && rect.top <= icon.top + 1 && rect.bottom >= icon.bottom - 1;
      });
    });
    await expect.poll(covered).toBe(true);
    await editor.press('ControlOrMeta+C');
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toContain('@main.tex');
    expect(copied).toContain('啊');
    expect(copied).toContain(' hi');
    await editor.press('ArrowRight');
    await expect(page.locator('.agent-composer-inline-input .cm-selectionBackground')).toHaveCount(0);
    const range = await editor.evaluate(element => {
      const line = element.querySelector('.cm-line')!;
      const rect = document.createRange();
      rect.selectNodeContents(line);
      const box = rect.getBoundingClientRect();
      return { left: box.left, right: box.right, y: (box.top + box.bottom) / 2 };
    });
    await page.mouse.move(range.left, range.y);
    await page.mouse.down();
    await page.mouse.move(range.right + 1, range.y, { steps: 12 });
    await page.mouse.up();
    await expect.poll(covered).toBe(true);
    await page.screenshot({ path: `.tmp/composer-reference-selection-${width}.png`, fullPage: true });
    await editor.press('Backspace');
    await expect(reference).toHaveCount(0);
    await editor.press('ControlOrMeta+Z');
    await expect(reference).toHaveCount(1);
  }
});
