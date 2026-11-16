import { expect,test } from "@playwright/test";

test("tabs retain file icons, italic previews, context home and cross-workspace ordering",async({page})=>{
  await page.setViewportSize({width:1434,height:898});await page.goto("/?preview=app");
  const tabs=page.getByRole("tablist",{name:"打开的标签页",exact:true});
  await page.getByRole("row",{name:/Vision by Logic/}).dblclick();
  const pdf=tabs.getByRole("tab",{name:/Vision by Logic/});
  await expect(pdf.locator(".workspace-tab-pdf")).toHaveText("PDF");
  await expect(tabs.locator('[data-home="true"]')).toHaveCount(1);
  const homePath=await tabs.getByRole("tab",{name:"返回文献列表",exact:true}).locator("svg").innerHTML();
  await page.getByRole("button",{name:"笔记",exact:true}).click();
  await page.locator(".notes-explorer").getByRole("button",{name:/研究计划（合成笔记）/}).click();
  const note=tabs.getByRole("tab",{name:"研究计划（合成笔记）",exact:true});
  await expect(note.locator(".workspace-tab-title")).toHaveCSS("font-style","italic");
  await expect(note.locator(".workspace-tab-title")).toHaveCSS("font-synthesis-style","auto");
  expect(await tabs.getByRole("tab",{name:"返回笔记首页",exact:true}).locator("svg").innerHTML()).toBe(homePath);
  await note.dblclick();await expect(note.locator(".workspace-tab-title")).toHaveCSS("font-style","normal");
  await page.getByRole("button",{name:"写作",exact:true}).click();
  await page.getByRole("button",{name:"Autonomy（合成稿件） main.tex",exact:true}).click();
  const code=tabs.getByRole("tab",{name:/main.tex/});
  await expect(code.locator(".lucide-file-code-corner")).toBeVisible();
  await expect(code.locator(".workspace-tab-pending")).toBeVisible();
  await expect(code).not.toContainText("◇");
  expect(await tabs.getByRole("tab",{name:"返回写作项目列表",exact:true}).locator("svg").innerHTML()).toBe(homePath);
  await expect(code.locator("..")).toHaveCSS("height","26px");
  const geometry=await tabs.locator('.workspace-tab').evaluateAll(rows=>rows.map(row=>{
    const button=row.querySelector('[role="tab"]')!,style=getComputedStyle(button);
    return {top:row.getBoundingClientRect().top,height:row.getBoundingClientRect().height,
      buttonHeight:button.getBoundingClientRect().height,fontSize:style.fontSize,lineHeight:style.lineHeight,
      paddingLeft:style.paddingLeft,paddingRight:style.paddingRight};
  }));
  expect(new Set(geometry.map(row=>JSON.stringify(row))).size).toBe(1);
  const start=await code.boundingBox(),target=await pdf.boundingBox();if(!start || !target)throw new Error("missing tab geometry");
  await page.mouse.move(start.x+start.width/2,start.y+start.height/2);await page.mouse.down();
  await page.mouse.move(target.x+2,target.y+target.height/2,{steps:10});await page.mouse.up();
  await expect(code).toHaveAttribute("aria-selected","true");
  const labels=await tabs.getByRole("tab").allTextContents();
  const order=await tabs.locator('.workspace-tab:not([data-home="true"])').evaluateAll(rows=>rows.map(row=>row.getAttribute("data-tab-id")));
  expect(labels.findIndex(label=>label.includes("main.tex"))).toBeLessThan(labels.findIndex(label=>label.includes("Vision by Logic")));
  await expect(tabs.locator(".workspace-tab").first()).toHaveAttribute("data-home","true");
  await page.getByRole("button",{name:"笔记",exact:true}).click();
  await expect(tabs.getByRole("tab",{name:"返回笔记首页",exact:true})).toBeVisible();
  expect(await tabs.locator('.workspace-tab:not([data-home="true"])').evaluateAll(rows=>rows.map(row=>row.getAttribute("data-tab-id")))).toEqual(order);
  await code.focus();await code.press("Alt+ArrowRight");
  expect(await tabs.locator('.workspace-tab:not([data-home="true"])').evaluateAll(rows=>rows.map(row=>row.getAttribute("data-tab-id")))).toEqual([order[1],order[0],...order.slice(2)]);
  await expect(page.getByRole("textbox",{name:"笔记标题",exact:true})).toBeVisible();
  await page.screenshot({path:".tmp/verification/workspace-tab-icons.png"});
});

test("floating note expands into the same pinned page and notes fill the pane with bottom marquee support",async({page})=>{
  await page.setViewportSize({width:1434,height:898});await page.goto("/?preview=app");
  await page.getByRole("row",{name:/Synthetic Research Preview/}).dblclick();
  await page.getByRole("tab",{name:"高亮与笔记",exact:true}).click();
  await page.getByRole("button",{name:"写文献笔记",exact:true}).click();
  const floating=page.getByRole("dialog",{name:"文献笔记编辑面板",exact:true});
  const editor=floating.locator(".bn-editor");await editor.click();await page.keyboard.type("Expanded canonical note");
  await floating.getByRole("button",{name:"在笔记标签页中打开",exact:true}).click();
  await expect(floating).toHaveCount(0);
  const main=page.locator(".notes-workspace-host .notes-content");
  await expect(main.locator(".bn-editor")).toContainText("Expanded canonical note");
  const tabs=page.getByRole("tablist",{name:"打开的标签页",exact:true});
  const note=tabs.getByRole("tab",{name:"文献笔记",exact:true});
  await expect(note.locator(".workspace-tab-title")).toHaveCSS("font-style","normal");
  const bounds=await main.boundingBox();if(!bounds)throw new Error("missing notes pane");
  expect(bounds.y+bounds.height).toBeCloseTo(898,0);
  const content=main.locator(".bn-block-content").first(),row=await content.boundingBox();if(!row)throw new Error("missing note content");
  await page.mouse.move(row.x+80,bounds.y+bounds.height-15);await page.mouse.down();
  await page.mouse.move(row.x+120,row.y+2,{steps:12});
  await expect(page.locator(".note-block-selection-rect")).toBeVisible();await page.mouse.up();
  await expect(content.locator('xpath=ancestor::*[contains(@class,"bn-block-outer")][1]')).toHaveCSS("background-color","rgb(234, 242, 237)");
  await page.keyboard.press("Escape");
  await tabs.getByRole("tab",{name:"返回笔记首页",exact:true}).click();
  await expect(page.getByRole("heading",{name:"建立你的研究笔记",exact:true})).toBeVisible();
  await expect(note).toBeVisible();await expect(tabs.getByRole("tab",{name:"返回文献列表",exact:true})).toHaveCount(0);
  const empty=await main.boundingBox();expect((empty?.y ?? 0)+(empty?.height ?? 0)).toBeCloseTo(898,0);
  await note.click();await expect(main.locator(".bn-editor")).toContainText("Expanded canonical note");
  await page.screenshot({path:".tmp/verification/note-expanded-tab.png"});
});
