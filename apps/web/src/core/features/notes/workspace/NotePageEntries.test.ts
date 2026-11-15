// @vitest-environment happy-dom
import {beforeEach,expect,it,vi} from "vitest";
import {noteBlockId,type NoteBlockV1,type NoteDocumentV1} from "@research-assistant/core/notes";
import {createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {createDocumentPort} from "@research-assistant/data/hbs";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {browserNotesGateway as gateway} from "../gateways/browserNotesGateway";
import {AppNoteHbsSession} from "./appNoteHbsSession";
import {selectAppRuntime} from "../../../runtime/appRuntime";
import {getHbsDocumentGateway} from "../../../services/hbsDocumentGateway";
import {publishBrowserNoteDraft} from "../gateways/browserNoteDraft";

beforeEach(()=>{localStorage.clear();selectAppRuntime({mode:"browser",storage:localStorage,gateways:{},dispose(){}});});
const text=(id:string,value:string):NoteBlockV1=>({id:noteBlockId(id),type:"paragraph",props:{},children:[],content:{kind:"inline",items:[{type:"text",text:value,styles:[]}]}});
const link=(id:string,target:string,entry=false):NoteBlockV1=>({id:noteBlockId(id),type:"researchObjectReference",props:entry?{pageEntry:true}:{},children:[],content:{kind:"reference",targetKind:"note_page",targetId:target,labelSnapshot:"Child",quoteSnapshot:""}});
const authority=(id:string)=>createDocumentPort(createNoteDocumentAdapter(directTextDiff),()=>getHbsDocumentGateway().read("note",id)).read();
async function save(document:NoteDocumentV1){const current=await gateway.loadDocument(document.pageId);return gateway.saveDocument({document,expectedRevision:current.page.revision,saveReason:"manual"});}
async function trash(id:import("@research-assistant/core/notes").NotePageId,operationId=crypto.randomUUID()){
  const page=await gateway.loadDocument(id),request={operationId,action:"trash" as const,items:[{id,expectedRevision:page.page.revision}]};
  return {request,receipt:await gateway.applyMetadataBatch(request)};
}
it("removes marked entrances while retaining ordinary links, nested text, and unrelated pending HBS changes",async()=>{
  const parent=await gateway.createPage({title:"Parent"}),child=await gateway.createPage({title:"Child"});
  const entry={...link("entry",child.page.id,true),children:[text("nested","preserve nested content")]};
  const original={...parent.document,blocks:[text("body","accepted"),entry,link("ordinary",child.page.id)]};
  const before=await save(original),port=getHbsDocumentGateway();
  const candidate={...before.document,blocks:[text("body","pending candidate"),entry,link("ordinary",child.page.id)]};
  await port.commit("note",parent.page.id,{id:"candidate",expectedVersion:before.page.revision,saved:candidate,baseline:before.document});
  const owner=await AppNoteHbsSession.open({loaded:await gateway.loadDocument(parent.page.id),onSnapshot:vi.fn(),onDurable:vi.fn(),onStatus:vi.fn()});owner.start();
  try {
    const {request,receipt}=await trash(child.page.id);await owner.synchronizeExternal(await gateway.loadDocument(parent.page.id));
    expect(receipt.bodyChangedIds).toEqual([parent.page.id]);
    const current=await authority(parent.page.id);
    expect((current.saved as NoteDocumentV1).blocks.map(block=>block.id)).toEqual(["body","nested","ordinary"]);
    expect((current.baseline as NoteDocumentV1).blocks[0]).toEqual(text("body","accepted"));
    expect((current.baseline as NoteDocumentV1).blocks.map(block=>block.id)).toEqual(["body","nested","ordinary"]);
    expect(owner.session.getSnapshot().content.blocks.some(block=>block.id==="entry")).toBe(false);
    expect(owner.session.getSnapshot().conflicts).toHaveLength(0);
    expect(owner.session.getSnapshot().changes.length).toBeGreaterThan(0);
    expect(await gateway.applyMetadataBatch(request)).toEqual(receipt);
  } finally {owner.dispose();}
});
it("uses exact creation receipts for older auto entrances without deleting a second ordinary link to the same page",async()=>{
  const parent=await gateway.createPage({title:"Parent"}),childId=crypto.randomUUID() as typeof parent.page.id;
  const child={...parent.document,pageId:childId,title:"Child",blocks:[text("child-body","child body")]};
  const source={...parent.document,blocks:[link("older-auto-entry",childId)]};
  await publishBrowserNoteDraft(childId,{commit:{id:"old-create",expectedVersion:0,saved:child,baseline:child},favorite:false,source:{document:source,expectedRevision:parent.page.revision,saveReason:"structure"}});
  await save({...source,blocks:[...source.blocks,link("manual-later",childId)]});
  await trash(childId);
  expect((await gateway.loadDocument(parent.page.id)).document.blocks.map(block=>block.id)).toEqual(["manual-later"]);
});
it("removes and restores legacy entries in S and B together, preserving ordinary references",async()=>{
  const parent=await gateway.createPage({title:"Parent"}),child=await gateway.createPage({title:"Child",parentPageId:parent.page.id});
  const initial=await gateway.loadDocument(parent.page.id),before=await save({...initial.document,blocks:[...initial.document.blocks,link("ordinary",child.page.id)]});
  const port=getHbsDocumentGateway();await port.commit("note",parent.page.id,{id:"settled",expectedVersion:before.page.revision,saved:before.document,baseline:before.document});
  await trash(child.page.id);
  const removed=await authority(parent.page.id);expect(removed.saved).toEqual(removed.baseline);
  expect((removed.saved as NoteDocumentV1).blocks.some(block=>block.type==="childPage")).toBe(false);
  const current=await gateway.loadDocument(child.page.id);
  await gateway.applyMetadataBatch({operationId:"restore",action:"restore",items:[{id:child.page.id,expectedRevision:current.page.revision}]});
  const restored=await authority(parent.page.id);expect(restored.saved).toEqual(restored.baseline);
  expect((restored.saved as NoteDocumentV1).blocks.some(block=>block.type==="childPage")).toBe(true);
  expect((restored.saved as NoteDocumentV1).blocks.some(block=>block.id==="ordinary")).toBe(true);
});
