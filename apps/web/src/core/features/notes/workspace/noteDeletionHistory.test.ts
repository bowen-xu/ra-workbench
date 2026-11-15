// @vitest-environment happy-dom
import {beforeEach,expect,it,vi} from "vitest";
import {noteBlockId,type NoteBlockV1} from "@research-assistant/core/notes";
import {NotesClient} from "../NotesClient";
import {NotesGatewayError} from "@research-assistant/data/notes";
import {browserNotesGateway as gateway} from "../gateways/browserNotesGateway";
import {selectAppStorage} from "../../../runtime/appStorage";
import {applyNotePageStatus} from "../../../../../../../plugins/note_manager/src/features/notes/workspace/noteFavoriteActions";
vi.mock("../../../services/notifications",()=>({notify:{success:vi.fn(),error:vi.fn(),update:vi.fn()}}));
beforeEach(()=>{localStorage.clear();selectAppStorage(localStorage);});
async function fixture(){
  const parent=await gateway.createPage({title:"Parent"}),child=await gateway.createPage({title:"Child"});
  const link=(id:string,entry:boolean):NoteBlockV1=>({id:noteBlockId(id),type:"researchObjectReference",props:entry?{pageEntry:true}:{},children:[],content:{kind:"reference",targetKind:"note_page",targetId:child.page.id,labelSnapshot:"Child",quoteSnapshot:""}});
  await gateway.saveDocument({document:{...parent.document,blocks:[link("entry",true),link("ordinary",false)]},expectedRevision:parent.page.revision,saveReason:"manual"});
  return{parent,child,client:new NotesClient(gateway)};
}
it("uses one shared note command for delete, undo, redo and repeated undo, including entrance restoration",async()=>{
  const {parent,child,client}=await fixture();
  await applyNotePageStatus(client,child.page.id,"trashed");
  expect((await gateway.loadDocument(parent.page.id)).document.blocks.map(b=>b.id)).toEqual(["ordinary"]);
  await client.history.run("undo");
  expect((await gateway.loadDocument(child.page.id)).page.status).toBe("active");
  expect((await gateway.loadDocument(parent.page.id)).document.blocks.map(b=>b.id)).toEqual(["entry","ordinary"]);
  await client.history.run("redo");
  expect((await gateway.loadDocument(child.page.id)).page.status).toBe("trashed");
  await client.history.run("undo");
  expect((await gateway.loadDocument(parent.page.id)).document.blocks.map(b=>b.id)).toEqual(["entry","ordinary"]);
});
it("restores only entrance identities, retaining source text written after deletion",async()=>{
  const {parent,child,client}=await fixture();await applyNotePageStatus(client,child.page.id,"trashed");
  const source=await gateway.loadDocument(parent.page.id);
  const text:NoteBlockV1={id:noteBlockId("new-text"),type:"paragraph",props:{},children:[],content:{kind:"inline",items:[{type:"text",text:"later text",styles:[]}]}};
  await gateway.saveDocument({document:{...source.document,blocks:[...source.document.blocks,text]},expectedRevision:source.page.revision,saveReason:"manual"});
  await client.history.run("undo");
  expect((await gateway.loadDocument(parent.page.id)).document.blocks.map(b=>b.id)).toEqual(["entry","ordinary","new-text"]);
});
it("keeps failed history retryable and never partially restores a changed target",async()=>{
  const {parent,child,client}=await fixture();await applyNotePageStatus(client,child.page.id,"trashed");
  const current=await gateway.loadDocument(child.page.id);
  await gateway.applyMetadataBatch({operationId:"another-restore",action:"restore",items:[{id:child.page.id,expectedRevision:current.page.revision}]});
  await expect(client.history.run("undo")).rejects.toThrow("note_history_changed");
  expect((await gateway.loadDocument(parent.page.id)).document.blocks.map(b=>b.id)).toEqual(["ordinary"]);
});
it("does not resurrect descendants that were already trashed before their parent's deletion",async()=>{
  const parent=await gateway.createPage({title:"Parent"}),old=await gateway.createPage({title:"Old child",parentPageId:parent.page.id});
  const client=new NotesClient(gateway);await applyNotePageStatus(client,old.page.id,"trashed");
  await applyNotePageStatus(client,parent.page.id,"trashed");await client.history.run("undo");
  expect((await gateway.loadDocument(parent.page.id)).page.status).toBe("active");
  expect((await gateway.loadDocument(old.page.id)).page.status).toBe("trashed");
  await client.history.run("redo");expect((await gateway.loadDocument(old.page.id)).page.status).toBe("trashed");
});
it("reconciles a lost replay acknowledgement with the identical request instead of inserting the entrance twice",async()=>{
  const {parent,child}=await fixture();let calls=0;
  const replay=vi.fn<NonNullable<typeof gateway.replayMetadataBatch>>(async request=>{
    const result=await gateway.replayMetadataBatch!(request);if(++calls===1)throw new NotesGatewayError("operation_unknown","lost acknowledgement");return result;
  });
  const client=new NotesClient({...gateway,replayMetadataBatch:replay});await applyNotePageStatus(client,child.page.id,"trashed");await client.history.run("undo");
  expect(replay).toHaveBeenCalledTimes(2);expect(replay.mock.calls[0]![0]).toEqual(replay.mock.calls[1]![0]);
  expect((await gateway.loadDocument(parent.page.id)).document.blocks.map(b=>b.id)).toEqual(["entry","ordinary"]);
});
