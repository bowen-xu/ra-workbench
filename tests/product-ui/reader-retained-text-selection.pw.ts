import {expect,test} from "@playwright/test";

test("editing another document after retained PDF suspension does not run a stale text-layer listener",async({page})=>{
  const errors:string[]=[];page.on("pageerror",error=>errors.push(error.message));
  await page.goto("/?preview=app");
  await page.getByRole("row",{name:/Synthetic Research Preview/}).dblclick();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.",{exact:true})).toBeVisible();
  await page.clock.install();
  await page.getByRole("button",{name:"笔记",exact:true}).click();
  await page.locator(".notes-explorer").getByRole("link",{name:/打开笔记：研究计划/}).first().click();
  const editor=page.locator(".retained-note-editor:not([hidden]) .bn-editor");
  await expect(editor).toBeVisible();
  await page.clock.fastForward(61_000);
  await expect(page.locator(".reader-tab-surface .textLayer span")).toHaveCount(0);
  await editor.click();await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.insertText(" selection after native-style PDF suspension");
  await expect(editor).toContainText("selection after native-style PDF suspension");
  await expect(page.locator(".notes-content:not([hidden]) .note-save-state")).toHaveAttribute("aria-label","已保存");
  await page.getByRole("button",{name:"文献",exact:true}).click();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.",{exact:true})).toBeVisible();
  expect(errors).toEqual([]);
  await page.screenshot({path:".tmp/reader-retained-text-selection.png"});
});
