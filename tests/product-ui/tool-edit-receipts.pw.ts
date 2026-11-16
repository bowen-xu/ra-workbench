import {test,expect} from "@playwright/test";
import {openToolJournal, setToolDeveloperMode, structuredFixture, toolHistoryRow} from "./tool-journal";
test("saved tool counts match the card after draft repair and failed edits reveal full commands only in developer data",async({page})=>{
 await page.setViewportSize({width:1434,height:898});
 const after={lines:Array.from({length:108},(_,index)=>`Saved line ${index+1}`)};
 const resource=structuredFixture({title:"文献画像",content:after,pendingFieldIds:["lines"],draft:null});
 const patch=`*** Begin Patch\n*** Update File: ${resource.sourceUri}\n@@\n-INVALID_DRAFT_MARKER\n-obsolete1\n-obsolete2\n-obsolete3\n*** End Patch`;
 const steps=["native","legacy","failed","declined"].map((kind,index)=>{
  const applied=index<2;
  const targetPath=applied?resource.sourceUri:`${resource.sourceUri}-${kind}`;
  const target={path:targetPath,name:resource.title,status:applied?"applied":"failed",message:applied?"Saved":"Draft retained",...(applied?{changeId:`saved-${kind}`} : {}),structuredResource:applied?{resourceId:resource.id,revision:1}:{status:"invalid",resourceId:resource.id,draftId:"synthetic-draft",errors:[{code:"invalid_evidence",message:"Synthetic quote mismatch",path:"$.claims[0].quote"}]}};
  const output=kind==="failed"?{status:"failed",targets:[{path:targetPath,name:resource.title,status:"failed",message:"Synthetic permission denial",code:"permission_denied",operationId:"synthetic-operation",httpStatus:403}]}:{status:applied?"complete":"failed",targets:[target],...applied?{fileDiffs:[kind==="native"?{path:resource.sourceUri,comparison:"saved",beforeSaved:null,afterSaved:after}:{path:resource.sourceUri,hunks:[{lines:["-INVALID_DRAFT_MARKER","-obsolete1","-obsolete2","-obsolete3"]}]}]}:{}};
  return {name:"research_apply_patch",arguments:{patch},output,isError:!applied};
 });
 const fixture=await openToolJournal(page,{steps,resources:[resource],currentResources:[{...resource,revision:7,content:{later:"Current content must not affect history"}}]});
 await fixture.locator(".writing-agent-turn-process > summary").click();
 await fixture.locator("[data-agent-activity-group] > summary").click();
 for(const index of [0,1]){
  const row=await toolHistoryRow(page,index);
  await expect(row.locator(".tool-row-compact .tool-patch-added-count")).toHaveText("+112");
  await expect(row.locator(".tool-row-compact .tool-patch-deleted-count")).toHaveText("-0");
 }
 const card=fixture.locator(".writing-agent-inline-diff-card");
 await expect(card.locator("header .writing-agent-inline-diff-counts")).toHaveText("+112 −0");
 for(const index of [2,3]){
  const row=await toolHistoryRow(page,index);
  await row.locator("summary").click();
  await expect(row.locator(".tool-shell-content")).toContainText("文献画像");
  await expect(row.locator(".tool-shell-content")).not.toContainText("*** Begin Patch");
  await expect(row.getByLabel("调用参数",{exact:true})).toHaveCount(0);
  for(const value of index===2?["permission_denied","synthetic-operation","403"]:["invalid_evidence","Synthetic quote mismatch"]){
   await expect(row.locator(".tool-shell-content")).toContainText(value);
  }
  await row.locator("summary").click();
 }
 await page.screenshot({path:".tmp/tool-edit-formal-counts-and-errors.png"});
 await setToolDeveloperMode(page,true);
 const failed=await toolHistoryRow(page,2);
 await failed.locator("summary").click();
 await failed.getByRole("button",{name:"查看原始数据",exact:true}).click();
 const raw=failed.locator("xpath=..").getByLabel("消息原始数据",{exact:true});
 await expect(raw).toContainText("INVALID_DRAFT_MARKER");
 await expect(raw).toContainText("permission_denied");
 await expect(failed.locator(".tool-shell-content")).not.toContainText("INVALID_DRAFT_MARKER");

});
