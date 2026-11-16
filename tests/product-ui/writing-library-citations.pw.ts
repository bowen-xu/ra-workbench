import { expect, test } from "@playwright/test";

test("library picker and inline title completion automatically maintain one managed bibliography", async ({page}) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "写作", exact: true}).click();
  await page.getByRole("button", {name: "新建稿件", exact: true}).click();
  await page.getByRole("dialog", {name: "新建 LaTeX 稿件"}).getByRole("textbox").fill("文献库自动引用验收");
  await page.getByRole("button", {name: "创建稿件", exact: true}).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toContainText("\\documentclass");
  await editor.click();
  await editor.press("ControlOrMeta+Home");
  for (let line = 0; line < 4; line++) await editor.press("ArrowDown");
  await editor.press("Home");

  async function chooseLibraryArticle() {
    await page.getByRole("button", {name: "插入引用", exact: true}).click();
    const picker = page.getByRole("dialog", {name: "插入文献引用", exact: true});
    await expect(picker.getByLabel("引用搜索范围")).toHaveValue("library");
    await picker.getByRole("textbox", {name: "搜索引用"}).fill("Synthetic Research Preview");
    await expect(picker.getByRole("checkbox")).toHaveCount(1);
    await picker.getByRole("checkbox").check();
    await picker.getByRole("button", {name: "插入引用", exact: true}).click();
    await expect(picker).toHaveCount(0);
  }

  await chooseLibraryArticle();
  const firstReference = editor.locator('.writing-citation-reference[data-reference-id="binding:10000000-0000-4000-8000-000000000001"]');
  await expect(firstReference).toHaveCount(1);
  await chooseLibraryArticle();
  await expect(firstReference).toHaveCount(2);
  await expect(firstReference.first()).toHaveAttribute("aria-label", /\(Preview, 2026\) Synthetic Research Preview/);
  await expect(firstReference.first().locator("svg")).toHaveCount(0);
  await expect(firstReference.first().locator(".writing-citation-title")).toHaveText("Synthetic…");
  expect(await firstReference.first().locator(".writing-citation-title").evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
  const alignment = await firstReference.first().evaluate(element => {
    const textRect = (node: Node) => {
      const range = document.createRange(); range.selectNodeContents(node);
      return range.getBoundingClientRect().top;
    };
    const command = element.closest(".cm-line")?.querySelector("span");
    const author = element.querySelector(".writing-citation-author");
    const title = element.querySelector(".writing-citation-title");
    if (!command || !author || !title) throw new Error("Missing citation line contents");
    return [textRect(command), textRect(author), textRect(title)];
  });
  expect(Math.max(...alignment) - Math.min(...alignment)).toBeLessThanOrEqual(1);
  await page.screenshot({path: ".tmp/citation-tags-preview.png"});
  // Removing the source delimiter must not expose a managed key as raw text.
  await firstReference.first().click();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Delete");
  await expect(firstReference).toHaveCount(2);
  await expect(firstReference.first().locator(".writing-citation-title")).toHaveText("Synthetic…");
  await page.screenshot({path: ".tmp/citation-missing-brace-preview.png"});
  await page.keyboard.insertText("}");
  await firstReference.first().click();
  await page.keyboard.press("Backspace");
  await expect(firstReference).toHaveCount(1);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(firstReference).toHaveCount(2);

  await editor.press("ControlOrMeta+End");
  await editor.press("Enter");
  await editor.pressSequentially("\\cite{Mock");
  await expect(page.locator(".writing-citation-completions completion-section")).toHaveText("源自文献库");
  await page.getByRole("option", {name: /Mock Methods and Reproducibility/}).click();
  const secondReference = editor.locator('.writing-citation-reference').filter({hasText:"Mock…"});
  await expect(secondReference).toHaveCount(1);
  await expect(firstReference.first()).toHaveAttribute("data-citation-key",/preview2026:[ab]/u);
  await expect(secondReference).toHaveAttribute("data-citation-key",/preview2026:[ab]/u);
  const firstKey = await firstReference.first().getAttribute("data-citation-key");
  const secondKey = await secondReference.getAttribute("data-citation-key");
  expect(firstKey).not.toBe(secondKey);
  await expect(firstReference.first()).toHaveAttribute("aria-label",/2026[ab]/u);
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await editor.press("ControlOrMeta+a");
  await editor.press("ControlOrMeta+c");
  const source = await page.evaluate(() => navigator.clipboard.readText());
  expect(source).toContain(`\\cite{${firstKey}}\\cite{${firstKey}}`);
  expect(source).toContain(`\\cite{${secondKey}}`);
  expect(source).not.toContain("(Preview, 2026)");
  await page.screenshot({path:".tmp/citation-key-disambiguation-preview.png"});
  // Removing the last occurrence of B collapses A to its bare key. Undo must
  // restore B's stable identity, then regenerate both disambiguation suffixes.
  await secondReference.click();await page.keyboard.press("Backspace");
  await expect(secondReference).toHaveCount(0);
  await expect(firstReference.first()).toHaveAttribute("data-citation-key","preview2026");
  await expect(firstReference.first()).toHaveAttribute("aria-label",/\(Preview, 2026\)/u);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(secondReference).toHaveCount(1);
  await expect(firstReference.first()).toHaveAttribute("data-citation-key",firstKey!);
  await expect(secondReference).toHaveAttribute("data-citation-key",secondKey!);
  await page.locator(".writing-file-row").getByRole("button", {name: "references.bib", exact: true}).click();
  await expect(editor).toHaveAttribute("contenteditable", "false");
  await expect(editor).toContainText("Mock");
  const bib = await editor.innerText();
  // BibTeX exporters may protect individual title words with brace groups.
  expect(bib.replace(/[{}]/gu, "")).toContain("Synthetic Research Preview");
  expect(bib.replace(/[{}]/gu, "")).toContain("Mock Methods and Reproducibility");
  expect(bib.match(/@article/gu)).toHaveLength(2);
  expect(bib).toContain(`@article{${firstKey},`);
  expect(bib).toContain(`@article{${secondKey},`);

  // Exercise the actual structured metadata editor, including live tag refresh.
  await page.getByRole("tablist", { name: "打开的标签页", exact: true}).getByRole("tab", {name: "main.tex", exact: true}).click();
  // Wait for the source session to replace the retained read-only bibliography
  // view before opening its toolbar menu.
  await expect(editor).toHaveAttribute("contenteditable", "true");
  await expect(editor).toContainText("\\documentclass");
  await expect(firstReference).toHaveCount(2);
  await page.locator(".writing-toolbar .writing-menu").locator('[aria-haspopup="menu"]').click();
  await page.getByRole("menuitem", {name: "参考文献管理", exact: true}).click();
  const manager = page.getByRole("dialog", {name: "参考文献管理", exact: true});
  await manager.getByRole("article").filter({hasText: "Synthetic Research Preview"}).getByRole("button", {name: "对照编辑"}).click();
  const longTitle = "Supercalifragilisticexpialidocious remaining full title";
  await manager.getByRole("textbox", {name: "编辑标题", exact: true}).fill(longTitle);
  await manager.getByRole("button", {name: "更新文献库", exact: true}).click();
  await expect(manager.getByRole("article").filter({hasText: longTitle})).toBeVisible();
  await manager.getByRole("button", {name: "关闭参考文献管理", exact: true}).click();
  const longWord = firstReference.first().locator(".writing-citation-title");
  await expect(longWord).toHaveText("Supercalifragilisticexpialidocious…");
  expect(await longWord.evaluate(element => element.scrollWidth > element.clientWidth)).toBe(true);
  await expect(firstReference.first()).toHaveAttribute("title", new RegExp(longTitle));
});

