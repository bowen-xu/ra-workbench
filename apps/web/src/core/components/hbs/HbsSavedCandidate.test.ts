import { afterEach, expect, it } from "vitest";
import { HbsSession, DocumentVersionConflict, createTextDocumentAdapter, createNoteDocumentAdapter, createStructuredDocumentAdapter,
  type DocumentCheckpoint, type DocumentPath, type DocumentSnapshot, type HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { noteFixture, structuredFixture } from "../../../app/previews/hbs-adapters/fixtures";
import { directTextDiff } from "./codeMirrorHbsPolicy";

const kinds=["text","note","cell","structured"] as const;
const sessions:HbsSession<unknown>[]=[];
afterEach(()=>sessions.splice(0).forEach(session=>session.dispose()));
const settle=async()=>{for(let n=0;n<80;n++)await Promise.resolve();};
async function fixture(kind:typeof kinds[number]){
  const adapter:HbsDocumentAdapter<unknown>=kind==="text"?createTextDocumentAdapter(directTextDiff)
    :kind==="note"||kind==="cell"?createNoteDocumentAdapter(directTextDiff):createStructuredDocumentAdapter(directTextDiff);
  const path:DocumentPath=kind==="text"?[]:kind==="note"?["blocks",{key:"id",value:"hbs-score"},"content","items",0,"text"]
    :kind==="cell"?["blocks",{key:"id",value:"hbs-table"},"content","rows",1,1,"items",0,"text"]:["fields",{key:"field",value:"research_question"},"statements",{key:"id",value:"hbs-statement-0"},"text"];
  const initial=kind==="text"?"score 80%\n\nmethod old":adapter.replace(kind==="note"||kind==="cell"?noteFixture:structuredFixture,path,"score 80%\n\nmethod old");
  let server:DocumentSnapshot<unknown>={version:1,saved:initial,baseline:initial},stored:DocumentCheckpoint<unknown>|undefined;
  const options={adapter,requestId:(n:number)=>`r-${n}`,preservation:{load:()=>stored,save:(value:DocumentCheckpoint<unknown>)=>{stored=structuredClone(value);}},port:{
    read:async()=>server,commit:async(request:{id:string;expectedVersion:number;saved:unknown;baseline:unknown})=>{
      if(request.expectedVersion!==server.version)throw new DocumentVersionConflict(server,request.id);
      server={version:server.version+1,saved:request.saved,baseline:request.baseline};return server;
    }}};
  let session=await HbsSession.open(options);sessions.push(session);
  const text=(value:unknown)=>String(adapter.read(value,path));
  const replace=(value:unknown,from:string,to:string)=>adapter.replace(value,path,text(value).replace(from,to));
  return {adapter,path,text,get session(){return session;},get server(){return server;},get stored(){return stored;},
    external:(from:string,to:string)=>{server={...server,version:server.version+1,saved:replace(server.saved,from,to)};},
    edit:(from:string,to:string,ordinary=false)=>{const state=session.getSnapshot();session.edit(replace(state.content,from,to),ordinary?replace(state.baselineProjection,from,to):state.baselineProjection);},
    restart:async()=>{session.dispose();session=await HbsSession.open(options);sessions.push(session);}};
}

it.each(kinds)("%s keeps the same human candidate conflict before and after saving",async kind=>{
  for(const save of [false,true]){
    const f=await fixture(kind);f.external("80%","85%");await f.session.syncLatest();f.edit("85%","91%");
    if(save)await f.session.save();
    f.external(save?"91%":"85%","90%");await f.session.syncLatest();
    expect(f.text(f.session.getSnapshot().content)).toContain("91%");
    expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
    expect(f.text(f.server.baseline)).toContain("80%");
  }
});
it.each(kinds)("%s persists the unconfirmed branch through restart and repeated saves",async kind=>{
  const f=await fixture(kind);f.external("80%","85%");await f.session.syncLatest();f.edit("85%","91%");
  await f.session.save();await f.session.save();expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await f.restart();f.edit("91%","92%");expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await f.session.save();f.external("92%","90%");await f.session.syncLatest();
  expect(f.text(f.session.getSnapshot().content)).toContain("92%");expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
  await f.restart();expect(f.text(f.session.getSnapshot().content)).toContain("92%");expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
});
it.each(kinds)("%s releases only an explicit Keep or Undo, not a candidate save",async kind=>{
  for(const action of ["keep","undo"] as const){
    const f=await fixture(kind);f.external("80%","85%");await f.session.syncLatest();f.edit("85%","91%");await f.session.save();
    const entry=f.session.perform(()=>f.session.resolve(f.session.getSnapshot().changes[0]!.id,action));await settle();
    const confirmed=action==="keep"?"91%":"80%";expect(f.text(f.server.baseline)).toContain(confirmed);
    f.external(confirmed,"95%");await f.session.syncLatest();
    expect(f.text(f.session.getSnapshot().content)).toContain("95%");expect(f.session.getSnapshot().conflicts).toHaveLength(0);
    expect(entry).toBeDefined();
  }
});
it.each(kinds)("%s does not turn unchanged saved new or compatible external updates into conflicts",async kind=>{
  const f=await fixture(kind);f.external("80%","85%");await f.session.syncLatest();f.edit("85%","91%");await f.session.save();
  f.external("method old","method remote");await f.session.syncLatest();expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  f.edit("91%","92%");expect(f.session.getSnapshot().conflicts).toHaveLength(0);await f.session.save();
  f.external("method remote","method newer");await f.session.syncLatest();expect(f.text(f.session.getSnapshot().content)).toContain("method newer");
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  f.external("92%","95%");await f.session.syncLatest();expect(f.text(f.session.getSnapshot().content)).toContain("92%");expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
});

it.each(kinds)("%s releases a candidate typed back to B only after saving it",async kind=>{
  const f=await fixture(kind);f.external("80%","85%");await f.session.syncLatest();f.edit("85%","91%");await f.session.save();
  f.edit("91%","80%");await f.session.save();expect(f.session.getSnapshot().changes).toHaveLength(0);
  f.external("80%","95%");await f.session.syncLatest();
  expect(f.text(f.session.getSnapshot().content)).toContain("95%");expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});

it.each(kinds)("%s retains other saved human new when Keep confirms only one range",async kind=>{
  const f=await fixture(kind);f.external("80%","85%");f.external("method old","method incoming");await f.session.syncLatest();
  f.edit("85%","91%");f.edit("method incoming","method mine");await f.session.save();
  const state=f.session.getSnapshot();
  f.session.review(state.content,f.adapter.replace(state.baselineProjection,f.path,f.text(state.baselineProjection).replace("80%","91%")));await settle();
  expect(f.text(f.server.baseline)).toContain("91%");expect(f.text(f.server.baseline)).toContain("method old");
  f.external("91%","95%");f.external("method mine","method newer");await f.session.syncLatest();
  expect(f.text(f.session.getSnapshot().content)).toContain("95%");expect(f.text(f.session.getSnapshot().content)).toContain("method mine");
  expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
});
