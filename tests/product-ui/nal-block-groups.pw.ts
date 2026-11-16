import { expect,test,type Page } from "@playwright/test";
import { copula,editSlot,addBlock,groupConnector,fillStatements } from "./nal-query-editor";
async function clause(page:Page,number:string,target:string){
  await copula(page,number);await editSlot(page,`条件 ${number} 左槽`,"变量","文章");await editSlot(page,`条件 ${number} 右槽`,target);
}

for(const route of ["/","/?preview=app"]){
  test(`block add menu is a bounded vertical popup without moving query content: ${route}`,async({page})=>{
    await page.goto(route);await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
    const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
    await panel.getByRole("group",{name:"条件 1",exact:true}).hover();
    const trigger=page.getByRole("button",{name:"添加 block：条件 1",exact:true});
    const outputs=panel.getByRole("region",{name:"输出变量",exact:true});
    const before=await outputs.boundingBox();
    await trigger.click();
    const menu=page.getByRole("menu",{name:"添加 block：条件 1",exact:true});
    await expect(menu).toBeVisible();
    const items=menu.getByRole("menuitem");await expect(items).toHaveCount(5);
    const boxes=await items.evaluateAll(elements=>elements.map(el=>{const r=el.getBoundingClientRect();return {x:r.x,y:r.y,bottom:r.bottom};}));
    for(let i=1;i<boxes.length;i++){
      expect(boxes[i]!.x).toBeCloseTo(boxes[0]!.x,0);
      expect(boxes[i]!.y).toBeGreaterThanOrEqual(boxes[i-1]!.bottom);
    }
    await expect(menu).not.toHaveCSS("background-color","rgba(0, 0, 0, 0)");
    expect((await outputs.boundingBox())!.y).toBeCloseTo(before!.y,0);
    expect(await menu.evaluate(el=>Boolean(el.closest(".nal-query-panel-content")))).toBe(false);
    if(route.includes("preview"))await page.screenshot({path:".tmp/verification/nal-block-menu-fixed.png"});
    await menu.press("Escape");await expect(menu).toBeHidden();await expect(trigger).toBeFocused();
    await trigger.press("ArrowDown");
    await page.getByRole("menuitem",{name:"添加 statement",exact:true}).press("Enter");
    await expect(panel.getByRole("group",{name:"条件 2",exact:true})).toBeVisible();
    await page.setViewportSize({width:540,height:420});
    await panel.getByRole("group",{name:"条件 2",exact:true}).hover();
    await page.getByRole("button",{name:"添加 block：条件 2",exact:true}).click();
    const narrowMenu=page.getByRole("menu",{name:"添加 block：条件 2",exact:true});
    await expect(narrowMenu).toBeVisible();
    await expect(async()=>{
      const rect=await narrowMenu.boundingBox();expect(rect).not.toBeNull();
      expect(rect!.x).toBeGreaterThanOrEqual(0);expect(rect!.x+rect!.width).toBeLessThanOrEqual(540);
      expect(rect!.y).toBeGreaterThanOrEqual(0);expect(rect!.y+rect!.height).toBeLessThanOrEqual(420);
    }).toPass();
    await narrowMenu.getByRole("menuitem",{name:"添加 disjunction 分组",exact:true}).click();
    await expect(panel.getByRole("button",{name:"分组 3 connector",exact:true})).toHaveText("∨ disjunction");
    await expect(narrowMenu).toBeHidden();
    await panel.getByRole("group",{name:"条件 1",exact:true}).hover();
  await page.getByRole("button",{name:"添加 block：条件 1",exact:true}).click();
    await panel.getByRole("heading",{name:"查询文献",exact:true}).click();
    await expect(page.getByRole("menu")).toHaveCount(0);await expect(panel).toBeVisible();
  });
}

test("compound slot commands live in connector and term menus and share production history",async({page})=>{
  await page.goto("/?preview=app");await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  await copula(page,1);await editSlot(page,"条件 1 左槽","product（×）");
  await editSlot(page,"条件 1 左槽 · 第 1 项","变量","root");
  await editSlot(page,"条件 1 左槽 · 第 2 项","AI领域");
  const expression=panel.getByLabel("条件 1 Narsese 语句",{exact:true});
  await expect(panel.getByRole("button",{name:"条件 1 左槽",exact:true})).toHaveCount(0);
  // Bracket commands only add boundary slots, never reopen whole-term type selection.
  await page.getByRole("button",{name:"条件 1 左槽 左括号",exact:true}).click();
  const bracket=page.getByRole("menu",{name:"条件 1 左槽 左括号",exact:true});
  await expect(bracket.getByRole("menuitem")).toHaveText(["增加槽位"]);
  await bracket.press("Escape");
  await page.getByRole("button",{name:"条件 1 左槽 connector 1",exact:true}).click();
  const connector=page.getByRole("menu",{name:"条件 1 左槽 connector 1",exact:true});
  await expect(connector.getByRole("menuitem",{name:"删除槽位",exact:true})).toHaveCount(0);
  await connector.getByRole("menuitem",{name:"增加槽位",exact:true}).click();
  await expect(connector).toHaveCount(0);
  await expect(expression).toHaveText("(?root × [选择词项] × AI领域) → [选择词项]");
  await page.getByRole("button",{name:"条件 1 左槽 · 第 2 项",exact:true}).click();
  const slot=page.getByRole("menu",{name:"条件 1 左槽 · 第 2 项",exact:true});
  await expect(slot.getByRole("menuitem",{name:"增加槽位",exact:true})).toHaveCount(0);
  await slot.getByRole("menuitem",{name:"删除槽位",exact:true}).click();
  await expect(slot).toHaveCount(0);
  await expect(expression).toHaveText("(?root × AI领域) → [选择词项]");
  await page.getByRole("button",{name:"条件 1 左槽 · 第 1 项",exact:true}).click();
  await page.getByRole("menuitem",{name:"删除槽位",exact:true}).click();
  await expect(expression).toHaveText("AI领域 → [选择词项]");
  await expect(panel.getByRole("button",{name:"条件 1 左槽",exact:true})).toBeFocused();
  await page.keyboard.press("ControlOrMeta+z");
  await expect(expression).toHaveText("(?root × AI领域) → [选择词项]");
  await page.keyboard.press("ControlOrMeta+z");
  await expect(expression).toHaveText("(?root × [选择词项] × AI领域) → [选择词项]");
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(expression).toHaveText("AI领域 → [选择词项]");
});

