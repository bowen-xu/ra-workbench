import { expect, test, type Page } from "@playwright/test";
import { chooseInline, copula, editSlot, addBlock, fillStatements } from "./nal-query-editor";

const papers = ["文章1", "文章2", "文章3"].map((title, i) => ({
  id: `p${i + 1}`, itemType: "journalArticle", title, abstractText: "", containerTitle: "", volume: "", issue: "", pages: "", doi: "", url: "", language: "", bibtexKey: "", creators: [], tags: [], readingStatus: "unread", attachments: [], projectIds: [], metadataSources: [], createdAt: "2026-10-15", updatedAt: "2026-10-15",
}));

async function open(page: Page) {
  await page.addInitScript(seed => {
    if (localStorage.getItem("nal-test-seeded")) return;
    localStorage.clear();
    localStorage.setItem("research-assistant.literature.v1", JSON.stringify(seed));
    localStorage.setItem("nal-test-seeded", "1");
  }, papers);
  await page.goto("/");
  await expect(page.locator(".literature-row")).toHaveCount(3);
  await expect(page.getByRole("button", {name: "管理概念与关系", exact: true})).toBeEnabled();
}

async function closeConcepts(page:Page){
  const panel=page.getByRole("dialog",{name:"管理概念与关系",exact:true});
  if(await panel.count())await panel.getByRole("button",{name:"关闭面板",exact:true}).click();
}
async function conceptDirectory(page:Page){
  const panel=page.getByRole("dialog",{name:"管理概念与关系",exact:true});
  if(!await panel.count())await page.getByRole("button",{name:"管理概念与关系",exact:true}).click();
  await panel.getByRole("tab",{name:"概念",exact:true}).click();
  return panel.getByRole("navigation",{name:"概念列表",exact:true});
}
async function newConcept(page:Page){
  await conceptDirectory(page);
  await page.getByRole("button",{name:"新增概念",exact:true}).click();
}
async function concept(page: Page, name: string, kind?: "ordinary" | "relation") {
  await newConcept(page);
  const dialog = page.getByRole("region", {name: "新增概念", exact: true});
  await dialog.getByLabel("概念类型", {exact:true}).selectOption(kind ?? (["所属领域","研究发展","基于方法"].includes(name) ? "relation" : "ordinary"));
  await dialog.getByLabel("名称", {exact: true}).fill(name);
  await dialog.getByLabel("解释", {exact: true}).fill(`测试词项 ${name}；关系的第一个参数为基础工作，第二个参数为后续工作。`);
  await dialog.getByRole("button", {name: "创建概念", exact: true}).click();
  await expect(dialog).toHaveCount(0);
}

