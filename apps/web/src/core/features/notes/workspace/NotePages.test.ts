// @vitest-environment happy-dom
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {noteBlockId,notePageId} from "@research-assistant/core/notes";
import type {NoteDocumentSnapshot} from "@research-assistant/data/notes";
import {NotesClient} from "../NotesClient";
import {browserNotesGateway} from "../gateways/browserNotesGateway";
import * as creationStorage from "../gateways/browserNoteDraft";
import {AppNoteHbsSession} from "./appNoteHbsSession";
import {selectAppRuntime} from "../../../runtime/appRuntime";
import {getPendingHbsDocumentCommit} from "../../../services/hbsDocumentGateway";
import {createRecoveryDraft,saveRecoveryDraft} from "../../../services/recoveryDraftStore";
import {noteDraftIdentity,markNoteDraftIdentity,isAbandonedEmptyNoteDraft} from "./noteDraftIdentity";

const controllers:AppNoteHbsSession[]=[];
beforeEach(()=>{localStorage.clear();selectAppRuntime({mode:"browser",storage:localStorage,gateways:{},dispose(){}});});
afterEach(()=>{for(const controller of controllers.splice(0))controller.dispose();vi.restoreAllMocks();});
async function fixture(favorite=false,source?:Parameters<NotesClient["creation"]["create"]>[1]){
  const client=new NotesClient(browserNotesGateway),initial=await client.creation.create(favorite,source);
  const controller=await AppNoteHbsSession.open({loaded:initial,onSnapshot:vi.fn(),onDurable:vi.fn(),onStatus:vi.fn()});
  controllers.push(controller);controller.start();return{client,initial,controller};
}
it("stores an empty canonical page, baseline and favorite immediately, including after reopening",async()=>{
  const first=await fixture(true),second=await fixture(true);
  expect(first.client.creation.isDraft(first.initial.page.id)).toBe(false);
  expect(first.client.creation.gateway(first.initial.page.id,vi.fn())).toBeUndefined();
  expect(isAbandonedEmptyNoteDraft(first.initial.page.id,[])).toBe(false);
  expect(first.initial.page.title).toBe("无标题");expect(first.initial.page.revision).toBe(1);
  expect(await browserNotesGateway.listFavoritePageIds()).toEqual([second.initial.page.id,first.initial.page.id]);
  const loaded=await new NotesClient(browserNotesGateway).gateway.loadDocument(first.initial.page.id);
  expect(loaded.document).toEqual(first.initial.document);
  expect(first.controller.session.getSnapshot().content).toEqual(first.initial.document);
});
it("uses the ordinary HBS save and preservation chain from the first edit",async()=>{
  const f=await fixture();
  f.controller.enqueue({...f.initial.document,title:"First title"});await f.controller.flush();
  expect((await browserNotesGateway.loadDocument(f.initial.page.id)).page.title).toBe("First title");
  f.controller.enqueue({...f.initial.document,title:"Recover this input"});await f.controller.ensurePreserved();f.controller.dispose();
  const loaded=await browserNotesGateway.loadDocument(f.initial.page.id);
  const reopened=await AppNoteHbsSession.open({loaded,onSnapshot:vi.fn(),onDurable:vi.fn(),onStatus:vi.fn()});controllers.push(reopened);
  expect(reopened.session.getSnapshot().content.title).toBe("Recover this input");
});
it("creates an empty page and its automatic source entrance in one transaction before typing",async()=>{
  const parent=await browserNotesGateway.createPage({title:"Parent"});
  const f=await fixture(false,{pageId:parent.page.id});
  const source=await browserNotesGateway.loadDocument(parent.page.id);
  expect(source.document.blocks.at(-1)).toMatchObject({props:{pageEntry:true},content:{kind:"reference",targetId:f.initial.page.id,labelSnapshot:"无标题"}});
  expect(source.page.revision).toBe(parent.page.revision+1);
  expect((await browserNotesGateway.loadDocument(f.initial.page.id)).page.title).toBe("无标题");
});
it("rejects a removed source insertion position without creating an orphan page",async()=>{
  const parent=await browserNotesGateway.createPage({title:"Parent"}),client=new NotesClient(browserNotesGateway);
  await expect(client.creation.create(false,{pageId:parent.page.id,blockId:"deleted"})).rejects.toThrow("MODEL_DRAFT_SOURCE_CHANGED");
  expect(await browserNotesGateway.listPages({kind:"all"})).toHaveLength(1);
  expect((await browserNotesGateway.loadDocument(parent.page.id)).page.revision).toBe(parent.page.revision);
});
it("replaces the captured slash block atomically and preserves its children",async()=>{
  const created=await browserNotesGateway.createPage({title:"Parent"}),original=created.document.blocks[0]!;
  const anchor={...original,children:[{...original,id:noteBlockId("nested")}]};
  const parent=await browserNotesGateway.saveDocument({document:{...created.document,blocks:[anchor]},expectedRevision:created.page.revision,saveReason:"manual"});
  const f=await fixture(false,{pageId:parent.page.id,blockId:anchor.id,anchor});
  expect((await browserNotesGateway.loadDocument(parent.page.id)).document.blocks[0]).toMatchObject({id:anchor.id,children:anchor.children,props:{pageEntry:true},content:{targetId:f.initial.page.id}});
});
it("does not overwrite newer source input or create a page from a stale anchor",async()=>{
  const parent=await browserNotesGateway.createPage({title:"Parent"}),anchor=parent.document.blocks[0]!;
  await browserNotesGateway.saveDocument({document:{...parent.document,blocks:[{...anchor,content:{kind:"inline",items:[{type:"text",text:"Later input",styles:[]}]}}]},expectedRevision:parent.page.revision,saveReason:"manual"});
  const client=new NotesClient(browserNotesGateway);
  await expect(client.creation.create(false,{pageId:parent.page.id,blockId:anchor.id,anchor})).rejects.toThrow("MODEL_DRAFT_SOURCE_CHANGED");
  expect(await browserNotesGateway.listPages({kind:"all"})).toHaveLength(1);
});
it("retries an unknown result with the exact request and no duplicate page or source entry",async()=>{
  const parent=await browserNotesGateway.createPage({title:"Parent"}),client=new NotesClient(browserNotesGateway);
  const publish=creationStorage.publishBrowserNoteDraft;
  const spy=vi.spyOn(creationStorage,"publishBrowserNoteDraft").mockImplementationOnce(async(id,request)=>{await publish(id,request);throw new Error("receipt transport lost");});
  const error=await client.creation.create(false,{pageId:parent.page.id}).then(()=>{throw new Error("Expected receipt failure");},reason=>reason as Error&{retryCreation:()=>Promise<NoteDocumentSnapshot>});
  expect(error).toBeInstanceOf(Error);
  const id=spy.mock.calls[0]![0];expect(getPendingHbsDocumentCommit("note",id)).toBeDefined();
  const created=await error.retryCreation();
  expect(spy.mock.calls[1]).toEqual(spy.mock.calls[0]);expect(created.page.id).toBe(id);
  expect((await browserNotesGateway.loadDocument(parent.page.id)).document.blocks.filter(block=>block.content.kind==="reference"&&block.content.targetId===id)).toHaveLength(1);
  expect(await browserNotesGateway.listPages({kind:"all"})).toHaveLength(2);
  expect(getPendingHbsDocumentCommit("note",id)).toBeUndefined();
});
it("does not open an unpublished page while creation is pending and deduplicates a repeated click",async()=>{
  const publish=creationStorage.publishBrowserNoteDraft;
  let release!:()=>void,entered!:()=>void;
  const waiting=new Promise<void>(resolve=>{release=resolve;}),started=new Promise<void>(resolve=>{entered=resolve;});
  const spy=vi.spyOn(creationStorage,"publishBrowserNoteDraft").mockImplementationOnce(async(id,request)=>{entered();await waiting;return publish(id,request);});
  const client=new NotesClient(browserNotesGateway);let opened=false;
  const first=client.creation.create().then(page=>{opened=true;return page;});await started;
  const repeated=client.creation.create();expect(opened).toBe(false);expect(await browserNotesGateway.listPages({kind:"all"})).toHaveLength(0);
  release();expect((await first).page.id).toBe((await repeated).page.id);expect(spy).toHaveBeenCalledTimes(1);
});
it("keeps older unpublished recovery records usable while all new creations are canonical",async()=>{
  const canonical=await fixture(),id=canonical.initial.page.id;
  const legacy={initial:canonical.initial,authority:{version:0,saved:canonical.initial.document,baseline:canonical.initial.document}};
  saveRecoveryDraft(createRecoveryDraft(noteDraftIdentity(id),legacy));markNoteDraftIdentity(id,true);
  const reopened=new NotesClient(browserNotesGateway);
  expect(reopened.creation.snapshot(id)?.document).toEqual(canonical.initial.document);
  await reopened.creation.observeStored(canonical.initial);
  expect(reopened.creation.snapshot(id)).toBeUndefined();expect(reopened.creation.gateway(id,vi.fn())).toBeUndefined();
});
it("restores and saves real input preserved by an older unpublished editor",async()=>{
  const f=await fixture(),id=notePageId(crypto.randomUUID());
  const initial={...f.initial,page:{...f.initial.page,id,revision:0},document:{...f.initial.document,pageId:id}};
  saveRecoveryDraft(createRecoveryDraft(noteDraftIdentity(id),{initial,authority:{version:0,saved:initial.document,baseline:initial.document}}));
  const old=new NotesClient(browserNotesGateway);
  const controller=await AppNoteHbsSession.open({loaded:initial,gateway:old.creation.gateway(id,vi.fn())!,onSnapshot:vi.fn(),onDurable:vi.fn(),onStatus:vi.fn()});
  controller.enqueue({...initial.document,title:"Older unsubmitted input"});await controller.ensurePreserved();controller.dispose();
  await expect(browserNotesGateway.loadDocument(id)).rejects.toThrow();
  const client=new NotesClient(browserNotesGateway),loaded=client.creation.snapshot(id)!;
  const reopened=await AppNoteHbsSession.open({loaded,gateway:client.creation.gateway(id,page=>reopened.adoptCreatedMetadata(page))!,onSnapshot:vi.fn(),onDurable:vi.fn(),onStatus:vi.fn()});controllers.push(reopened);
  expect(reopened.session.getSnapshot().content.title).toBe("Older unsubmitted input");
  reopened.start();await reopened.flush();
  expect((await browserNotesGateway.loadDocument(id)).page.title).toBe("Older unsubmitted input");
});