test("nested OR/AND block groups execute, reorder, reopen and change their actual query semantics",async({page})=>{
  await page.goto("/?preview=app");await page.getByRole("button",{name:"管理概念与关系",exact:true}).click();
  const manager=page.getByRole("dialog",{name:"管理概念与关系",exact:true});
  await manager.getByRole("tab",{name:"关系语句",exact:true}).click();
  await fillStatements(page,"Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic → 深度学习领域\nSynthetic Research Preview: Evidence and Autonomy → AI领域\nSynthetic Research Preview: Evidence and Autonomy → 方法");
  await manager.getByRole("button",{name:"添加语句",exact:true}).click();await expect(manager.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("[选择连接符]");
  await manager.getByRole("button",{name:"关闭面板",exact:true}).click();
  await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  await page.getByRole("button",{name:"添加 block：条件 1",exact:true}).click();
  await page.getByRole("menuitem",{name:"放入 disjunction 分组",exact:true}).click();
  await clause(page,"1.1","深度学习领域");
  await editSlot(page,"条件 1.2 连接符","conjunction（∧）");
  await clause(page,"1.2.1","AI领域");await clause(page,"1.2.2","方法");
  await expect(panel.getByRole("button",{name:"移除输出变量 ?文章",exact:true})).toHaveCount(1);
  await expect(panel.getByRole("button",{name:"新建 statement",exact:true})).toHaveCount(0);
  await panel.getByRole("group",{name:"分组 1.2",exact:true}).hover();
  await page.getByRole("button",{name:"拖动分组 1.2",exact:true}).dragTo(panel.getByRole("group",{name:"条件 1.1",exact:true}));
  await expect(panel.getByRole("button",{name:"分组 1.1 connector",exact:true})).toHaveText("∧ conjunction");
  await panel.getByRole("switch",{name:"保存为查询集合",exact:true}).check();
  await panel.getByLabel("查询集合名称",{exact:true}).fill("嵌套逻辑");await panel.getByRole("button",{name:"查询并保存",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(2);
  await page.reload();await page.getByRole("button",{name:"查询操作：嵌套逻辑",exact:true}).click();await page.getByRole("menuitem",{name:"编辑查询：嵌套逻辑",exact:true}).click();
  const saved=page.getByRole("dialog",{name:"编辑查询集合",exact:true});
  await expect(saved.getByRole("button",{name:"分组 1 connector",exact:true})).toHaveText("∨ disjunction");
  await expect(saved.getByRole("button",{name:"分组 1.1 connector",exact:true})).toHaveText("∧ conjunction");
  await groupConnector(page,"分组 1 connector","conjunction");
  await saved.getByRole("button",{name:"保存查询集合",exact:true}).click();await page.getByRole("button",{name:"嵌套逻辑",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(0);
});

test("variable tags stay visible with plus/remove actions and save their projection",async({page})=>{
  await page.goto("/?preview=app");await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  await copula(page,1);await editSlot(page,"条件 1 左槽","变量","x");await editSlot(page,"条件 1 右槽","变量","y");
  const outputs=panel.getByRole("region",{name:"输出变量",exact:true});
  await expect(outputs.getByRole("checkbox")).toHaveCount(0);
  await expect(outputs.getByRole("button")).toHaveCount(2);
  await outputs.getByRole("button",{name:"移除输出变量 ?x",exact:true}).click();
  await expect(outputs.getByRole("button",{name:"添加输出变量 ?x",exact:true})).toHaveAttribute("aria-pressed","false");
  await expect(outputs.getByRole("button")).toHaveCount(2);
  await outputs.getByRole("button",{name:"添加输出变量 ?x",exact:true}).press("Enter");
  await expect(outputs.getByRole("button",{name:"移除输出变量 ?x",exact:true})).toHaveAttribute("aria-pressed","true");
  await outputs.getByRole("button",{name:"移除输出变量 ?y",exact:true}).click();
  await panel.getByRole("switch",{name:"保存为查询集合",exact:true}).check();
  await panel.getByLabel("查询集合名称",{exact:true}).fill("标签输出");await panel.getByRole("button",{name:"查询并保存",exact:true}).click();
  await expect(panel).toHaveCount(0);
  await page.reload();await page.getByRole("button",{name:"查询操作：标签输出",exact:true}).click();await page.getByRole("menuitem",{name:"编辑查询：标签输出",exact:true}).click();
  const saved=page.getByRole("dialog",{name:"编辑查询集合",exact:true});
  await expect(saved.getByRole("button",{name:"移除输出变量 ?x",exact:true})).toBeVisible();
  await expect(saved.getByRole("button",{name:"添加输出变量 ?y",exact:true})).toBeVisible();
});
