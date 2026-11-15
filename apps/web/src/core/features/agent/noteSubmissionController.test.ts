import { afterEach, expect, it, vi } from "vitest";
import type { AgentNotePrepareRequest, AgentNoteSubmissionPreview, AgentStartResponse } from "@research-assistant/core/agent";
import { NoteSubmissionController, noteSubmissionOwnerKey, noteSubmissionOwners, resetNoteSubmissionOwnersForTests } from "./noteSubmissionController";

const conversationId = "a1111111-1111-4111-8111-111111111111";
const pageId = "b1111111-1111-4111-8111-111111111111";
const requestId = "c1111111-1111-4111-8111-111111111111";
const turnId = "d1111111-1111-4111-8111-111111111111";
const contextSnapshotId = "e1111111-1111-4111-8111-111111111111";
function request(): AgentNotePrepareRequest {
  return {conversationId,requestId,text:"Explain saved note",context:{pageId,expectedRevision:3,blockId:null},model:null};
}
function preview(): AgentNoteSubmissionPreview {
  return {schemaVersion:1,conversationId,pageId,requestId,turnId,contextSnapshotId,input:{
    text:"Explain saved note",model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},
    reasoningEffort:null,context:[],
  }};
}
function deferred<T>() {
  let resolve!: (value:T)=>void;
  let reject!: (error:unknown)=>void;
  const promise = new Promise<T>((yes,no)=>{resolve=yes;reject=no;});
  return {promise,resolve,reject};
}
function setup() {
  const scope = {conversationId,pageId};
  const admission: AgentStartResponse = {schemaVersion:1,conversationId,requestId,turnId,alreadyStarted:false,outcome:null};
  const gateway = {prepare:vi.fn().mockResolvedValue(preview()),restore:vi.fn().mockResolvedValue(preview()),start:vi.fn().mockResolvedValue(admission)};
  const turns = {interrupt:vi.fn().mockResolvedValue({schemaVersion:1,conversationId,turnId,cancellationRequested:true})};
  return {scope,gateway,turns,admission,controller:new NoteSubmissionController(scope,gateway,turns)};
}

afterEach(() => resetNoteSubmissionOwnersForTests());

it("copies scope and prepared input and prevents duplicate preparation or implicit start", async()=>{
  const {controller,gateway,scope} = setup();
  scope.pageId = conversationId;
  const input = structuredClone(request());
  const waiting = deferred<AgentNoteSubmissionPreview>();
  gateway.prepare.mockReturnValueOnce(waiting.promise);
  const pending = controller.prepare(input);
  Object.assign(input.context,{expectedRevision:9});
  await controller.prepare(request());
  expect(gateway.prepare).toHaveBeenCalledExactlyOnceWith(request());
  expect(controller.dismiss()).toBe(false);
  waiting.resolve(preview());await pending;
  expect(controller.getSnapshot().phase).toBe("preview");
  expect(gateway.start).not.toHaveBeenCalled();
  expect(controller.dismiss()).toBe(true);
  await expect(controller.prepare({...request(),context:{pageId:conversationId,expectedRevision:3}})).rejects.toThrow("scope mismatch");
});

it("reattaches a switched note view to the same owner while preparation is pending", async()=>{
  const {scope,gateway,turns}=setup();
  const waiting=deferred<AgentNoteSubmissionPreview>();
  gateway.prepare.mockReturnValueOnce(waiting.promise);
  const key=noteSubmissionOwnerKey(scope);
  const first=noteSubmissionOwners.acquire(key,()=>new NoteSubmissionController(scope,gateway,turns));
  const pending=first.controller.prepare(request());
  first.release();
  const second=noteSubmissionOwners.acquire(key,()=>new NoteSubmissionController(scope,gateway,turns));
  expect(second.controller).toBe(first.controller);
  waiting.resolve(preview());await pending;
  expect(second.controller.getSnapshot()).toMatchObject({phase:"preview",request:request()});
  second.release();
});

it("acknowledges the exact draft when an owner starts after its view detached",async()=>{
  const {scope,gateway,turns}=setup();const acknowledge=vi.fn();
  const lease=noteSubmissionOwners.acquire(noteSubmissionOwnerKey(scope),()=>new NoteSubmissionController(scope,gateway,turns));
  await lease.controller.prepare(request(),acknowledge);expect(acknowledge).not.toHaveBeenCalled();
  lease.release();await lease.controller.confirm("deepSeekNonThinking");
  expect(acknowledge).toHaveBeenCalledOnce();expect(lease.controller.getSnapshot().phase).toBe("started");
});

it("does not reuse a dismissed draft acknowledgement for a recovered receipt",async()=>{
  const {controller,gateway}=setup();const acknowledge=vi.fn();
  await controller.prepare(request(),acknowledge);gateway.start.mockRejectedValueOnce({code:"preparation"});
  await controller.confirm("deepSeekNonThinking");expect(controller.dismiss()).toBe(true);
  await controller.recover({conversationId,pageId,requestId});await controller.confirm("deepSeekNonThinking");
  expect(acknowledge).not.toHaveBeenCalled();
});

it("restores or retries the original preparation without refreshing the source", async()=>{
  const {controller,gateway} = setup();
  gateway.prepare.mockRejectedValueOnce(new Error("reply lost"));
  await controller.prepare(request());
  gateway.restore.mockRejectedValueOnce(new Error("still offline"));
  await controller.retryPrepare(true);
  expect(gateway.restore).toHaveBeenCalledExactlyOnceWith({conversationId,requestId,pageId});
  await controller.retryPrepare();
  expect(gateway.prepare.mock.calls).toEqual([[request()],[request()]]);
  expect(gateway.start).not.toHaveBeenCalled();
});

