import {test,expect} from "@playwright/test";
import {openToolJournal, structuredFixture} from "./tool-journal";
test("edit states resolve resource labels and use the existing readable source hint",async({page})=>{
 await page.setViewportSize({width:1434,height:898});
 const resource=structuredFixture({revision:3,content:{},pendingFieldIds:[]});
 const steps=["failed","declined","legacy","validation","storage","completed"].map(status=>{
  const reason=status==="failed"?{code:"permission_denied",message:"synthetic sandbox denial"}:{code:"edit_declined",message:"synthetic explicit refusal"};
  return {name:"research_apply_patch",arguments:{patch:`*** Begin Patch\n*** Update File: ${resource.sourceUri}\n@@\n-old\n+new\n*** End Patch`},
   output:{status:["legacy","validation","storage"].includes(status)?"failed":status,targets:[{path:resource.sourceUri,status:status==="completed"?"applied":"failed",message:["validation","storage"].includes(status)?"Draft retained":JSON.stringify(reason),...(["validation","storage"].includes(status)?{structuredResource:{status:"invalid",resourceId:resource.id,draftId:"synthetic-draft",errors:[{code:status==="validation"?"invalid_evidence":"storage_error",path:"$.field",message:"synthetic reason"}]}}:{})}]}};
 });
 const fixture=await openToolJournal(page,{steps,resources:[resource]});
 await fixture.locator(".writing-agent-turn-process > summary").click();
 await fixture.locator("[data-agent-activity-group] > summary").click();
 const rows=fixture.locator(".tool-history-row");await expect(rows).toHaveCount(6);
 for(const [index,label] of ["无法编辑","拒绝编辑","拒绝编辑","拒绝编辑","无法编辑","已编辑"].entries()){
  await expect(rows.nth(index).locator(".tool-row-compact")).toContainText(`${label} 合成文献画像`);
  await expect(rows.nth(index).locator(".tool-row-compact")).not.toContainText("source://");
 }
 await rows.first().locator(".tool-row-compact .tool-patch-file-name").hover();
 const hint=page.getByRole("tooltip");await expect(hint).toContainText("合成文献画像");await expect(hint).toContainText("v3");await expect(hint).not.toContainText("source://");
 await page.screenshot({path:".tmp/tool-resource-title-hints.png"});
 await rows.first().locator(":scope > summary").click();await expect(rows.first().getByLabel("调用结果",{exact:true})).toContainText("permission_denied");
 const validation=rows.nth(3);await validation.locator(":scope > summary").click();
 await expect(validation).toHaveAttribute("data-edit-status","declined");await expect(validation.locator(".lucide-minus")).toHaveCount(1);await expect(validation.locator(".lucide-x")).toHaveCount(0);
 await expect(validation.getByLabel("调用结果",{exact:true})).toContainText("invalid_evidence");await expect(validation.getByLabel("调用结果",{exact:true})).toContainText("synthetic-draft");await expect(validation.locator(".tool-patch-diff")).toHaveCount(0);
 await rows.last().locator(":scope > summary").click();await expect(rows.last().locator("header .tool-patch-file-name")).toHaveText("合成文献画像");
});
test("file listings display readable category and object paths, omitting scope only for the full context",async({page})=>{
 await page.setViewportSize({width:1434,height:898});
 const paperId="private-paper-id",noteId="private-note-id",manuscriptId="private-project-id";
 const cases=[{},{resourceType:"note"},{resourceType:"paper"},{resourceType:"manuscriptFile"},{resourceId:`source://library/${paperId}`},{resourceId:`source://writings/${manuscriptId}`},{resourceId:`source://notes/${noteId}`},{resourceId:`source://library/${paperId}`,resourceType:"note"}];
 const fixture=await openToolJournal(page,{steps:cases.map(arguments_=>({name:"research_list_sources",arguments:arguments_,output:{sources:[],total:0}})),sourceTitles:{paperId,noteId,manuscriptId}});
 await fixture.locator(".writing-agent-turn-process > summary").click();
 await fixture.locator("[data-agent-activity-group] > summary").click();
 const rows=fixture.locator(".tool-row-compact");
 for(const [index,text] of ["已列出文件","已列出文件（范围 所有笔记）","已列出文件（范围 所有文献）","已列出文件（范围 所有写作项目）","已列出文件（范围 所有文献 / 论文甲）","已列出文件（范围 所有写作项目 / 稿件甲）","已列出文件（范围 所有笔记 / 实验笔记）","已列出文件（范围 所有文献 / 论文甲 / 笔记）"].entries())await expect(rows.nth(index)).toHaveText(text);
 await expect(fixture).not.toContainText("private-");await expect(fixture).not.toContainText("source://");
 await rows.nth(4).locator(".tool-read-file-name").hover();const hint=page.getByRole("tooltip");
 await expect(hint).toBeVisible();await expect(hint).toContainText("所有文献 / 论文甲");await expect(hint).not.toContainText("private-");await expect(hint).not.toContainText("source://");
 await page.screenshot({path:".tmp/tool-list-readable-scope.png"});
});
test("global discovery omits scope and resource discovery reuses read-name hints even for zero matches",async({page})=>{
 await page.setViewportSize({width:1434,height:898});
 const resource=structuredFixture({id:"search-portrait",title:"文献画像",sourceUri:"source://library/private-paper-id/search-portrait",revision:3,content:{},pendingFieldIds:[]});
 const cases=[
  {name:"research_search_sources",arguments:{query:"证据"},output:{matches:[{sourceUrl:resource.sourceUri}]}},
  {name:"research_search_sources",arguments:{query:"证据",resourceId:resource.sourceUri},output:{matches:[]}},
  {name:"research_find_sources",arguments:{nameQuery:"画像",resourceId:resource.sourceUri},output:{sources:[]}},
  {name:"research_list_sources",arguments:{resourceId:resource.sourceUri},output:{sources:[]}},
  {name:"research_list_sources",arguments:{},output:{sources:[]}},
  {name:"research_search_sources",arguments:{query:"证据",fileId:"opaque-file-handle"},output:{matches:[],scope:{sourceUrl:resource.sourceUri,name:"文献画像"}}},
 ];
 const fixture=await openToolJournal(page,{steps:cases,resources:[resource]});
 await fixture.locator(".writing-agent-turn-process > summary").click();
 await fixture.locator("[data-agent-activity-group] > summary").click();
 const titles=fixture.locator(".tool-row-compact");await expect(titles).toHaveCount(6);
 for(const [index,label] of ["已搜索 证据","已搜索 证据（文献画像）","已查找（文献画像，名称 画像）","已列出文件（范围 所有文献 / 论文甲 / 文献画像）","已列出文件","已搜索 证据（文献画像）"].entries())await expect(titles.nth(index)).toHaveText(label);
 await expect(titles.nth(0)).not.toContainText("范围");await expect(titles.nth(1)).not.toContainText("范围");await expect(fixture).not.toContainText("source://");await expect(fixture).not.toContainText("opaque-file-handle");
 const name=titles.nth(1).locator(".tool-read-file-name");await expect(name).toHaveCSS("text-decoration-line","underline");
 await name.hover();const hint=page.getByRole("tooltip");await expect(hint).toContainText("文献画像");await expect(hint).toContainText("v3");await expect(hint).not.toContainText("source://");
 const bounds=await hint.boundingBox();expect(bounds).not.toBeNull();
 await expect(hint).toBeVisible();
 const hit=await name.evaluate(el=>{const b=el.getBoundingClientRect();return el.contains(document.elementFromPoint(b.x+b.width/2,b.y+b.height/2));});expect(hit).toBe(true);
 await page.screenshot({path:".tmp/tool-discovery-scope.png"});
 const running=await openToolJournal(page,{steps:cases,resources:[resource],pendingStep:{name:"research_search_sources",arguments:{query:"新证据",resourceId:resource.sourceUri}}});
 const headline=running.locator("[data-agent-activity-group] > summary .writing-agent-activity-title");
 await expect(headline).toHaveText("正在搜索 新证据（文献画像）");
 await expect(headline).toHaveAttribute("data-shimmer-active","true");
 await headline.locator(".tool-read-file-name").hover();
 await expect(page.getByRole("tooltip")).toContainText("文献画像");await expect(page.getByRole("tooltip")).toContainText("v3");
});