test("citation completion separates library-only matches and keeps its compact width", async ({page}) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "写作", exact: true}).click();
  await page.getByRole("button", {name: "新建稿件", exact: true}).click();
  await page.getByRole("dialog", {name: "新建 LaTeX 稿件"}).getByRole("textbox").fill("引用候选分组验收");
  await page.getByRole("button", {name: "创建稿件", exact: true}).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toContainText("\\documentclass");
  await editor.click(); await editor.press("ControlOrMeta+End");
  await page.getByRole("button", {name: "插入引用", exact: true}).click();
  const picker = page.getByRole("dialog", {name: "插入文献引用", exact: true});
  await picker.getByRole("textbox", {name: "搜索引用"}).fill("Synthetic Research Preview");
  await picker.getByRole("checkbox").check();
  await picker.getByRole("button", {name: "插入引用", exact: true}).click();
  await expect(picker).toHaveCount(0);
  await editor.press("ControlOrMeta+End"); await editor.press("Enter");
  await editor.pressSequentially("\\cite{Preview");
  const popup = page.locator(".writing-citation-completions");
  await expect(popup.getByRole("option")).toHaveCount(2);
  await expect(popup.getByRole("option").first()).toContainText("Synthetic Research Preview");
  await expect(popup.getByRole("option").last()).toContainText("Mock Methods");
  await expect(popup.locator("completion-section")).toHaveText("源自文献库");
  const geometry = await popup.evaluate(element => {
    const children = Array.from(element.querySelector("ul")!.children);
    const title = element.querySelector<HTMLElement>(".cm-completionLabel")!;
    return {width: element.getBoundingClientRect().width, listWidth: element.querySelector("ul")!.getBoundingClientRect().width,
      rowWidths: children.map(row => row.getBoundingClientRect().width), order: children.map(row => row.tagName),
      truncated: title.scrollWidth > title.clientWidth};
  });
  expect(geometry.width).toBeCloseTo(200, 0);
  expect(geometry.listWidth).toBeLessThanOrEqual(200);
  expect(Math.max(...geometry.rowWidths)).toBeLessThanOrEqual(200);
  expect(geometry.order).toEqual(["LI", "COMPLETION-SECTION", "LI"]);
  expect(geometry.truncated).toBe(true);
  await page.screenshot({path: ".tmp/citation-groups-preview.png"});
  await editor.press("ArrowDown"); await editor.press("Enter");
  await expect(editor.locator('.writing-citation-reference').filter({hasText:"Mock…"})).toHaveCount(1);
  await editor.press("ControlOrMeta+End"); await editor.press("Enter");
  await editor.pressSequentially("\\cite{Preview");
  await expect(popup.getByRole("option")).toHaveCount(2);
  await expect(popup.locator("completion-section")).toHaveCount(0);
});