it("keeps an ambiguous admission locked even when a subsequent retry is rejected", async()=>{
  const {controller,gateway} = setup();
  await controller.prepare(request());
  gateway.start.mockRejectedValueOnce(new Error("reply lost"));
  await controller.confirm("deepSeekNonThinking");
  expect(controller.dismiss()).toBe(false);
  gateway.start.mockRejectedValueOnce({code:"preparation",reason:"unbound"});
  await controller.confirm("deepSeekNonThinking");
  expect(controller.getSnapshot().error).toBe("start");
  expect(controller.dismiss()).toBe(false);
  expect(gateway.start.mock.calls).toEqual([[preview(),"deepSeekNonThinking"],[preview(),"deepSeekNonThinking"]]);
  controller.observeOutcome(turnId,"completed");
  expect(controller.dismiss()).toBe(true);
});

it.each(["unsupportedModel","confirmationMismatch","preparation","modelUnavailable"])("allows editing after a first definitive %s rejection", async code=>{
  const {controller,gateway} = setup();
  await controller.prepare(request());
  gateway.start.mockRejectedValueOnce({code});
  await controller.confirm("deepSeekNonThinking");
  expect(controller.getSnapshot().error).toBe("startRejected");
  expect(controller.dismiss()).toBe(true);
});

it("keeps durable outcomes arriving before admission and cancellation replies", async()=>{
  const {controller,gateway,turns,admission} = setup();
  const start = deferred<AgentStartResponse>();
  const stop = deferred<{schemaVersion:1;conversationId:string;turnId:string;cancellationRequested:boolean}>();
  gateway.start.mockReturnValueOnce(start.promise);
  turns.interrupt.mockReturnValueOnce(stop.promise);
  await controller.prepare(request());
  const pending = controller.confirm("deepSeekNonThinking");
  await controller.confirm("deepSeekNonThinking");
  expect(gateway.start).toHaveBeenCalledOnce();
  const stopping = controller.stop();await controller.stop();
  expect(turns.interrupt).toHaveBeenCalledOnce();
  controller.observeOutcome(turnId,"canceled");
  start.resolve(admission);stop.resolve({schemaVersion:1,conversationId,turnId,cancellationRequested:true});
  await Promise.all([pending,stopping]);
  expect(controller.getSnapshot()).toMatchObject({phase:"started",outcome:"canceled",stopRequested:false,stopPending:false});
});

it("treats a stop acknowledgement as pending until history confirms the outcome", async()=>{
  const {controller,turns} = setup();
  await controller.prepare(request());await controller.confirm("deepSeekNonThinking");
  await controller.stop();await controller.stop();
  expect(turns.interrupt).toHaveBeenCalledOnce();
  expect(controller.getSnapshot()).toMatchObject({outcome:null,stopRequested:true});
  expect(controller.dismiss()).toBe(false);
  controller.observeOutcome(pageId,"completed");
  expect(controller.getSnapshot().outcome).toBeNull();
  controller.observeOutcome(turnId,"canceled");
  expect(controller.dismiss()).toBe(true);
});

it("ignores disposed replies without canceling the native owner", async()=>{
  const {controller,gateway,turns} = setup();
  const waiting = deferred<AgentNoteSubmissionPreview>();
  gateway.prepare.mockReturnValueOnce(waiting.promise);
  const listener = vi.fn();controller.subscribe(listener);
  const pending = controller.prepare(request());controller.dispose();
  const previous = controller.getSnapshot();listener.mockClear();
  waiting.resolve(preview());await pending;
  expect(controller.getSnapshot()).toBe(previous);
  expect(listener).not.toHaveBeenCalled();expect(turns.interrupt).not.toHaveBeenCalled();
});

it("restores a discovered receipt without preparing or replacing the current draft",async()=>{
  const {controller,gateway}=setup();
  const identity={conversationId,pageId,requestId};
  await controller.recover(identity);
  expect(gateway.prepare).not.toHaveBeenCalled();expect(gateway.start).not.toHaveBeenCalled();
  expect(gateway.restore).toHaveBeenCalledExactlyOnceWith(identity);
  expect(controller.getSnapshot()).toMatchObject({phase:"preview",request:null,recoveryReceipt:identity,preview:preview()});
  await controller.confirm("deepSeekNonThinking");
  expect(gateway.start).toHaveBeenCalledExactlyOnceWith(preview(),"deepSeekNonThinking");
});

it("retries failed discovery restoration with the same identity and isolates invalid scope",async()=>{
  const {controller,gateway}=setup();
  await expect(controller.recover({conversationId,pageId:conversationId,requestId})).rejects.toThrow("scope mismatch");
  expect(gateway.restore).not.toHaveBeenCalled();
  gateway.restore.mockRejectedValueOnce(new Error("offline"));
  await controller.recover({conversationId,pageId,requestId});
  await controller.retryPrepare();
  expect(gateway.restore.mock.calls).toEqual([[{conversationId,pageId,requestId}],[{conversationId,pageId,requestId}]]);
  expect(gateway.prepare).not.toHaveBeenCalled();expect(controller.getSnapshot().phase).toBe("preview");
});
