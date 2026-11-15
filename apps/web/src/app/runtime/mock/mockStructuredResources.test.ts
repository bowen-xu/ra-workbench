// @vitest-environment happy-dom
import { afterEach,expect,it,vi } from "vitest";
import { createMockStructuredResources } from "./mockStructuredResources";
import { decodeStructuredState } from "./mockStructuredStateCodec";
const values=new Map<string,string>();
const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
afterEach(()=>storage.clear());
const content=(text:string)=>({kind:"paper_overview_unit_v1",schemaVersion:1,fields:[{field:"question",statements:[{id:"q",text,evidenceItemIds:[]}]}]});
it("precreates stable empty resources and keeps a failed draft across reopen before a successful repair",async()=>{
  let gateway=createMockStructuredResources(storage);
  const reading=(await gateway.list("paper")).find(value=>value.kind === "structuredReading")!;
  expect(reading.revision).toBe(0);expect(reading.content).toBeNull();
  const failed=await gateway.write({resourceId:reading.id,expectedRevision:0,requestId:"invalid",content:"{",editorKind:"agent"});
  expect(failed.status).toBe("invalid");expect(failed.resource.content).toBeNull();
  gateway=createMockStructuredResources(storage);expect((await gateway.get(reading.id)).draft?.content).toBe("{");
  const fixed=await gateway.write({resourceId:reading.id,expectedRevision:0,expectedDraftRevision:failed.draft!.revision,requestId:"repair",content:JSON.stringify(content("new")),editorKind:"agent"});
  expect(fixed.status).toBe("applied");expect(fixed.resource.id).toBe(reading.id);expect(fixed.resource.draft).toBeUndefined();expect(fixed.resource.pendingFieldIds).toEqual(expect.arrayContaining(["question","$metadata","$order"]));
  expect(await gateway.write({resourceId:reading.id,expectedRevision:0,requestId:"repair",content:JSON.stringify(content("new")),editorKind:"agent"})).toEqual(fixed);
  const undone=await gateway.review({resourceId:reading.id,expectedRevision:1,requestId:"undo",action:"undo",scope:"all"});expect(undone.pendingFieldIds).toEqual([]);
});
it("saves template changes with pending Diff and reviews them without a publication step",async()=>{
 const gateway=createMockStructuredResources(storage),template=await gateway.createTemplate("Template","create");
 const body={kind:"reading_template_v1",schemaVersion:1,name:"Template",fields:[{id:"q",title:"Question",question:"Why?"}]};
 const saved=await gateway.write({resourceId:template.id,expectedRevision:0,requestId:"save",content:JSON.stringify(body),editorKind:"agent"});
 expect(saved.status).toBe("applied");expect(saved.resource.draft).toBeUndefined();
 expect(saved.resource.pendingFieldIds).toContain("q");
 const kept=await gateway.review({resourceId:template.id,expectedRevision:saved.resource.revision,requestId:"keep",action:"keep",scope:"all"});
 expect(kept.pendingFieldIds).toEqual([]);
 const changed=await gateway.write({resourceId:template.id,expectedRevision:kept.revision,requestId:"change",content:JSON.stringify({...body,name:"Renamed",fields:[{...body.fields[0],question:"Changed"}]}),editorKind:"agent"});
 expect(changed.resource.reviewBaseline).toEqual(body);
 const undone=await gateway.review({resourceId:template.id,expectedRevision:changed.resource.revision,requestId:"undo-template",action:"undo",scope:"all"});
 expect(undone.content).toEqual(body);expect(undone.title).toBe("Template");expect((await gateway.getVersion(template.id,saved.resource.revision)).content).toEqual(body);
});
it("keeps paper template updates separate from its reading body and other papers",async()=>{
 const gateway=createMockStructuredResources(storage),resources=await gateway.list("paper");
 const template=resources.find(value=>value.kind==="readingTemplate")!,reading=resources.find(value=>value.kind==="structuredReading")!;
 const saved=await gateway.write({resourceId:template.id,expectedRevision:0,requestId:"save-paper-template",content:JSON.stringify({kind:"reading_template_v1",schemaVersion:1,name:"Paper template",fields:[{id:"q",title:"Question",question:"Why?"}]}),editorKind:"agent"});
 expect(saved.status).toBe("applied");expect(saved.resource.sourceUri).toBe(`source://library/paper/${template.id}`);
 expect((await gateway.get(reading.id)).templateRef).toBeUndefined();
 await expect(gateway.bindTemplate({paperId:"other",templateId:template.id,templateRevision:saved.resource.revision,expectedRevision:0,confirmed:true,requestId:"wrong-paper"})).rejects.toThrow();
});

it("rejects a partial Undo that would remove schema identity and keeps the saved version intact", async () => {
  const gateway = createMockStructuredResources(storage);
  const reading = (await gateway.list("paper")).find(value => value.kind === "structuredReading")!;
  const saved = await gateway.write({resourceId: reading.id, expectedRevision: 0, requestId: "save-schema",
    content: JSON.stringify(content("body")), editorKind: "agent"});
  await expect(gateway.review({resourceId: reading.id, expectedRevision: saved.resource.revision,
    requestId: "partial-undo", action: "undo", fieldIds: ["$metadata"]})).rejects.toThrow("破坏资源格式");
  expect((await gateway.get(reading.id)).content).toEqual(saved.resource.content);
  const restored = await gateway.review({resourceId: reading.id, expectedRevision: saved.resource.revision,
    requestId: "full-undo", action: "undo", scope: "all"});
  expect(restored.content).toBeNull();
});

