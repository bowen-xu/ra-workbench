import { expect, test, type Page } from "@playwright/test";

async function openEditor(page: Page, route = "/?preview=app") {
  await page.goto(route);
  await page.getByRole("button", {name: "写作", exact: true}).click();
  await page.getByRole("button", {name: "新建稿件", exact: true}).click();
  await page.getByRole("dialog", {name: "新建 LaTeX 稿件"}).getByRole("textbox").fill("环境补全验收");
  await page.getByRole("button", {name: "创建稿件", exact: true}).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toContainText("\\documentclass");
  await editor.click(); await editor.press("ControlOrMeta+a");
  return editor;
}

for (const route of ["/", "/?preview=app"]) {
  test(`list environment snippets retain normal editing and undo on ${route}`, async ({page}) => {
    const editor = await openEditor(page, route);
    await editor.pressSequentially("\\begin");
    await page.getByRole("option", {name: /^begin\s*begin environment$/u}).click();
    await expect(editor).toHaveText("\\begin{}");
    await page.keyboard.type("itemi");
    await page.getByRole("option", {name: "\\begin{itemize} …", exact: true}).click();
    await expect(editor).toHaveText("\\begin{itemize}  \\item \\end{itemize}");
    await editor.press("ControlOrMeta+z");
    await expect(editor).toHaveText("\\begin{itemi}");
    await editor.press("ControlOrMeta+Shift+z");
    await expect(editor).toHaveText("\\begin{itemize}  \\item \\end{itemize}");
    // CodeMirror redo restores source with its ordinary insertion-end cursor.
    await editor.press("ControlOrMeta+Home");
    await editor.press("ArrowDown");
    await editor.press("End");
    await page.keyboard.insertText("First");
    await editor.press("Tab");
    await page.keyboard.insertText("item");
    await expect(editor).toHaveText("\\begin{itemize}  \\item First  item\\end{itemize}");
  });
}

test("figure fields support Tab/Shift+Tab and Escape restores ordinary editing", async ({page}) => {
  const editor = await openEditor(page);
  await editor.pressSequentially("\\fig");
  await page.getByRole("option", {name: "\\begin{figure} …", exact: true}).click();
  await page.keyboard.insertText("plot.pdf");
  await editor.press("Tab");
  await editor.press("Shift+Tab");
  await page.keyboard.insertText("other.pdf");
  await editor.press("Tab");
  await page.keyboard.insertText("A caption");
  await editor.press("Tab");
  await page.keyboard.insertText("fig:test");
  await editor.press("Escape");
  await editor.press("Tab");
  await page.keyboard.insertText("x");
  await expect(editor).toContainText("\\includegraphics[width=0.5\\linewidth]{other.pdf}");
  await expect(editor).toContainText("\\caption{A caption}");
  await expect(editor).toContainText("\\label{fig:test  x}");
  await expect(editor).toContainText("\\end{figure}");
  await page.screenshot({path: ".tmp/writing-environment-figure.png"});
});

test("end completion chooses the innermost repeated environment and custom names are discoverable", async ({page}) => {
  const editor = await openEditor(page);
  await page.keyboard.insertText("\\begin{document}\n\\begin{enumerate}\n\\begin{enumerate}\n\\end{enumerate}\n\\end{");
  await editor.press("Control+Space");
  await expect(page.getByRole("option").first()).toHaveText("enumerate");
  await editor.press("Enter");
  await expect(editor).toContainText("\\end{enumerate}");
  await expect(page.locator(".cm-line").last()).toHaveText("\\end{enumerate}");
  await editor.press("ControlOrMeta+a");
  await page.keyboard.insertText("\\newenvironment{custom-box}{}{}\n\\begin{");
  await page.keyboard.type("custom-b");
  await page.getByRole("option", {name: "\\begin{custom-box} …", exact: true}).click();
  await page.keyboard.insertText("Body");
  await expect(editor).toContainText("\\begin{custom-box}");
  await expect(editor).toContainText("Body");
  await expect(editor).toContainText("\\end{custom-box}");
});

test("a previously unknown environment remains available while its name is typed", async ({page}) => {
  const editor = await openEditor(page);
  await editor.pressSequentially("\\begin{brandnew}");
  await page.getByRole("option", {name: "\\begin{brandnew} …", exact: true}).click();
  await page.keyboard.insertText("Body");
  await expect(editor).toHaveText("\\begin{brandnew}  Body\\end{brandnew}");
});