async function openStatements(page:Page){
  await closeConcepts(page);
  await page.getByRole("button",{name:"管理概念与关系",exact:true}).click();
  await page.getByRole("tab",{name:"关系语句",exact:true}).click();
}
async function statements(page: Page, text: string) {
  await closeConcepts(page);
  await openStatements(page);
  const dialog = page.getByRole("dialog", {name: "管理概念与关系", exact: true});
  await fillStatements(page,text);
  await dialog.getByRole("button", {name: "添加语句", exact: true}).click();
  await expect(dialog.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("[选择连接符]");
  await closeConcepts(page);
}

async function query(page: Page, expression: string) {
  await closeConcepts(page);
  await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  const [subject,predicate]=expression.replaceAll("-->","→").replaceAll("*","×").split("→").map(v=>v.trim());
  let target=predicate!,relation="继承",reverse=false;
  if(subject!.startsWith("(")){
    const terms=subject!.slice(1,-1).split("×").map(v=>v.trim());
    reverse=!/^[?$]/u.test(terms[0]!);target=terms[reverse ? 0 : 1]!;relation=predicate!;
  }
  await chooseInline(page,1,"关系",relation);
  await chooseInline(page,1,"对象",target);
  if(reverse)await panel.getByRole("group",{name:"条件 1 左槽 第 2 项（可拖动排序）",exact:true}).press("Alt+ArrowLeft");
  await panel.getByRole("button",{name:"查询文献",exact:true}).click();
  await expect(panel).toHaveCount(0);
}
async function queryAction(page:Page,name:string,action:string){
  await page.getByRole("button",{name:"查询操作："+name,exact:true}).click();
  await page.getByRole("menuitem",{name:action+"："+name,exact:true}).click();
}
async function saveQuery(page:Page,name:string){
  await closeConcepts(page);
  await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  await panel.getByRole("switch",{name:"保存为查询集合",exact:true}).check();
  await panel.getByLabel("查询集合名称").fill(name);
  await panel.getByRole("button",{name:"查询并保存",exact:true}).click();
  await expect(panel).toHaveCount(0);
}

test("rejects duplicate names and edits the existing name alongside the new contextual draft",async({page})=>{
  await open(page);
  await concept(page,"感知");
  await statements(page,"文章1 → 感知");
  await newConcept(page);
  const dialog=page.getByRole("region",{name:"新增概念",exact:true});
  await dialog.getByLabel("名称",{exact:true}).fill("感知");
  await dialog.getByLabel("解释",{exact:true}).fill("心理学语境中的感知。");
  await expect(dialog.getByRole("button",{name:"创建概念",exact:true})).toBeDisabled();
  await dialog.getByRole("button",{name:"编辑已有概念",exact:true}).click();
  await expect(dialog.getByLabel("已有概念名称",{exact:true})).toHaveValue("感知");
  await dialog.getByLabel("已有概念名称",{exact:true}).fill("感知`AI");
  await dialog.getByLabel("名称",{exact:true}).fill("感知`心理学");
  await dialog.getByRole("button",{name:"创建概念",exact:true}).click();
  await expect(dialog.getByRole("button",{name:"新概念已保存",exact:true})).toBeDisabled();
  await expect(dialog.getByLabel("已有概念名称",{exact:true})).toHaveValue("感知`AI");
  await expect(dialog.getByLabel("解释",{exact:true})).toHaveValue("心理学语境中的感知。");
  await dialog.getByRole("button",{name:"保存已有概念",exact:true}).click();
  await expect(dialog).toHaveCount(0);
  const directory=(await conceptDirectory(page));
  await expect(directory.getByRole("button",{name:"感知`AI",exact:true})).toBeVisible();
  await expect(directory.getByRole("button",{name:"感知`心理学",exact:true})).toBeVisible();
  await expect(directory.getByRole("button",{name:"感知`AI",exact:true}).locator(".concept-name-context")).toHaveText("`AI");
  await expect(directory.getByRole("button",{name:"感知`心理学",exact:true}).locator(".concept-name-context")).toHaveText("`心理学");
  await query(page,"?文章 → 感知`AI");
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await page.reload();
  await conceptDirectory(page);
  await directory.getByRole("button",{name:"感知`AI",exact:true}).click();
  await expect(page.locator(".nal-statements:visible")).toContainText(/文章1.*→ 感知`AI/u);
  await newConcept(page);
  await dialog.getByRole("combobox",{name:"概念类型",exact:true}).selectOption("relation");
  await dialog.getByLabel("名称",{exact:true}).fill("感知`AI");
  await dialog.getByLabel("解释",{exact:true}).fill("跨概念类型也不能同名。");
  await expect(dialog.getByRole("button",{name:"创建概念",exact:true})).toBeDisabled();
});

test("creates typed concepts from the visible entry and immediately uses a new relation",async({page})=>{
  await open(page);
  await conceptDirectory(page);
  await page.getByLabel("查找概念",{exact:true}).fill("不会匹配新概念的筛选");
  const draft="(?文章 × 神经科学) → 促进研究";
  await newConcept(page);
  const dialog=page.getByRole("region",{name:"新增概念",exact:true});
  await expect(dialog.getByLabel("名称",{exact:true})).toBeFocused();
  await expect(dialog.getByLabel("概念类型",{exact:true})).toHaveValue("ordinary");
  await dialog.getByLabel("名称",{exact:true}).fill("神经科学");
  await dialog.getByLabel("解释",{exact:true}).fill("研究神经系统的领域概念。");
  await dialog.getByRole("button",{name:"创建概念",exact:true}).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.getByLabel("查找概念",{exact:true})).toHaveValue("");
  await expect(page.getByRole("region",{name:"普通概念",exact:true}).getByRole("button",{name:"神经科学",exact:true})).toBeVisible();
  await expect(page.locator(".nal-concept-detail").getByLabel("概念类型",{exact:true})).toHaveValue("ordinary");
  await expect(page.getByRole("button",{name:"添加传递规则",exact:true})).toHaveCount(0);
  await expect(page.locator(".nal-sidebar textarea:visible")).toHaveCount(0);
  await concept(page,"促进研究","relation");
  await expect(page.getByRole("region",{name:"关系概念",exact:true}).getByRole("button",{name:"促进研究",exact:true})).toBeVisible();
  await expect(page.getByRole("button",{name:"添加传递规则",exact:true})).toBeVisible();
  await expect(page.locator(".nal-relation-rule code")).toHaveCount(0);
  await expect(page.locator(".nal-sidebar textarea:visible")).toHaveCount(0);
  await page.reload();
  await conceptDirectory(page);
  await expect(page.getByRole("region",{name:"普通概念",exact:true}).getByRole("button",{name:"神经科学",exact:true})).toBeVisible();
  await page.getByRole("region",{name:"关系概念",exact:true}).getByRole("button",{name:"促进研究",exact:true}).click();
  await closeConcepts(page);
  await page.locator(".literature-row").filter({hasText:"文章1"}).click();
  await page.locator(".library-toolbar").getByRole("button",{name:"管理文献关系",exact:true}).click();
  const relations=page.getByRole("dialog",{name:"文献关系 · 1 篇",exact:true});
  await expect(relations.getByRole("combobox")).toHaveCount(0);
  await fillStatements(page,"(文章1 × 神经科学) → 促进研究");
  await relations.getByRole("button",{name:"为选中文献添加关系",exact:true}).click();
  await expect(relations).toHaveCount(0);
  await query(page,draft);
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".literature-row")).toContainText("文章1");
  await (await conceptDirectory(page)).getByRole("button",{name:"促进研究",exact:true}).click();
  await page.locator(".nal-concept-detail").getByLabel("概念类型",{exact:true}).selectOption("ordinary");
  await expect(page.getByText("该概念已用作关系；请先移除相关关系语句和规则",{exact:true})).toBeVisible();
  await expect(page.locator(".nal-concept-detail").getByLabel("概念类型",{exact:true})).toHaveValue("relation");
});