test("command completion inserts braces, retains the argument cursor and reuses existing braces", async ({page}) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "写作", exact: true}).click();
  await page.getByRole("button", {name: "新建稿件", exact: true}).click();
  await page.getByRole("dialog", {name: "新建 LaTeX 稿件"}).getByRole("textbox").fill("命令参数补全验收");
  await page.getByRole("button", {name: "创建稿件", exact: true}).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toContainText("\\documentclass");
  await editor.click();
  await editor.press("ControlOrMeta+a");
  await editor.pressSequentially("\\cit");
  await page.getByRole("option", {name: "cite", exact: true}).click();
  await expect(editor).toHaveText("\\cite{}");
  await expect(page.locator(".cm-tooltip-autocomplete")).toHaveCount(0);
  await page.keyboard.insertText("Synthetic");
  await expect(editor).toHaveText("\\cite{Synthetic}");
  await editor.press("ControlOrMeta+a");
  await page.keyboard.insertText("\\sec{Existing}");
  await editor.press("Home");
  for (let index = 0; index < 4; index++) await editor.press("ArrowRight");
  await editor.press("Control+Space");
  await page.getByRole("option", {name: /^section/}).click();
  await expect(editor).toHaveText("\\section{Existing}");
  await page.keyboard.insertText("New ");
  await expect(editor).toHaveText("\\section{New Existing}");
});

test("citation suggestions require an input clue, including explicit completion", async ({page}) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "写作", exact: true}).click();
  await page.getByRole("button", {name: "新建稿件", exact: true}).click();
  await page.getByRole("dialog", {name: "新建 LaTeX 稿件"}).getByRole("textbox").fill("空引用候选验收");
  await page.getByRole("button", {name: "创建稿件", exact: true}).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toContainText("\\documentclass");
  await editor.click();
  await editor.press("ControlOrMeta+a");
  await editor.pressSequentially("\\cite{");
  await editor.press("Control+Space");
  await expect(page.locator(".cm-tooltip-autocomplete")).toHaveCount(0);
  await editor.pressSequentially("Synthetic");
  await expect(page.getByRole("option", {name: /Synthetic Research Preview/})).toBeVisible();
  for (const _letter of "Synthetic") await editor.press("Backspace");
  await expect(page.locator(".cm-tooltip-autocomplete")).toHaveCount(0);
});