it("restores the complete pre-generation version after a partial Keep", async () => {
  const gateway = createMockStructuredResources(storage);
  const reading = (await gateway.list("paper")).find(value => value.kind === "structuredReading")!;
  const saved = await gateway.write({resourceId: reading.id, expectedRevision: 0, requestId: "generation",
    content: JSON.stringify(content("body")), editorKind: "agent"});
  const kept = await gateway.review({resourceId: reading.id, expectedRevision: saved.resource.revision,
    requestId: "keep-body", action: "keep", fieldIds: ["question"]});
  const restored = await gateway.review({resourceId: reading.id, expectedRevision: kept.revision,
    requestId: "undo-generation", action: "undoGeneration"});
  expect(restored.content).toBeNull();
  expect(restored.pendingFieldIds).toEqual([]);
});

it("reopens trusted manual/review history and rejects stale or template replay",async()=>{
  let gateway=createMockStructuredResources(storage);
  const reading=(await gateway.list("history-paper")).find(value=>value.kind==="structuredReading")!;
  const initial=(await gateway.write({resourceId:reading.id,expectedRevision:0,requestId:"history-initial",editorKind:"human",content:JSON.stringify(content("before"))})).resource;
  const edited=(await gateway.write({resourceId:reading.id,expectedRevision:initial.revision,requestId:"history-agent",editorKind:"agent",content:JSON.stringify(content("after"))})).resource;
  const kept=await gateway.review({resourceId:reading.id,expectedRevision:edited.revision,requestId:"history-keep",action:"keep",scope:"all"});
  gateway=createMockStructuredResources(storage);
  const request={resourceId:reading.id,expectedRevision:kept.revision,operationRevision:kept.revision,direction:"undo" as const,requestId:"history-undo"};
  const undone=await gateway.restoreHistory(request);
  expect(undone.content).toEqual(edited.content);expect(undone.reviewBaseline).toEqual(edited.reviewBaseline);
  expect(await gateway.restoreHistory(request)).toEqual(undone);
  const redone=await gateway.restoreHistory({...request,expectedRevision:undone.revision,direction:"redo",requestId:"history-redo"});
  expect(redone.reviewBaseline).toEqual(kept.reviewBaseline);
  await expect(gateway.restoreHistory({...request,requestId:"history-stale"})).rejects.toThrow("已更新");
  const template=await gateway.createTemplate("unapproved","history-template");
  await expect(gateway.restoreHistory({...request,resourceId:template.id,expectedRevision:0,requestId:"history-template-undo"})).rejects.toThrow("找不到对应");
});

it("commits each review/history with one write, publishes complete receipts and leaves failed saves retryable", async () => {
  const gateway = createMockStructuredResources(storage);
  const reading = (await gateway.list("atomic-review")).find(value => value.kind === "structuredReading")!;
  const saved = (await gateway.write({resourceId: reading.id, expectedRevision: 0, requestId: "atomic-generate",
    content: JSON.stringify(content("pending")), editorKind: "agent"})).resource;
  const setItem = vi.spyOn(storage, "setItem");
  const request = {resourceId: reading.id, expectedRevision: saved.revision, requestId: "atomic-keep",
    action: "keep" as const, scope: "all" as const};
  let published = 0;
  const stop = await gateway.subscribeChanges!(() => {
    published++;
    const state = decodeStructuredState<{reviewReceipts: Record<string, unknown>}>(storage.getItem("mock-structured-resources:v1")!);
    expect(state.reviewReceipts[request.requestId]).toBeDefined();
  });
  try {
    setItem.mockImplementationOnce(() => {throw new Error("storage blocked");});
    await expect(gateway.review(request)).rejects.toThrow("storage blocked");
    expect((await gateway.get(reading.id)).revision).toBe(saved.revision);
    expect(published).toBe(0);
    setItem.mockClear();
    const kept = await gateway.review(request);
    expect(setItem).toHaveBeenCalledTimes(1); expect(published).toBe(1);
    expect(await gateway.review(request)).toEqual(kept);
    expect(setItem).toHaveBeenCalledTimes(1);
    const history = {resourceId: reading.id, expectedRevision: kept.revision,
      operationRevision: kept.revision, direction: "undo" as const, requestId: "atomic-history"};
    setItem.mockImplementationOnce(() => {throw new Error("history blocked");});
    await expect(gateway.restoreHistory(history)).rejects.toThrow("history blocked");
    expect((await gateway.get(reading.id)).revision).toBe(kept.revision);
    expect(published).toBe(1);
    setItem.mockClear();
    const undone = await gateway.restoreHistory(history);
    expect(setItem).toHaveBeenCalledTimes(1);
    expect(undone.pendingFieldIds).toEqual(saved.pendingFieldIds);
  } finally {stop(); setItem.mockRestore();}
});