test("inheritance queries include descendant papers, preserve ordered roles, and survive reload", async ({page}) => {
  test.setTimeout(60_000); // This path now constructs five compound facts through visible menus.
  await open(page);
  for (const name of ["领域", "AI领域", "深度学习领域", "所属领域", "研究发展"]) await concept(page, name);
  await statements(page, "(AI领域 × 领域) → 所属领域\n(深度学习领域 × AI领域) → 所属领域\n(文章1 × AI领域) → 所属领域\n(文章3 × 深度学习领域) → 所属领域\n(文章1 × 文章2) → 研究发展");
  await query(page, "(?文章 × AI领域) → 所属领域");
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await (await conceptDirectory(page)).getByRole("button", {name:"所属领域",exact:true}).click();
  await expect(page.locator(".nal-relation-rule code")).toHaveCount(0);
  await page.getByRole("button",{name:"添加传递规则",exact:true}).click();
  await expect(page.getByRole("button",{name:"删除传递规则",exact:true})).toBeVisible();
  await expect(page.locator(".nal-relation-rule input[type=checkbox]")).toHaveCount(0);
  await query(page, "(?文章 × AI领域) → 所属领域");
  await expect(page.locator(".literature-row")).toHaveCount(2);
  await (await conceptDirectory(page)).getByRole("button",{name:"所属领域",exact:true}).click();
  await page.getByRole("button",{name:"删除传递规则",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".nal-relation-rule code")).toHaveCount(0);
  await page.getByRole("button",{name:"添加传递规则",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(2);
  await expect(page.locator(".literature-row").filter({hasText: "文章3"})).toBeVisible();
  await expect(page.locator(".nal-proof")).toHaveCount(0);
  await expect(page.locator(".nal-concept-detail .nal-statements")).toContainText("(深度学习领域 × AI领域) → 所属领域");
  await saveQuery(page,"AI领域文献");
  await page.reload();
  await page.getByRole("button", {name: "AI领域文献", exact: true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(2);
  await query(page, "(文章1 × ?后续) → 研究发展");
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".literature-row")).toContainText("文章2");
  await page.getByRole("textbox", {name: "搜索文献", exact: true}).fill("文章2");
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".literature-row")).toContainText("文章2");
  await page.getByRole("textbox", {name: "搜索文献", exact: true}).fill("");
  await query(page, "(文章2 × ?后续) → 研究发展");
  await expect(page.locator(".literature-row")).toHaveCount(0);
  await page.getByRole("button", {name: "全部文献", exact: true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(3);
  await expect(page.getByRole("button", {name: "新建集合", exact: true})).toHaveCount(0);
});

test("annotations reuse the reading note floating host and keep one document", async ({page}) => {
  await open(page); await concept(page, "领域"); await concept(page, "AI领域");
  await statements(page, "AI领域 → 领域");
  const openNote=async()=>{
    await openStatements(page);
    await page.getByRole("dialog",{name:"管理概念与关系",exact:true}).getByRole("button", {name: "语句笔记：AI领域 → 领域", exact: true}).click();
  };
  await openNote();
  const annotation = page.getByRole("dialog", {name: "语句笔记编辑面板", exact: true});
  await expect(annotation).toHaveClass(/reading-note-floating-panel/);
  await expect(page.locator("dialog.nal-dialog")).toHaveCount(0);
  await expect(annotation.getByRole("button",{name:"新建笔记",exact:true})).toHaveCount(0);
  await annotation.getByRole("group", {name: "笔记正文"}).getByRole("textbox").fill("这条继承关系的 annotation。");
  await expect(annotation.getByRole("status", {name: "已保存", exact: true})).toBeVisible();
  await annotation.getByRole("button", {name: "关闭阅读笔记", exact: true}).click();
  await openNote();
  await expect(annotation.getByRole("group", {name: "笔记正文"})).toContainText("这条继承关系的 annotation。");
  const counts=await page.evaluate(async()=>{
    const networkPath="/plugins/ra-literature-library/src/services/nalGateway.ts",notesPath="/ra-workbench/apps/web/src/core/features/notes/gateways/index.ts";
    const state=await (await import(networkPath)).getNalGateway().read();
    const notes=await (await import(notesPath)).getNotesGateway().listPages({kind:"all"});
    return {links:state.annotations.length,pages:notes.length};
  });
  expect(counts).toEqual({links:1,pages:1});
  await annotation.getByRole("button", {name: "关闭阅读笔记", exact: true}).click();
  await openStatements(page);
  page.once("dialog", d => d.accept());
  await page.getByRole("dialog",{name:"管理概念与关系",exact:true}).getByRole("button", {name: "删除语句：AI领域 → 领域", exact: true}).click();
  await expect(page.getByRole("button", {name: "语句笔记：AI领域 → 领域", exact: true})).toHaveCount(0);
  await page.getByRole("dialog", {name: "管理概念与关系", exact: true}).getByRole("button", {name: "关闭面板"}).click();
  await expect(page.locator(".literature-row")).toHaveCount(3);
  await page.getByRole("navigation").getByRole("button", {name: "笔记", exact: true}).click();
  await expect(page.getByText("AI领域 → 领域", {exact: true}).first()).toBeVisible();
});

test("query is a view; Delete trashes and restore keeps the relation", async ({page}) => {
  await open(page); await concept(page, "AI领域"); await concept(page, "所属领域");
  await statements(page, "(文章3 × AI领域) → 所属领域");
  await query(page, "(?文章 × AI领域) → 所属领域");
  const row = page.locator(".literature-row");
  await row.click(); page.once("dialog", d => d.accept()); await row.press("Delete");
  await expect(row).toHaveCount(0);
  await page.getByRole("button", {name: "回收站", exact: true}).click();
  await expect(row).toHaveCount(1); await row.click();
  await page.getByRole("button", {name: "从回收站恢复", exact: true}).click();
  await page.getByRole("button", {name: "返回文献库", exact: true}).click();
  await expect(row).toHaveCount(1); await expect(row).toContainText("文章3");
  await page.getByRole("button", {name: "切换关系栏", exact: true}).click();
  await page.reload();
  await expect(page.getByRole("button", {name: "切换关系栏", exact: true})).toHaveAttribute("aria-expanded", "false");
});

test("cached query directories conjoin ancestor conditions and update after relation changes",async({page})=>{
  await open(page);
  for(const name of ["AI领域","感知领域","所属领域"])await concept(page,name);
  await statements(page,"(文章1 × AI领域) → 所属领域\n(文章2 × AI领域) → 所属领域\n(文章1 × 感知领域) → 所属领域\n(文章3 × 感知领域) → 所属领域");
  await query(page,"($x * AI领域) --> 所属领域");
  await saveQuery(page,"AI文献");
  const directory=page.getByRole("navigation",{name:"文献视图",exact:true});
  await expect(directory.getByRole("button",{name:"AI文献",exact:true})).toContainText("2");
  await queryAction(page,"AI文献","添加子查询");
  const editor=page.getByRole("dialog",{name:"新增子查询",exact:true});
  await editor.getByLabel("查询集合名称").fill("感知文献");
  await chooseInline(page,1,"关系","所属领域");
  await chooseInline(page,1,"对象","感知领域");
  await editor.getByRole("button",{name:"保存查询集合",exact:true}).click();
  await expect(editor).toHaveCount(0);
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".literature-row")).toContainText("文章1");
  await expect(page.locator(".nal-query-summary")).toHaveCount(0);
  await statements(page,"(文章2 × 感知领域) → 所属领域");
  await expect(directory.getByRole("button",{name:"感知文献",exact:true})).toContainText("2");
  await expect(page.locator(".literature-row")).toHaveCount(2);
  await page.reload();
  await directory.getByRole("button",{name:"感知文献",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(2);
  await openStatements(page);
  const manager=page.getByRole("dialog",{name:"管理概念与关系",exact:true});
  page.once("dialog",d=>d.accept());
  await manager.getByRole("button",{name:"删除语句：(文章1 × AI领域) → 所属领域",exact:true}).click();
  await manager.getByRole("button",{name:"关闭面板",exact:true}).click();
  await expect(directory.getByRole("button",{name:"感知文献",exact:true})).toContainText("1");
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".literature-row")).toContainText("文章2");
  await queryAction(page,"AI文献","编辑查询");
  const edit=page.getByRole("dialog",{name:"编辑查询集合",exact:true});
  await chooseInline(page,1,"对象","感知领域");
  await edit.getByRole("button",{name:"保存查询集合",exact:true}).click();
  await expect(edit).toHaveCount(0);
  await expect(page.locator(".literature-row")).toHaveCount(3);
  page.once("dialog",d=>d.accept());
  await queryAction(page,"AI文献","删除查询集合");
  await expect(directory.getByRole("button",{name:"感知文献",exact:true})).toHaveCount(0);
  await expect(directory.getByRole("button",{name:"全部文献",exact:true})).toBeVisible();
  await expect(directory.locator(".nal-query-actions button")).toHaveCount(0);
  await expect(page.locator(".literature-row")).toHaveCount(3);
});

