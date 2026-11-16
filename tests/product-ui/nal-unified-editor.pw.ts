import { expect, test, type Locator, type Page } from "@playwright/test";
import type { NalState } from "../../../ra-workbench-sdk/packages/core/src/nal/index";
import { addBlock, editSlot, fillStatements } from "./nal-query-editor";

async function inside(toolbar:Locator,container:Locator){
  await expect(async()=>{
    const a=await toolbar.boundingBox(),b=await container.boundingBox();
    expect(a).not.toBeNull();expect(b).not.toBeNull();
    expect(a!.x).toBeGreaterThanOrEqual(b!.x-1);
    expect(a!.x+a!.width).toBeLessThanOrEqual(b!.x+b!.width+1);
    expect(a!.y).toBeGreaterThanOrEqual(b!.y-1);
    expect(a!.y+a!.height).toBeLessThanOrEqual(b!.y+b!.height+1);
  }).toPass();
}
async function readState(page:Page):Promise<NalState>{
  return page.evaluate(async()=>{const path="/plugins/ra-literature-library/src/services/nalGateway.ts";return (await import(path)).getNalGateway().read();});
}

for(const route of ["/?preview=app","/nal-editor.html"]){
  test(`shared floating block gutter stays within the editor at wide and narrow sizes: ${route}`,async({page})=>{
    await page.goto(route);
    if(route.includes("preview=app"))await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
    else await page.getByRole("button",{name:"从空白开始",exact:true}).click();
    const container=route.includes("preview=app") ? page.getByRole("dialog",{name:"文献查询设置",exact:true}) : page.locator(".nal-query-editor");
    for(const width of [1280,1146,420]){
      await page.setViewportSize({width,height:730});
      const row=page.getByRole("group",{name:"条件 1",exact:true});await row.hover();
      const toolbar=page.getByRole("toolbar",{name:"块控件：条件 1",exact:true});
      await inside(toolbar,container);
      const before=await row.boundingBox();
      await page.getByRole("button",{name:"添加 block：条件 1",exact:true}).click();
      const menu=page.getByRole("menu",{name:"添加 block：条件 1",exact:true});
      await expect(menu).toBeVisible();await inside(toolbar,container);
      expect(await row.boundingBox()).toEqual(before);
      expect(await menu.getByRole("menuitem").first().evaluate(el=>{const r=el.getBoundingClientRect();return el.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));})).toBe(true);
      if(width===1146)await page.screenshot({path:`.tmp/verification/nal-gutter-${route.includes("preview=app") ? "app" : "isolated"}.png`});
      await menu.press("Escape");
    }
    await editSlot(page,"条件 1 连接符","disjunction（∨）");
    for(const label of ["分组 1","条件 1.1","条件 1.2"]){
      await page.getByRole("group",{name:label,exact:true}).hover();
      await inside(page.getByRole("toolbar",{name:`块控件：${label}`,exact:true}),container);
    }
  });
}

