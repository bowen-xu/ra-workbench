// @vitest-environment happy-dom
import {act,useState} from "react";
import {createRoot,type Root} from "react-dom/client";
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {notePageId} from "@research-assistant/core/notes";
import {createAppRuntime} from "../../../app/runtime/createAppRuntime";
import {getAppRuntime,selectAppRuntime} from "../../runtime/appRuntime";
import {getNotesGateway} from "../../features/notes/gateways";
import {NotesProvider} from "../../features/notes/NotesClient";
import {AppNoteHbsSession} from "../../features/notes/workspace/appNoteHbsSession";
import {RetainedNoteEditors} from "../../features/notes/workspace/RetainedNoteEditors";
import type {RetainedNoteSession} from "../../features/notes/workspace/retainedNoteSession";
import {useNoteReviewSaveState} from "../../features/notes/editor/noteTextEditorPorts";
import {getHbsDocumentGateway} from "../../services/hbsDocumentGateway";
const notices=vi.hoisted(()=>({error:vi.fn(),success:vi.fn(),info:vi.fn()}));
vi.mock("../../services/notifications",()=>({notify:notices}));
vi.hoisted(()=>{if(!document.doctype)document.insertBefore(document.implementation.createDocumentType("html","",""),document.documentElement);Object.defineProperty(document,"compatMode",{configurable:true,value:"CSS1Compat"});});
let root:Root|undefined,host:HTMLDivElement;
const owners:AppNoteHbsSession[]=[];
beforeEach(()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);localStorage.clear();selectAppRuntime({mode:"browser",storage:localStorage,gateways:{},dispose(){}});
  vi.stubGlobal("fetch",vi.fn(async()=>new Response("%PDF-1.4 synthetic")));Object.values(notices).forEach(mock=>mock.mockClear());
  host=document.createElement("div");document.body.append(host);
});
afterEach(async()=>{if(root)await act(async()=>root!.unmount());root=undefined;owners.splice(0).forEach(owner=>owner.dispose());getAppRuntime().dispose();host.remove();vi.restoreAllMocks();vi.unstubAllGlobals();});
async function fixture(reopen:boolean){
  const runtime=await createAppRuntime("?preview=app&note-recovery=resolution-failure",localStorage);runtime.preview!.configure({delayMs:0,nextFailure:null});
  const id=notePageId(runtime.noteRecovery!.pageId),notes=getNotesGateway();
  async function open(){const owner=await AppNoteHbsSession.open({loaded:await notes.loadDocument(id),onSnapshot:()=>{},onStatus:()=>{},onDurable:()=>{}});owner.start();owners.push(owner);return owner;}
  let owner=await open();
  if(reopen){owner.session.resolveMany(owner.session.getSnapshot().conflicts.map(change=>change.id),"keep");await vi.waitFor(()=>expect(owner.session.getSnapshot().message).toContain("MOCK_NOTE_RESOLUTION_BEFORE_WRITE"));owner.dispose();owner=await open();}
  const record:RetainedNoteSession={snapshot:await notes.loadDocument(id),editorSessionId:1,saveStatus:"saved",saveMessage:undefined,
    outgoing:[],incoming:[],controller:owner,hbs:owner.session,unregisterText:undefined,recoveryTimer:undefined,invalidated:false,observedRevision:0};
  function Harness(){const [target,setTarget]=useState<HTMLDivElement|null>(null);const saved=useNoteReviewSaveState(id);record.saveStatus=saved.status;record.saveMessage=saved.message;
    return <NotesProvider gateway={notes}><div ref={setTarget}/><RetainedNoteEditors diffControlsTarget={target} sessions={[record]} activeId={id} conflictLocked={false} onChange={()=>{}}/></NotesProvider>;}
  root=createRoot(host);await act(async()=>root!.render(<Harness/>));await act(async()=>new Promise<void>(resolve=>requestAnimationFrame(()=>resolve())));
  await vi.waitFor(()=>expect(host.querySelector('[aria-label="Keep all"]')).not.toBeNull());
  return {owner,runtime,id};
}
async function clickKeep(all:boolean){const button=host.querySelector<HTMLButtonElement>(all?'[aria-label="Keep all"]':'.writing-diff-chunk-keep');expect(button).not.toBeNull();expect(button!.disabled).toBe(false);await act(async()=>button!.click());}
it.each([false,true])("keeps failure retry available after reopen=%s and replays only the original request",async reopen=>{
  const f=await fixture(reopen),commit=vi.spyOn(getHbsDocumentGateway(),"commit"),perform=vi.spyOn(f.owner.session,"perform");
  await clickKeep(true);
  await vi.waitFor(()=>expect(notices.error.mock.lastCall?.[0]).toContain("尚未保存"));
  expect(f.owner.session.getSnapshot().changes.length).toBeGreaterThan(0);
  expect(f.owner.session.getSnapshot().conflicts).toHaveLength(0);
  const original=commit.mock.calls[0]![2];
  await clickKeep(false);
  await vi.waitFor(()=>expect(commit).toHaveBeenCalledTimes(2));
  await vi.waitFor(()=>expect(notices.error).toHaveBeenCalledTimes(2));
  expect(commit.mock.calls[1]![2]).toEqual(original);
  expect(perform).toHaveBeenCalledTimes(reopen?0:1);
  expect(f.owner.session.getSnapshot().queued).toBeLessThanOrEqual(1);
  await f.runtime.noteRecovery!.releaseFault();await clickKeep(true);
  await vi.waitFor(()=>expect(f.owner.session.getSnapshot().changes).toHaveLength(0));
  expect(f.owner.session.getSnapshot().pending).toBe(false);
  expect(commit.mock.calls[2]![2]).toEqual(original);
  expect(notices.success).toHaveBeenCalled();
});

it("reopens an unsettled Keep without locking local Undo, editing or preservation",async()=>{
  const f=await fixture(true),commit=vi.spyOn(getHbsDocumentGateway(),"commit");
  const red=f.owner.session.getSnapshot().baselineProjection;
  expect(f.owner.isEditingBlocked()).toBe(false);
  const undo=host.querySelector<HTMLButtonElement>('.writing-diff-chunk-undo')!;
  expect(undo.disabled).toBe(false);
  await act(async()=>undo.click());
  await f.owner.ensurePreserved();
  expect(f.owner.session.adapter.equal(f.owner.session.getSnapshot().content,red)).toBe(true);
  expect(commit).not.toHaveBeenCalled();
  const checkpoint=await getHbsDocumentGateway().readPreservation("note",f.id) as {content:unknown};
  expect(checkpoint.content).toEqual(red);
  // A later decision cannot replace the old outbox request. On explicit retry
  // it is confirmed first; the existing model then validates the new command.
  await f.runtime.noteRecovery!.releaseFault();
  await act(async()=>f.owner.retry());
  expect(commit.mock.calls[0]![2].saved).not.toEqual(red);
  expect(f.owner.session.adapter.equal(f.owner.session.getSnapshot().content,red)).toBe(true);
  expect(f.owner.requiresRequestRetry()).toBe(false);
});