test("visual query popover supports AND conditions, cancel, focus return and viewport fitting",async({page})=>{
  await open(page);
  for(const name of ["AI领域","感知领域","所属领域"])await concept(page,name);
  await statements(page,"(文章1 × AI领域) → 所属领域\n(文章2 × AI领域) → 所属领域\n(文章1 × 感知领域) → 所属领域");
  const trigger=page.getByRole("button",{name:"设置文献查询",exact:true});
  await trigger.click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  await expect(page.locator(".nal-sidebar > form")).toHaveCount(0);
  await expect(panel.locator("textarea")).toHaveCount(0);
  const narsese=panel.getByLabel("条件 1 Narsese 语句",{exact:true});
  await expect(narsese).toHaveText("[选择连接符]");
  await chooseInline(page,1,"关系","所属领域");
  await chooseInline(page,1,"对象","AI领域");
  await expect(narsese).toHaveText("(?文章 × AI领域) → 所属领域");
  await panel.getByRole("group",{name:"条件 1 左槽 第 2 项（可拖动排序）",exact:true}).press("Alt+ArrowLeft");
  await expect(narsese).toHaveText("(AI领域 × ?文章) → 所属领域");
  await chooseInline(page,1,"关系","继承");
  await expect(narsese).toHaveText("?文章 → 所属领域");
  await chooseInline(page,1,"关系","所属领域");
  await chooseInline(page,1,"对象","AI领域");
  await addBlock(page);
  await chooseInline(page,2,"对象","感知领域");
  await expect(panel.locator(".nal-query-block")).toHaveCount(2);
  await panel.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await trigger.click();
  await expect(panel).toBeVisible();
  await page.getByRole("button",{name:"管理概念与关系",exact:true}).click();
  await expect(panel).toHaveCount(0);
  await expect(page.getByRole("dialog",{name:"管理概念与关系",exact:true})).toBeVisible();
  await closeConcepts(page);
  await expect(page.locator(".literature-row")).toHaveCount(3);
  await trigger.click();
  await expect(panel.locator(".nal-query-block")).toHaveCount(1);
  await chooseInline(page,1,"对象","AI领域");
  await addBlock(page);
  await chooseInline(page,2,"对象","感知领域");
  await page.setViewportSize({width:760,height:640});
  await expect(async () => {
    const bounds=await panel.boundingBox();
    expect(bounds).not.toBeNull();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x+bounds!.width).toBeLessThanOrEqual(760);
    expect(bounds!.y+bounds!.height).toBeLessThanOrEqual(640);
  }).toPass({timeout:5_000});
  await panel.getByRole("button",{name:"查询文献",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".literature-row")).toContainText("文章1");
  await trigger.click();
  await panel.getByRole("button",{name:"移除条件 2",exact:true}).click();
  await panel.getByRole("button",{name:"查询文献",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(2);
  await trigger.click();
  await panel.getByRole("button",{name:"清除查询",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(3);
});

test("app preview uses the same visual query panel and production query execution",async({page})=>{
  await page.goto("/?preview=app");
  await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const panel=page.getByRole("dialog",{name:"文献查询设置",exact:true});
  await expect(panel.locator("textarea")).toHaveCount(0);
  await chooseInline(page,1,"关系","所属领域");
  await chooseInline(page,1,"对象","AI领域");
  await panel.getByRole("button",{name:"查询文献",exact:true}).click();
  await expect(panel).toHaveCount(0);
  await expect(page.locator(".nal-query-summary")).toHaveCount(0);
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(page.locator(".library-relation-query")).toHaveCount(0);
  await expect(page.getByRole("button",{name:"清除关系查询",exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:"全部文献",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(5);
});

test("preview upgrade deletes obsolete data persistently and corrects the old field inheritance",async({page})=>{
  await page.goto("/?preview=app");
  await expect(page.getByRole("button",{name:"管理概念与关系",exact:true})).toBeEnabled();
  await page.evaluate(async()=>{
    const storageModule="/ra-workbench/apps/web/src/core/runtime/appStorage.ts",persistenceModule="/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts";
    const storage:Storage=(await import(storageModule)).getAppStorage();
    const conceptKey="research-assistant.concepts.v1",nalKey="research-assistant.nal.v1";
    const concepts=JSON.parse(storage.getItem(conceptKey)!);
    const ai=concepts.find((c:{label:string})=>c.label === "AI领域"),deep=concepts.find((c:{label:string})=>c.label === "深度学习领域");
    const old={...ai,id:"obsolete-related",label:"related-field",explanation:"旧集合关系。反向显示：related-field；有向：false。"};
    storage.setItem(conceptKey,JSON.stringify([...concepts,old,{...ai,id:"duplicate-ai"}]));
    const state=JSON.parse(storage.getItem(nalKey)!);
    storage.setItem(nalKey,JSON.stringify({...state,statements:[...state.statements,{id:"old-field-arrow",origin:"human",subject:{kind:"concept",id:deep.id},predicate:{kind:"concept",id:ai.id}}]}));
    await (await import(persistenceModule)).flushStorage(storage);
  });
  await page.reload();
  const list=(await conceptDirectory(page));
  await expect(list.getByRole("button",{name:"AI领域",exact:true})).toHaveCount(1);
  await expect(list.getByRole("button",{name:"related-field",exact:true})).toHaveCount(0);
  const stored=await page.evaluate(async()=>{
    const storageModule="/ra-workbench/apps/web/src/core/runtime/appStorage.ts";
    const storage:Storage=(await import(storageModule)).getAppStorage();
    const state=JSON.parse(storage.getItem("research-assistant.nal.v1")!);
    return {ids:state.concepts.map((c:{id:string})=>c.id),statement:state.statements.find((s:{id:string})=>s.id === "old-field-arrow"),deleted:JSON.parse(storage.getItem("research-assistant.concept-deletions.v1")!)};
  });
  expect(stored.ids).not.toContain("obsolete-related");
  expect(stored.ids).not.toContain("duplicate-ai");
  expect(stored.deleted).toEqual(expect.arrayContaining(["obsolete-related","duplicate-ai"]));
  expect(stored.statement.subject.kind).toBe("product");
  await list.getByRole("button",{name:"深度学习领域",exact:true}).click();
  await expect(page.locator(".nal-concept-detail code").filter({hasText:"深度学习领域"}).first()).toContainText("(深度学习领域 × AI领域) → 所属领域");
  await closeConcepts(page);
  await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  await copula(page,1);
  await page.getByRole("button",{name:"条件 1 右槽",exact:true}).click();
  await page.getByRole("menuitem",{name:"条件 1 右槽 atomic",exact:true}).click();
  await page.getByRole("menuitem",{name:"条件 1 右槽 concept",exact:true}).click();
  const options=page.getByRole("menu",{name:"条件 1 右槽 concept",exact:true});
  await expect(options.getByRole("menuitemradio",{name:"AI领域",exact:true})).toHaveCount(1);
  await expect(options.getByRole("menuitemradio",{name:"related-field",exact:true})).toHaveCount(0);
  await page.reload();
  await conceptDirectory(page);
  await expect(list.getByRole("button",{name:"related-field",exact:true})).toHaveCount(0);
});

test("preview example queries can be selected and deleted, then stay hidden on reopen",async({page})=>{
  await page.goto("/?preview=app");
  const directory=page.getByRole("navigation",{name:"文献视图",exact:true});
  await directory.getByRole("button",{name:"AI领域文献",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(1);
  const selectedRow=directory.locator(".nal-query-row").filter({has:page.getByRole("button",{name:"AI领域文献",exact:true})});
  await expect(selectedRow).toHaveAttribute("data-selected","true");
  await expect(selectedRow.locator(".nal-query-actions button")).toHaveCount(1);
  expect(await selectedRow.evaluate(el=>getComputedStyle(el).backgroundColor)).not.toBe("rgba(0, 0, 0, 0)");
  expect(await selectedRow.locator(".nal-query-select").evaluate(el=>getComputedStyle(el).backgroundColor)).toBe("rgba(0, 0, 0, 0)");
  await directory.getByRole("button",{name:"深度学习领域文献",exact:true}).click();
  await expect(selectedRow).toHaveAttribute("data-selected","false");
  await expect(page.locator(".literature-row")).toHaveCount(1);
  page.once("dialog",dialog=>dialog.accept());
  await queryAction(page,"深度学习领域文献","删除查询集合");
  await expect(directory.getByRole("button",{name:"深度学习领域文献",exact:true})).toHaveCount(0);
  await expect(directory.getByRole("button",{name:"AI领域文献",exact:true})).toBeVisible();
  page.once("dialog",dialog=>dialog.accept());
  await queryAction(page,"AI领域文献","删除查询集合");
  await expect(directory.getByRole("button",{name:"全部文献",exact:true})).toBeVisible();
  await expect(directory.locator(".nal-query-actions button")).toHaveCount(0);
  await expect(page.locator(".literature-row")).toHaveCount(5);
  await page.reload();
  await expect(page.getByRole("button",{name:"设置文献查询",exact:true})).toBeEnabled();
  await expect(directory.getByRole("button",{name:"全部文献",exact:true})).toBeVisible();
  await expect(directory.locator(".nal-query-actions button")).toHaveCount(0);
});

test("NAL paper, concept and variable terms have distinct backgrounds in rules and query previews",async({page})=>{
  await page.goto("/?preview=app");
  await (await conceptDirectory(page)).getByRole("button",{name:"所属领域",exact:true}).click();
  const rule=page.locator(".nal-relation-rule code");
  await expect(rule.locator(".nal-variable-term")).toHaveCount(6);
  await expect(rule).toHaveText("($y × $z) → 所属领域 ∧ ($x × $y) → 所属领域 ⇒ ($x × $z) → 所属领域");
  const background=(selector:string)=>page.locator(selector).first().evaluate(el=>getComputedStyle(el).backgroundColor);
  const concept=await background(".nal-concept-term"),paper=await background(".nal-paper-name"),variable=await background(".nal-variable-term");
  expect(new Set([concept,paper,variable]).size).toBe(3);
  expect([concept,paper,variable]).not.toContain("rgba(0, 0, 0, 0)");
  await closeConcepts(page);
  await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  const preview=page.getByLabel("条件 1 Narsese 语句",{exact:true});
  await copula(page,1);
  await editSlot(page,"条件 1 左槽","变量","文章");
  await expect(preview.locator(".nal-variable-term")).toHaveText("?文章");
  expect(await preview.locator(".nal-variable-term").evaluate(el=>getComputedStyle(el).backgroundColor)).toBe(variable);
});

test("concepts and paper statements live in icon-opened management panels",async({page})=>{
  await page.goto("/?preview=app");
  const sidebar=page.getByRole("complementary",{name:"文献概念与关系",exact:true});
  await expect(sidebar.getByRole("navigation",{name:"概念列表",exact:true})).toHaveCount(0);
  await expect(sidebar.getByLabel("查找概念",{exact:true})).toHaveCount(0);
  await expect(sidebar.locator(".nal-concept-detail")).toHaveCount(0);
  await page.locator(".literature-row").filter({hasText:"Synthetic Research Preview"}).click();
  const detail=page.locator(".literature-detail");
  await expect(detail.locator(".nal-statements")).toHaveCount(0);
  await expect(detail.getByText("关系语句",{exact:true})).toHaveCount(0);
  await expect(detail.getByRole("button",{name:"添加关系",exact:true})).toHaveCount(0);
  await expect(detail.getByRole("button",{name:"管理文献关系",exact:true})).toHaveCount(0);
  await expect(detail.locator(".nal-literature-relations")).toHaveCount(0);
  const trigger=page.locator(".library-toolbar").getByRole("button",{name:"管理文献关系",exact:true});
  await expect(trigger.locator("svg")).toHaveCount(1);expect((await trigger.innerText()).trim()).toBe("");
  await trigger.click();
  const relations=page.getByRole("dialog",{name:"文献关系 · 1 篇",exact:true});
  await expect(relations.locator(".nal-statements")).toContainText("深度学习领域");
  await relations.getByRole("button",{name:"关闭面板",exact:true}).click();
  const list=await conceptDirectory(page);
  await expect(list.getByRole("button",{name:"领域",exact:true})).toBeVisible();
  await list.getByRole("button",{name:"深度学习领域",exact:true}).click();
  await expect(page.locator(".nal-concept-detail")).toContainText("深度学习领域");
  await page.getByRole("button",{name:"概念笔记：深度学习领域",exact:true}).click();
  const note=page.getByRole("dialog",{name:"概念笔记编辑面板",exact:true});
  await expect(note).toBeVisible();
  await expect(page.getByRole("dialog",{name:"管理概念与关系",exact:true})).toHaveCount(0);
  await expect(note.locator(".reading-note-document-topline")).toHaveCount(1);
  await expect(note.locator(".note-page-title")).toHaveCount(0);
});

test("one management panel preserves drafts and the unified directory includes all papers",async({page})=>{
  await page.goto("/?preview=app");
  const sidebar=page.getByRole("complementary",{name:"文献概念与关系",exact:true});
  await expect(sidebar.locator(".nal-sidebar-heading button")).toHaveCount(2);
  await expect(sidebar.locator(".nal-sidebar-heading").getByRole("button",{name:"设置文献查询",exact:true})).toBeVisible();
  await expect(sidebar.getByText("关系查询",{exact:true})).toHaveCount(0);
  await expect(sidebar.getByRole("heading",{name:"查询集合",exact:true})).toHaveCount(0);
  const directory=sidebar.getByRole("navigation",{name:"文献视图",exact:true});
  const all=directory.getByRole("button",{name:"全部文献",exact:true});
  await expect(all.locator("small")).toHaveText("5");
  await all.click();await expect(page.locator(".literature-row")).toHaveCount(5);
  await expect(all).toHaveClass("nal-query-select");
  await directory.getByRole("button",{name:"AI领域文献",exact:true}).click();
  await expect(page.locator(".literature-row")).toHaveCount(1);
  await expect(all).not.toHaveAttribute("aria-current","page");
  await newConcept(page);
  const manager=page.getByRole("dialog",{name:"管理概念与关系",exact:true});
  await expect(page.locator("dialog[open]")).toHaveCount(1);
  const draft=manager.getByRole("region",{name:"新增概念",exact:true});
  await draft.getByLabel("名称",{exact:true}).fill("未保存的概念");
  await manager.getByRole("tab",{name:"关系语句",exact:true}).click();
  const expression=manager.getByLabel("条件 1 Narsese 语句",{exact:true});
  await fillStatements(page,"AI领域 → 领域");
  await manager.getByRole("tab",{name:"概念",exact:true}).click();
  await expect(draft.getByLabel("名称",{exact:true})).toHaveValue("未保存的概念");
  await draft.getByRole("button",{name:"取消新增",exact:true}).click();
  await manager.getByRole("tab",{name:"关系语句",exact:true}).click();
  await expect(expression).toHaveText("AI领域 → 领域");
  await manager.getByRole("tab",{name:"关系语句",exact:true}).press("ArrowLeft");
  await expect(manager.getByRole("tab",{name:"概念",exact:true})).toHaveAttribute("aria-selected","true");
});

test("compact query rows use one menu with keyboard actions and retain selection",async({page})=>{
  await page.goto("/?preview=app");
  const directory=page.getByRole("navigation",{name:"文献视图",exact:true});
  const parent=directory.getByRole("button",{name:"AI领域文献",exact:true});
  const child=directory.getByRole("button",{name:"深度学习领域文献",exact:true});
  const p=await parent.boundingBox(),c=await child.boundingBox();
  expect(c!.x-p!.x).toBeGreaterThanOrEqual(8);expect(c!.x-p!.x).toBeLessThanOrEqual(12);
  await child.click();
  const more=directory.getByRole("button",{name:"查询操作：AI领域文献",exact:true});
  await more.press("Enter");
  const menu=page.getByRole("menu",{name:"查询操作：AI领域文献",exact:true});
  await expect(menu.getByRole("menuitem")).toHaveCount(3);
  await expect(child).toHaveAttribute("aria-current","page");
  await menu.getByRole("menuitem",{name:"添加子查询：AI领域文献",exact:true}).press("Escape");
  await expect(menu).toBeHidden();await expect(more).toBeFocused();
  await queryAction(page,"AI领域文献","编辑查询");
  const editor=page.getByRole("dialog",{name:"编辑查询集合",exact:true});
  await expect(editor.getByLabel("查询集合名称",{exact:true})).toHaveValue("AI领域文献");
  await editor.getByRole("button",{name:"关闭面板",exact:true}).click();
  await expect(child).toHaveAttribute("aria-current","page");
});