test("management source statements use the shared editor, persist facts and retain draft history across tabs",async({page})=>{
  await page.goto("/?preview=app");await page.getByRole("button",{name:"管理概念与关系",exact:true}).click();
  await page.getByRole("tab",{name:"关系语句",exact:true}).click();
  const manager=page.getByRole("dialog",{name:"管理概念与关系",exact:true});
  const editor=manager.getByRole("form",{name:"Narsese 关系语句编辑器",exact:true});
  await expect(editor.locator("textarea,select")).toHaveCount(0);
  await expect(editor.getByRole("region",{name:"输出变量",exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:"条件 1 连接符",exact:true}).click();
  const menu=page.getByRole("menu",{name:"条件 1 连接符",exact:true});
  await expect(menu.getByRole("menuitemradio")).toHaveText(["-->（继承）"]);
  await expect(menu.getByRole("menuitemradio").first()).toHaveCSS("border-width","0px");
  await page.screenshot({path:".tmp/verification/nal-native-shared-menu.png"});
  await menu.press("Escape");
  await fillStatements(page,"AI领域 → 方法");
  await page.getByRole("button",{name:"条件 1 左槽",exact:true}).click();
  await expect(page.getByRole("menuitemradio",{name:"statement（语句）",exact:true})).toHaveCount(0);
  await page.getByRole("menuitem",{name:"条件 1 左槽 atomic",exact:true}).click();
  const atomic=page.getByRole("menu",{name:"条件 1 左槽 atomic",exact:true});
  await expect(atomic.locator('[role^="menuitem"]')).toHaveText(["概念","文献"]);await atomic.press("Escape");
  await page.getByRole("tab",{name:"概念",exact:true}).click();
  await page.getByRole("tab",{name:"关系语句",exact:true}).click();
  await expect(editor.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("AI领域 → 方法");
  await editor.getByRole("button",{name:"撤销语句编辑",exact:true}).click();
  await expect(editor.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("AI领域 → [选择词项]");
  await editor.getByRole("button",{name:"重做语句编辑",exact:true}).click();
  await editor.getByRole("group",{name:"条件 1",exact:true}).hover();
  await inside(page.getByRole("toolbar",{name:"块控件：条件 1",exact:true}),manager);
  await page.screenshot({path:".tmp/verification/nal-management-shared-editor.png"});
  await editor.getByRole("button",{name:"添加语句",exact:true}).click();
  await expect(editor.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("[选择连接符]");
  await expect(editor.getByRole("button",{name:"撤销语句编辑",exact:true})).toBeDisabled();
  const state=await readState(page),ai=state.concepts.find(c=>c.label==="AI领域")!,active=state.concepts.find(c=>c.label==="方法")!;
  expect(state.statements.some(s=>s.subject.kind==="concept"&&s.subject.id===ai.id&&s.predicate.kind==="concept"&&s.predicate.id===active.id)).toBe(true);
});

test("selected-paper relation dialog binds a product template for each selected paper through the same editor",async({page})=>{
  await page.goto("/?preview=app");
  const rows=page.locator(".literature-row");await rows.nth(0).click();await rows.nth(1).click({modifiers:["ControlOrMeta"]});
  const ids=await page.locator(".literature-row.selected").evaluateAll(nodes=>nodes.map(n=>n.getAttribute("data-item-id")));
  expect(ids).toHaveLength(2);
  await page.getByRole("button",{name:"管理文献关系",exact:true}).click();
  const dialog=page.getByRole("dialog",{name:"文献关系 · 2 篇",exact:true});
  await expect(dialog.locator("textarea,select")).toHaveCount(0);
  await expect(dialog.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("?选中文献 → [选择词项]");
  await editSlot(page,"条件 1 左槽","product（×）");
  await editSlot(page,"条件 1 左槽 · 第 1 项","选中文献");
  await editSlot(page,"条件 1 左槽 · 第 2 项","方法");
  await editSlot(page,"条件 1 右槽","基于方法");
  await page.getByRole("button",{name:"条件 1 左槽 connector 1",exact:true}).click();
  const connector=page.getByRole("menu",{name:"条件 1 左槽 connector 1",exact:true});
  await expect(connector.getByRole("menuitemradio")).toHaveText(["product（×）"]);await connector.press("Escape");
  await dialog.getByRole("group",{name:"条件 1",exact:true}).hover();
  await inside(page.getByRole("toolbar",{name:"块控件：条件 1",exact:true}),dialog);
  await page.screenshot({path:".tmp/verification/nal-selected-papers-shared-editor.png"});
  await dialog.getByRole("button",{name:"为选中文献添加关系",exact:true}).click();await expect(dialog).toHaveCount(0);
  const state=await readState(page),relation=state.concepts.find(c=>c.label==="基于方法")!,target=state.concepts.find(c=>c.label==="方法")!;
  for(const id of ids)expect(state.statements.some(s=>s.subject.kind==="product"&&s.subject.terms[0]?.kind==="paper"&&s.subject.terms[0].id===id&&s.subject.terms[1]?.kind==="concept"&&s.subject.terms[1].id===target.id&&s.predicate.kind==="concept"&&s.predicate.id===relation.id)).toBe(true);
});

test("persistent block controls let Escape dismiss only the current menu then its query panel",async({page})=>{
  await page.goto("/?preview=app");await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  await addBlock(page);
  await page.getByRole("group",{name:"条件 2",exact:true}).hover();
  await page.getByRole("button",{name:"添加 block：条件 2",exact:true}).click();
  const menu=page.getByRole("menu",{name:"添加 block：条件 2",exact:true});await menu.press("Escape");
  await expect(menu).toHaveCount(0);await expect(panel).toBeVisible();
  await panel.press("Escape");await expect(panel).toHaveCount(0);
  await expect(page.getByRole("button",{name:"设置文献查询",exact:true})).toBeFocused();
});
