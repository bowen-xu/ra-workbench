import { expect, test } from "@playwright/test";

test("managed BibTeX preserves syntax highlighting and exposes no raw editing controls", async ({page}) => {
  await page.goto("/?preview=app&bibliography=1");
  await page.getByRole("button",{name:"写作",exact:true}).click();
  await page.getByRole("button",{name:"Autonomy（合成稿件） main.tex",exact:true}).click();
  // The app fixture restores an admitted Agent delivery into main.tex on open.
  // Wait for that production lifecycle before navigating to another file.
  await expect(page.getByRole("group", {name:"第 2 处变更操作", exact:true})).toBeVisible();
  await expect(page.locator(".writing-toolbar .save-state")).toHaveText("已保存");
  await page.getByRole("button",{name:"project.bib",exact:true}).click();
  const editor=page.locator(".writing-editor-host .cm-content");
  await expect(editor).toHaveAttribute("contenteditable","false");
  await expect(page.locator(".writing-editor-status")).toContainText("BibTeX · 受管只读");
  await expect(page.getByRole("button",{name:"粗体",exact:true})).toBeDisabled();
  await expect(page.getByRole("button",{name:"文件操作 project.bib",exact:true})).toHaveCount(0);
  const colors=await editor.locator("span").evaluateAll(spans=>spans.map(span=>({text:span.textContent,color:getComputedStyle(span).color})));
  const color=(text:string)=>colors.find(item=>item.text?.trim()===text)?.color;
  expect(color("@article")).toBeTruthy();expect(color("previewimported2026")).toBeTruthy();expect(color("title")).toBeTruthy();
  expect(new Set([color("@article"),color("previewimported2026"),color("title"),color("2026")]).size).toBeGreaterThanOrEqual(3);
});


test("archived manuscript permanent deletion", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "新建稿件", exact: true }).click();
  await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("删除与 BibTeX 验收");
  await page.getByRole("button", { name: "创建稿件", exact: true }).click();
  await expect(page.locator(".writing-editor-host .cm-content")).toBeVisible();
  await page.getByRole("tab", { name: "返回写作项目列表", exact: true }).click();
  await expect(page.getByRole("button", { name: "永久删除 删除与 BibTeX 验收", exact: true })).toHaveCount(0);
  page.once("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "归档 删除与 BibTeX 验收", exact: true }).click();
  await page.getByRole("button", { name: /已归档/ }).click();
  const trigger = page.getByRole("button", { name: "永久删除 删除与 BibTeX 验收", exact: true });
  // WebKit mouse clicks do not necessarily focus buttons. Restore the actual
  // opener even when it never became document.activeElement.
  await trigger.evaluate((button) => button.addEventListener("mousedown", (event) => event.preventDefault()));
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: '永久删除“删除与 BibTeX 验收”？' });
  await expect(dialog.getByRole("button", { name: "取消", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await dialog.getByRole("button", { name: "永久删除", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(trigger).toHaveCount(0);
  await expect(page.locator('[data-sonner-toast]')).toContainText("已永久删除");
  await page.getByRole("button", { name: /我的稿件/ }).click();
  await expect(page.locator(".writing-project-link")).toHaveCount(0);
  await expect(page.locator(".writing-editor-host .cm-content")).toHaveCount(0);
});
