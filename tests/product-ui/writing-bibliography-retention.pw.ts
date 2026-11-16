import {expect, test, type Page} from "@playwright/test";

async function create(page: Page, imported = false) {
  await page.goto(imported ? "/?preview=app&bibliography=1" : "/?preview=app");
  await page.getByRole("button",{name:"写作",exact:true}).click();
  await page.getByRole("button",{name:"新建稿件",exact:true}).click();
  await page.getByRole("dialog",{name:"新建 LaTeX 稿件"}).getByRole("textbox").fill("引用来源清理验收");
  await page.getByRole("button",{name:"创建稿件",exact:true}).click();
  const editor=page.locator(".writing-editor-host .cm-content");
  await expect(editor).toContainText("\\documentclass");
  await expect(page.locator(".writing-editor-host .cm-editor")).toHaveClass(/cm-focused/);
  return editor;
}
async function manager(page: Page) {
  await page.locator(".writing-toolbar .writing-menu").locator('[aria-haspopup="menu"]').click();
  await page.getByRole("menuitem",{name:"参考文献管理",exact:true}).click();
  return page.getByRole("dialog",{name:"参考文献管理",exact:true});
}

test("automatic entries disappear after the last citation and return with the same key on undo",async({page})=>{
  const editor=await create(page);
  await editor.click();await editor.press("ControlOrMeta+Home");
  await page.getByRole("button",{name:"插入引用",exact:true}).click();
  const picker=page.getByRole("dialog",{name:"插入文献引用",exact:true});
  await picker.getByRole("textbox",{name:"搜索引用"}).fill("Mock Methods");
  await picker.getByRole("checkbox").check();await picker.getByRole("button",{name:"插入引用",exact:true}).click();
  const tag=editor.locator(".writing-citation-reference");await expect(tag).toHaveCount(1);
  const key=await tag.getAttribute("data-citation-key");
  await tag.click();await page.keyboard.press("Backspace");
  await expect(tag).toHaveCount(0);await editor.press("ControlOrMeta+s");
  await expect(page.locator(".save-state")).toHaveText("已保存");
  const empty=await manager(page);await expect(empty.getByRole("article")).toHaveCount(0);
  await empty.getByRole("button",{name:"关闭参考文献管理",exact:true}).click();
  await editor.press("ControlOrMeta+z");await expect(tag).toHaveAttribute("data-citation-key",key!);
  await editor.press("ControlOrMeta+s");await expect(page.locator(".save-state")).toHaveText("已保存");
  const restored=await manager(page);await expect(restored.getByRole("article")).toHaveCount(1);
  await expect(restored.getByRole("article")).toContainText(key!);
  await restored.getByRole("button",{name:"关闭参考文献管理",exact:true}).click();
  await page.locator(".writing-file-row").getByRole("button",{name:"references.bib",exact:true}).click();
  await expect(editor).toContainText(`@article{${key},`);
  expect((await editor.innerText()).match(new RegExp(`@article\\{${key},`,"gu"))).toHaveLength(1);
  await expect(page.locator("[data-sonner-toast]").filter({hasText:"未使用引用"})).toHaveCount(0);
});

test("unused imports survive automatically and require explicit cleanup, protecting a fresh unsaved citation",async({page})=>{
  const editor=await create(page,true);
  const dialog=await manager(page);await expect(dialog.getByRole("article")).toHaveCount(3);
  await dialog.getByRole("button",{name:"关闭参考文献管理",exact:true}).click();
  await editor.click();await editor.press("ControlOrMeta+Home");
  await page.keyboard.insertText("\\cite{localmethod2026}\n");
  const cleanup=await manager(page);
  await cleanup.getByRole("button",{name:"清理未使用导入条目",exact:true}).click();
  await expect(cleanup.getByRole("article")).toHaveCount(1);
  await expect(cleanup.getByRole("article")).toContainText("localmethod2026");
  await expect(page.locator("[data-sonner-toast]").filter({hasText:"未使用引用"})).toHaveCount(0);
});
