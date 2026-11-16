import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";

for (const entry of ["/?preview=app", "/?demo=app"]) test(`compact citation menus copy and export without a preview: ${entry}`, async ({page, context}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.setViewportSize({width:1434,height:898});
  await page.goto(entry);
  const sidebar=page.getByRole("complementary",{name:"主导航",exact:true});
  for(const name of ["文献","笔记","写作"]){
    const icon=sidebar.getByRole("button",{name,exact:true}).locator("svg");
    await expect(icon).toHaveAttribute("width","20");await expect(icon).toHaveAttribute("height","20");
    await expect(icon).toHaveAttribute("stroke-width","1.8");
  }
  await page.getByRole("row", {name:/Vision by Logic/}).click();
  const detail=page.locator(".literature-detail"),actions=detail.getByRole("group",{name:"引用与导出",exact:true});
  await expect(actions).toBeVisible();
  await expect(detail.locator(".citation-card, .export-row")).toHaveCount(0);
  await expect(detail.getByRole("button",{name:"归档此文献",exact:true})).toHaveCount(0);
  await expect(detail.getByRole("button",{name:"删除（移入回收站）",exact:true})).toHaveCount(0);
  for(const width of [1434,1276]){
    await page.setViewportSize({width,height:898});
    const row=await actions.boundingBox();
    const controls=await actions.locator("select, button").evaluateAll(elements=>elements.map(element=>element.getBoundingClientRect().top));
    expect(row!.height).toBeLessThan(45);expect(new Set(controls).size).toBe(1);
  }
  const heading=detail.locator(".literature-citation-heading"),readingHeading=detail.locator(".source-pdf-label");
  const typography=(element:Element)=>{const style=getComputedStyle(element);return {size:style.fontSize,weight:style.fontWeight,color:style.color};};
  expect(await heading.evaluate(typography)).toEqual(await readingHeading.evaluate(typography));
  const styleSelect=actions.getByRole("combobox",{name:"引用样式",exact:true});
  await styleSelect.click();
  const choices=page.getByRole("listbox",{name:"引用样式",exact:true});
  await expect(choices).toHaveCSS("border-radius","20px");
  await expect(choices.getByRole("option",{name:"Nature",exact:true})).toHaveAttribute("aria-selected","true");
  await choices.getByRole("option",{name:"APA",exact:true}).click();
  await expect(styleSelect).toHaveText("APA");
  const copy=actions.getByRole("button",{name:"复制引用",exact:true});
  await copy.focus();await page.keyboard.press("Enter");
  await page.getByRole("menuitem",{name:"复制文内引用",exact:true}).click();
  await expect(page.getByText("已复制文内引用",{exact:true})).toBeVisible();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toMatch(/Xu.*2026/);
  await copy.click();await page.getByRole("menuitem",{name:"复制参考文献",exact:true}).click();
  await expect(page.getByText("已复制参考文献条目",{exact:true})).toBeVisible();
  expect(await page.evaluate(()=>navigator.clipboard.readText())).toContain("Vision by Logic");
  await expect(copy).toBeFocused();
  for(const [format,extension] of [["BibTeX","bib"],["RIS","ris"],["CSL JSON","json"]]){
    await actions.getByRole("button",{name:"导出文献",exact:true}).click();
    const result=page.waitForEvent("download");
    await page.getByRole("menuitem",{name:format!,exact:true}).click();
    const download=await result;expect(download.suggestedFilename()).toMatch(new RegExp(`\\.${extension}$`));
    const file=await download.path();expect(file).toBeTruthy();
    const text=await readFile(file!,"utf8");expect(text.replace(/[{}]/g,"")).toContain("Vision by Logic");
    if(extension==="json")expect(JSON.parse(text)).toBeInstanceOf(Array);
  }
  await page.setViewportSize({width:1276,height:600});
  await detail.evaluate(element=>{element.scrollTop=element.scrollHeight;});
  const lastRow=await actions.boundingBox(),launcher=await page.getByRole("button",{name:"打开对话",exact:true}).boundingBox();
  expect(lastRow!.y+lastRow!.height).toBeLessThanOrEqual(launcher!.y-12);
  expect(await detail.evaluate(element=>getComputedStyle(element).paddingBottom)).toBe("88px");
  await page.setViewportSize({width:1434,height:898});
  await detail.evaluate(element=>{element.scrollTop=element.scrollHeight;});
  await expect(page.getByRole("button",{name:"关闭通知",exact:true})).toHaveCount(0,{timeout:12000});
  await page.screenshot({path:`.tmp/verification/citation-actions-${entry.includes("demo")?"demo":"preview"}.png`});
  await actions.screenshot({path:`.tmp/verification/citation-actions-bar-${entry.includes("demo")?"demo":"preview"}.png`});
  await page.evaluate(()=>{navigator.clipboard.writeText=async()=>{throw new Error("clipboard denied");};});
  await copy.click();await page.getByRole("menuitem",{name:"复制文内引用",exact:true}).click();
  await expect(page.getByText("复制失败：clipboard denied",{exact:true})).toBeVisible();
});
