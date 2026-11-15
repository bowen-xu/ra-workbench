// @vitest-environment happy-dom
import {beforeEach,expect,it} from "vitest";
import {createNoteDocumentAdapter,HbsSession,type DocumentCommit} from "@research-assistant/core/hbs";
import {noteBlockId,type NoteBlockV1,type NoteDocumentV1} from "@research-assistant/core/notes";
import {createDocumentPort,createDocumentPreservationPort} from "@research-assistant/data/hbs";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {selectAppStorage} from "../../../runtime/appStorage";
import {createMemoryNotesDatabase} from "../../../runtime/testing/memoryNotesDatabase";
import {configureBrowserNotesDatabase} from "./browserNotesDatabase";
import {browserNotesGateway as notes} from "./browserNotesGateway";
import {createBrowserNoteHbsAuthority} from "./browserNoteHbsAuthority";
const adapter=createNoteDocumentAdapter(directTextDiff);
const entrance=(id:string,target:string,owned=true):NoteBlockV1=>({id:noteBlockId(id),type:"researchObjectReference",props:{pageEntry:owned},children:[],content:{kind:"reference",targetKind:"note_page",targetId:target,labelSnapshot:"Page",quoteSnapshot:""}});
beforeEach(()=>{localStorage.clear();selectAppStorage(localStorage);configureBrowserNotesDatabase(createMemoryNotesDatabase().open);});
it.each([false,true])("cut/paste and original HBS history restore one resource (legacy=%s)",async legacy=>{
 const parent=await notes.createPage({title:"Parent"});const child=await notes.createPage({title:"Child",...(legacy?{parentPageId:parent.page.id}:{})});const authority=createBrowserNoteHbsAuthority();
 let point=await authority.read(parent.page.id);
 if(!legacy){const doc={...point.saved,blocks:[...point.saved.blocks,entrance("entry",child.page.id)]};await authority.commit(parent.page.id,{id:"entry",expectedVersion:point.version,saved:doc,baseline:doc});point=await authority.read(parent.page.id);}
 let serial=0;const session=await HbsSession.open({adapter,requestId:()=>`save:${++serial}`,port:createDocumentPort(adapter,(op,request)=>op==="read"?authority.read(parent.page.id):authority.commit(parent.page.id,request as DocumentCommit<unknown>)),preservation:createDocumentPreservationPort(adapter,(op,value)=>op==="readPreservation"?authority.readPreservation(parent.page.id):authority.savePreservation(parent.page.id,value))});
 const empty={...point.saved,blocks:point.saved.blocks.filter(b=>!(b.content.kind==="reference"&&b.content.targetId===child.page.id))};
 const cut=session.perform(()=>session.edit(empty));await session.save();expect((await notes.loadDocument(child.page.id)).page.status).toBe("trashed");
 const pasted={...empty,blocks:[...empty.blocks,entrance("new-block-id",child.page.id)]};const paste=session.perform(()=>session.edit(pasted));await session.save();
 expect((await notes.loadDocument(child.page.id)).page).toMatchObject({id:child.page.id,status:"active"});
 session.restore(paste,"before");await session.save();expect((await notes.loadDocument(child.page.id)).page.status).toBe("trashed");
 session.restore(cut,"before");await session.save();expect((await notes.loadDocument(child.page.id)).page.status,session.getSnapshot().message).toBe("active");
 session.restore(cut,"after");await session.save();expect((await notes.loadDocument(child.page.id)).page.status).toBe("trashed");
 session.restore(paste,"after");await session.save();expect((await notes.loadDocument(child.page.id)).page.status).toBe("active");
 expect(session.getSnapshot().pending).toBe(false);expect(session.getSnapshot().conflicts).toEqual([]);session.dispose();
});
it("paste failure is atomic and duplicate success does not repeat lifecycle changes",async()=>{
 const memory=createMemoryNotesDatabase();let fail=false;configureBrowserNotesDatabase(async name=>{const db=await memory.open(name);return{...db,commit:async(...args)=>{if(fail)throw new Error("synthetic disk full");return db.commit(...args);}};});
 const source=await notes.createPage({title:"Source"}),target=await notes.createPage({title:"Target"}),authority=createBrowserNoteHbsAuthority();let point=await authority.read(source.page.id);
 const doc={...point.saved,blocks:[entrance("entry",target.page.id)]};await authority.commit(source.page.id,{id:"entry",expectedVersion:point.version,saved:doc,baseline:doc});point=await authority.read(source.page.id);
 const empty={...doc,blocks:[]};await authority.commit(source.page.id,{id:"cut",expectedVersion:point.version,saved:empty,baseline:empty});point=await authority.read(source.page.id);
 const request={id:"paste",expectedVersion:point.version,saved:doc,baseline:doc};fail=true;await expect(authority.commit(source.page.id,request)).rejects.toThrow("synthetic disk full");
 expect(await authority.read(source.page.id)).toEqual(point);expect((await notes.loadDocument(target.page.id)).page.status).toBe("trashed");
 fail=false;const receipt=await authority.commit(source.page.id,request),revision=(await notes.loadDocument(target.page.id)).page.revision;
 expect(await authority.commit(source.page.id,request)).toEqual(receipt);expect((await notes.loadDocument(target.page.id)).page.revision).toBe(revision);
});
it("ordinary references do not restore pages and another live entrance retains them",async()=>{
 const a=await notes.createPage({title:"A"}),b=await notes.createPage({title:"B"}),child=await notes.createPage({title:"Child"}),authority=createBrowserNoteHbsAuthority();
 async function save(id:typeof a.page.id,key:string,blocks:NoteDocumentV1["blocks"]){const point=await authority.read(id),doc={...point.saved,blocks};return authority.commit(id,{id:key,expectedVersion:point.version,saved:doc,baseline:doc});}
 await save(a.page.id,"a",[entrance("a",child.page.id)]);await save(b.page.id,"b",[entrance("b",child.page.id)]);await save(a.page.id,"cut-a",[]);
 expect((await notes.loadDocument(child.page.id)).page.status).toBe("active");await save(b.page.id,"cut-b",[entrance("ordinary",child.page.id,false)]);expect((await notes.loadDocument(child.page.id)).page.status).toBe("trashed");
 await save(a.page.id,"ordinary",[entrance("ordinary-copy",child.page.id,false)]);expect((await notes.loadDocument(child.page.id)).page.status).toBe("trashed");
});
