// @vitest-environment happy-dom
import {afterEach,beforeEach,expect,it,vi} from "vitest";
import {notePageId} from "@research-assistant/core/notes";
import {createAppRuntime} from "../../../../app/runtime/createAppRuntime";
import {getAppRuntime,selectAppRuntime} from "../../../runtime/appRuntime";
import {getNotesGateway} from "../gateways";
import {AppNoteHbsSession} from "../workspace/appNoteHbsSession";
import {getPendingHbsDocumentCommit} from "../../../services/hbsDocumentGateway";
import {collectNoteRecovery} from "./noteRecoveryExport";
import {diagnosticError} from "./noteRecoveryDiagnostics";

const owners:AppNoteHbsSession[]=[];
beforeEach(()=>{
  localStorage.clear();selectAppRuntime({mode:"browser",storage:localStorage,gateways:{},dispose(){}});
  vi.stubGlobal("fetch",vi.fn(async()=>new Response("%PDF-1.4\nsynthetic")));
});
afterEach(()=>{owners.splice(0).forEach(owner=>owner.dispose());getAppRuntime().dispose();vi.unstubAllGlobals();});
async function open(scenario:string){
  const runtime=await createAppRuntime(`?preview=app&note-recovery=${scenario}`,localStorage);
  runtime.preview!.configure({delayMs:0,nextFailure:null});
  const id=runtime.noteRecovery!.pageId,notes=getNotesGateway();
  const owner=await AppNoteHbsSession.open({loaded:await notes.loadDocument(notePageId(id)),onSnapshot:()=>{},onStatus:()=>{},onDurable:()=>{}});
  owner.start();owners.push(owner);return {id,notes,owner};
}
it("exports unsettled memory, S/B, raw request and binary assets without saving or settling anything",async()=>{
  const f=await open("save-failure"),before=getPendingHbsDocumentCommit("note",f.id);
  const state=structuredClone(f.owner.session.getSnapshot());
  const data=await collectNoteRecovery(f.id,f.notes,{status:"failed",message:"MODEL_REQUEST_UNSETTLED"});
  expect(data.missing).toEqual([]);
  expect(data.assets).toHaveLength(1);expect(atob(data.assets[0]!.base64!)).toContain("Synthetic recovery image");
  expect(data.materials.find(item=>item.source==="outbox")?.value).toEqual(before);
  expect(JSON.stringify(data.materials.find(item=>item.source==="session")?.value)).toContain("81%");
  expect(JSON.stringify(data.materials.find(item=>item.source==="authority")?.value)).toContain("80%");
  expect(getPendingHbsDocumentCommit("note",f.id)).toEqual(before);
  expect(f.owner.session.getSnapshot()).toEqual(state);
  const log=JSON.stringify(data.diagnostics);
  expect(log).not.toContain("模型准确率");expect(log).not.toContain("Synthetic recovery image");
  expect(log).not.toContain("base64");expect(log).not.toContain("故障预览 ·");
  expect(log).toContain(before!.id);expect(log).toContain("MODEL_REQUEST_UNSETTLED");
  expect(log).toContain("MOCK_NOTE_COMMIT_BEFORE_WRITE");
  expect(log).toContain("frames");
});
it("keeps corrupt raw recovery material and explicitly marks failed validation and asset reads",async()=>{
  const f=await open("checkpoint-invalid");
  const notes={...f.notes,readNoteAsset:async()=>{throw new DOMException("storage unavailable","NotReadableError");}};
  const data=await collectNoteRecovery(f.id,notes,{status:"failed"});
  expect(data.materials.find(item=>item.source==="checkpoint")?.value).toMatchObject({content:"broken mock checkpoint"});
  expect(data.missing).toContain("checkpointValidation");
  expect(data.missing.some(source=>source.startsWith("asset:"))).toBe(true);
  expect(data.assets[0]!.error).toMatchObject({name:"NotReadableError"});
  expect(data.materials.find(item=>item.source==="session")?.value).toBeDefined();
  expect(f.owner.session.getSnapshot().preservationError).toContain("MODEL_PRESERVATION_FAILED");
});
it("continues independent reads and diagnostics when the checkpoint cannot be read",async()=>{
  const f=await open("checkpoint-read"),read=vi.spyOn(f.notes,"readNoteAsset");
  const data=await collectNoteRecovery(f.id,f.notes,{status:"failed"},false);
  expect(data.missing).toContain("checkpoint");expect(data.materials.find(item=>item.source==="page")?.value).toBeDefined();
  expect(read).not.toHaveBeenCalled();expect(data.assets).toEqual([]);
  read.mockRestore();
});
it("never copies arbitrary error payloads, private messages or credentials into diagnostic logs",()=>{
  const diagnostic=diagnosticError(Object.assign(new Error("MODEL_PRESERVATION_FAILED: private text Bearer sk-secret https://private.example/a?token=secret"),{code:"MODEL_TEST",requestId:"request-123",status:503,body:"private body",cause:new DOMException("full private cause","QuotaExceededError")}));
  const log=JSON.stringify(diagnostic);
  expect(log).not.toMatch(/private|secret|Bearer|example|body/);
  expect(diagnostic).toMatchObject({code:"MODEL_TEST",requestId:"request-123",httpStatus:503,underlying:{name:"QuotaExceededError"}});
});
it("validates each diagnostic correlation value without rereading its getter",()=>{
  const issue = new Error("timeout");
  const reads = new Map<string, number>();
  for (const key of ["requestId", "turnId", "operationId", "taskId"]) {
    Object.defineProperty(issue, key, {get() {
      const count = (reads.get(key) ?? 0) + 1; reads.set(key, count);
      return count === 1 ? `${key}-safe` : "private body and credential=secret";
    }});
  }
  const diagnostic = diagnosticError(issue);
  expect([...reads.values()]).toEqual([1, 1, 1, 1]);
  expect(diagnostic).toMatchObject({requestId: "requestId-safe", turnId: "turnId-safe",
    operationId: "operationId-safe", taskId: "taskId-safe"});
  expect(JSON.stringify(diagnostic)).not.toMatch(/private|secret|credential/);
});
