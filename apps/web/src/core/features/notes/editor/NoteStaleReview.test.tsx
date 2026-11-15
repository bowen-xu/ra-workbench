// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {createPortal} from "react-dom";
import {afterEach,expect,it,vi} from "vitest";
import {HbsSession,createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {notePageId,noteBlockId,type NoteDocumentV1} from "@research-assistant/core/notes";
import {HbsNoteEditor} from "../../../components/hbs/HbsNoteEditor";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {selectAppStorage,getAppStorage} from "../../../runtime/appStorage";
import {selectAppRuntime} from "../../../runtime/appRuntime";
import {getNoteReviewSaveState,useNoteReviewSaveState} from "./noteTextEditorPorts";
import {loadNoteRecoveryDraft} from "../workspace/noteRecovery";
import type {TextPatchResolution,TextPatchReview} from "../../agent/textPatchGateway";
import {undo} from "@tiptap/pm/history";
import type {NoteEditorInstance} from "./noteEditorSchema";
import {notify} from "../../../services/notifications";
import {NoteSaveState} from "../workspace/NoteSaveState";
import {notifyTextPatchesChanged} from "../../agent/textPatchGateway";

const calls=vi.hoisted(()=>({review:vi.fn(),resolve:vi.fn(),forPage:vi.fn()}));
vi.mock("../../agent/textPatchGateway",async original=>({...await original<typeof import("../../agent/textPatchGateway")>(),textPatchGateway:{forPage:calls.forPage,review:calls.review,resolve:calls.resolve}}));
vi.mock("../NotesClient",async()=>{const {NoteCommandHistory}=await import("./NoteCommandHistory");const client={gateway:{},mutate:vi.fn(),history:new NoteCommandHistory()};return{useNotesClient:()=>client,useNotesVersion:()=>0};});
vi.mock("../../../services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn(),info:vi.fn()}}));
vi.hoisted(()=>{if(!document.doctype)document.insertBefore(document.implementation.createDocumentType("html","",""),document.documentElement);Object.defineProperty(document,"compatMode",{configurable:true,value:"CSS1Compat"});});
const cleanup:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const stop of cleanup.splice(0).reverse())await stop();getAppStorage().clear();vi.clearAllMocks();vi.unstubAllGlobals();});
function body(text:string):NoteDocumentV1{return{schemaVersion:1,pageId:notePageId("recovery-page"),blocks:[{id:noteBlockId("text"),type:"paragraph",props:{},children:[],content:{kind:"inline",items:[{type:"text",text,styles:[]}]}}]};}
function SaveStatus({target}:{readonly target:HTMLElement}){const state=useNoteReviewSaveState("recovery-page");return createPortal(<NoteSaveState pageId="recovery-page" status={state.status} message={state.message}/>,target);}
async function setup(related=false,readOnly=false,mode:"preview"|"browser"="preview"){
 vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);selectAppStorage(localStorage);localStorage.clear();
 selectAppRuntime({mode,storage:localStorage,gateways:{},dispose(){}});calls.forPage.mockResolvedValue(["conversation"]);
 let authority={version:3,saved:body("Current content"),baseline:body("Current content")};
 const commit=vi.fn(async(request)=>{authority={version:authority.version+1,saved:request.saved,baseline:request.baseline};return authority;});
 const session=await HbsSession.open({adapter:createNoteDocumentAdapter(directTextDiff),port:{read:async()=>authority,commit},preservation:{load:()=>undefined,save:()=>{}},requestId:n=>`body:${n}`});
 const issue:TextPatchReview={proposalId:"stale-proposal",generation:1,state:"pending",baselineMatches:false,pages:[{pageId:related?"other-page":body("").pageId,path:"source://notes/other-page",title:related?"Related child":"Current page",kind:"update",before:null,after:null,expectedRevision:2,metadataPending:false,recovery:{currentRevision:3,status:readOnly?"trashed":"active"}}]};
 calls.review.mockResolvedValue(issue);
 calls.resolve.mockImplementation(async(request:TextPatchResolution)=>{calls.review.mockResolvedValue(null);return{requestId:request.requestId,proposalId:request.proposalId,action:request.action,generation:1,review:null,pages:[]};});
 const host=document.createElement("div"),toolbar=document.createElement("div");document.body.append(host,toolbar);const root=createRoot(host);let editor:NoteEditorInstance|undefined;
 await act(async()=>root.render(<><SaveStatus target={toolbar}/><HbsNoteEditor legacyMetadata session={session} diffControlsTarget={toolbar} isReadOnly={readOnly} onEditorReady={value=>{editor=value;}}/></>));
 cleanup.push(async()=>{await act(async()=>root.unmount());host.remove();toolbar.remove();session.dispose();});
 return{session,commit,toolbar,get:()=>authority,external:()=>{authority={version:authority.version+1,saved:body("External update"),baseline:body("External update")};},editor:()=>editor!,open:async()=>{await act(async()=>toolbar.querySelector<HTMLButtonElement>('.note-recovery-menu-host button')!.click());await vi.waitFor(()=>expect(document.querySelector('[role="menu"][aria-label="笔记恢复与诊断"]')).not.toBeNull());},retain:async()=>{
  await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button=>button.textContent==="保留当前内容并解决冲突")!.click());
 }};
}
it("does not call a native proposal inspector or publish a failed metadata contribution in the browser runtime",async()=>{
 const f=await setup(false,false,"browser");
 expect(calls.forPage).not.toHaveBeenCalled();expect(calls.review).not.toHaveBeenCalled();
 expect(getNoteReviewSaveState("recovery-page").status).toBe("saved");
 await act(async()=>{f.session.edit(body("Browser input"));await f.session.save();});
 expect(f.get().saved).toEqual(body("Browser input"));expect(calls.resolve).not.toHaveBeenCalled();
});
it("does not expose another page's stale proposal or navigation in this page's menu",async()=>{
 const f=await setup(true);
 expect(f.toolbar.querySelector('.note-recovery-menu-host')).toBeNull();
 expect(document.body.textContent).not.toContain("Related child");
 expect(document.body.textContent).not.toContain("打开该页面处理");
 expect(getNoteReviewSaveState("recovery-page").status).toBe("saved");
 expect(calls.resolve).not.toHaveBeenCalled();
 expect(calls.review.mock.calls.every(call=>call[3]==="recovery-page")).toBe(true);
});
it("rechecks an inspection failure without saving or resolving a proposal",async()=>{
 const f=await setup(true);
 calls.review.mockRejectedValue(new Error("Query returned no rows"));
 await act(async()=>notifyTextPatchesChanged());
 await f.open();
 calls.review.mockResolvedValue(null);
 await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button=>button.textContent==="重新核对当前页面")!.click());
 expect(f.toolbar.querySelector('.note-recovery-menu-host')).toBeNull();
 expect(getNoteReviewSaveState("recovery-page").status).toBe("saved");
 expect(f.commit).not.toHaveBeenCalled();
 expect(calls.resolve).not.toHaveBeenCalled();
});
it("keeps current through the real HBS and editor history, then reconciles only its stale mirror",async()=>{
 const f=await setup();
 await act(async()=>{f.session.edit(body("My current edits"));f.external();await f.session.syncLatest();});
 expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
 expect(f.session.getSnapshot().content).toEqual(body("My current edits"));
 await f.open();await f.retain();
 expect(notify.error).not.toHaveBeenCalled();
 expect(f.get().saved).toEqual(body("My current edits"));expect(f.get().baseline).toEqual(f.get().saved);
 expect(calls.resolve).toHaveBeenCalledOnce();expect(calls.resolve.mock.calls[0]![0]).toMatchObject({pageId:"recovery-page",action:"retain-current",expectedRevision:f.get().version,document:f.get().saved});
 expect(f.toolbar.querySelector('.note-recovery-menu-host')).toBeNull();
 await act(async()=>{const view=f.editor().prosemirrorView;expect(undo(view.state,view.dispatch)).toBe(true);});
 expect(f.session.getSnapshot().content).toEqual(body("My current edits"));
 expect(f.session.getSnapshot().changes.length+f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
});
it("settles an inactive page's already-confirmed content without rewriting it",async()=>{
 const f=await setup(false,true);await f.open();await f.retain();
 expect(f.commit).not.toHaveBeenCalled();expect(calls.resolve).toHaveBeenCalledOnce();expect(f.get().version).toBe(3);
});
it("waits for the original body receipt before reconciling the stale mirror",async()=>{
 const f=await setup();let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;});
 const commit=f.commit.getMockImplementation()!;f.commit.mockImplementationOnce(async request=>{await gate;return commit(request);});
 await act(async()=>f.session.edit(body("Current chosen input")));
 await f.open();await f.retain();
 expect(f.commit).toHaveBeenCalled();expect(calls.resolve).not.toHaveBeenCalled();expect(notify.error).not.toHaveBeenCalled();
 await act(async()=>{release();await gate;});
 await vi.waitFor(()=>expect(calls.resolve).toHaveBeenCalledOnce());
 expect(f.get().saved).toEqual(body("Current chosen input"));expect(notify.error).not.toHaveBeenCalled();
});
it("retries the exact uncertain recovery request and preserves newer local input",async()=>{
 const f=await setup();calls.resolve.mockRejectedValueOnce(new Error("Transport outcome unknown"));
 await f.open();await f.retain();
 const original=calls.resolve.mock.calls[0]![0];expect(loadNoteRecoveryDraft(notePageId("recovery-page"))?.review?.request).toEqual(original);
 await act(async()=>f.session.edit(body("Newer input while retrying")));
 await f.open();await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button=>button.textContent==="核对并重试原恢复请求")!.click());
 expect(calls.resolve.mock.calls[1]![0]).toEqual(original);expect(f.session.getSnapshot().content).toEqual(body("Newer input while retrying"));
 expect(loadNoteRecoveryDraft(notePageId("recovery-page"))).toBeUndefined();
});
