import {afterEach,expect,it,vi} from "vitest";
import type {AgentReadingNotePrepareRequest,AgentReadingNoteSubmissionPreview,AgentStartResponse} from "@research-assistant/core/agent";
import {ReadingNoteSubmissionController,readingNoteSubmissionOwnerKey,readingNoteSubmissionOwners,resetReadingNoteSubmissionOwnersForTests} from "./readingNoteSubmissionController";

const conversationId="a1111111-1111-4111-8111-111111111111",readingNoteId="b1111111-1111-4111-8111-111111111111";
const requestId="c1111111-1111-4111-8111-111111111111",turnId="d1111111-1111-4111-8111-111111111111";
const receipt={conversationId,readingNoteId,requestId};
function request():AgentReadingNotePrepareRequest {return {conversationId,requestId,text:"Explain this reading note",context:{readingNoteId,expectedRevision:3},model:null};}
function preview():AgentReadingNoteSubmissionPreview {return {schemaVersion:1,...receipt,turnId,contextSnapshotId:"e1111111-1111-4111-8111-111111111111",input:{text:request().text,model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[]}};}
function deferred<T>() {
  let resolve!:(value:T)=>void;
  const promise=new Promise<T>(done=>{resolve=done;});return {promise,resolve};
}
function setup() {
  const scope={conversationId,readingNoteId};
  const admission:AgentStartResponse={schemaVersion:1,conversationId,requestId,turnId,alreadyStarted:false,outcome:null};
  const gateway={prepare:vi.fn().mockResolvedValue(preview()),restore:vi.fn().mockResolvedValue(preview()),start:vi.fn().mockResolvedValue(admission)};
  const turns={interrupt:vi.fn().mockResolvedValue({schemaVersion:1,conversationId,turnId,cancellationRequested:true})};
  return {scope,gateway,turns,admission,controller:new ReadingNoteSubmissionController(scope,gateway,turns)};
}
afterEach(()=>resetReadingNoteSubmissionOwnersForTests());
it("freezes the reading scope and revision, rejects page receipts and waits for explicit confirmation",async()=>{
  const {controller,gateway,scope}=setup();scope.readingNoteId=conversationId;
  await expect(controller.recover({conversationId,requestId,pageId:readingNoteId} as never)).rejects.toThrow();
  await expect(controller.prepare({...request(),context:{readingNoteId:conversationId,expectedRevision:3}})).rejects.toThrow("scope mismatch");
  expect(gateway.prepare).not.toHaveBeenCalled();expect(gateway.restore).not.toHaveBeenCalled();
  const input=structuredClone(request()),pending=deferred<AgentReadingNoteSubmissionPreview>();
  gateway.prepare.mockReturnValueOnce(pending.promise);
  const waiting=controller.prepare(input);Object.assign(input.context,{expectedRevision:8});
  await controller.prepare(request());expect(gateway.prepare).toHaveBeenCalledExactlyOnceWith(request());
  expect(controller.dismiss()).toBe(false);pending.resolve(preview());await waiting;
  expect(controller.getSnapshot()).toMatchObject({phase:"preview",request:request()});expect(gateway.start).not.toHaveBeenCalled();
});
it("reattaches a switched reading-note view to the same owner while preparation is pending",async()=>{
  const {scope,gateway,turns}=setup();
  const waiting=deferred<AgentReadingNoteSubmissionPreview>();gateway.prepare.mockReturnValueOnce(waiting.promise);
  const key=readingNoteSubmissionOwnerKey(scope);
  const first=readingNoteSubmissionOwners.acquire(key,()=>new ReadingNoteSubmissionController(scope,gateway,turns));
  const pending=first.controller.prepare(request());first.release();
  const second=readingNoteSubmissionOwners.acquire(key,()=>new ReadingNoteSubmissionController(scope,gateway,turns));
  expect(second.controller).toBe(first.controller);
  waiting.resolve(preview());await pending;
  expect(second.controller.getSnapshot()).toMatchObject({phase:"preview",request:request()});second.release();
});
it("acknowledges the exact reading-note draft when admission succeeds after detach",async()=>{
  const {controller}=setup();const acknowledge=vi.fn();
  await controller.prepare(request(),acknowledge);expect(acknowledge).not.toHaveBeenCalled();
  await controller.confirm("deepSeekNonThinking");expect(acknowledge).toHaveBeenCalledOnce();
});
it("recovers the original reading receipt without preparing again or claiming the current draft",async()=>{
  const {controller,gateway}=setup();gateway.restore.mockRejectedValueOnce(new Error("offline"));
  await controller.recover(receipt);await controller.retryPrepare();
  expect(gateway.restore.mock.calls).toEqual([[receipt],[receipt]]);
  expect(gateway.prepare).not.toHaveBeenCalled();expect(gateway.start).not.toHaveBeenCalled();
  expect(controller.getSnapshot()).toMatchObject({phase:"preview",request:null,recoveryReceipt:receipt});
  await controller.confirm("deepSeekNonThinking");expect(gateway.start).toHaveBeenCalledExactlyOnceWith(preview(),"deepSeekNonThinking");
});
it("does not release ambiguous reading starts after a later rejection or replay another submission",async()=>{
  const {controller,gateway}=setup();await controller.prepare(request());
  gateway.start.mockRejectedValueOnce(new Error("lost reply"));await controller.confirm("deepSeekNonThinking");
  gateway.start.mockRejectedValueOnce({code:"preparation"});await controller.confirm("deepSeekNonThinking");
  expect(controller.getSnapshot().error).toBe("start");expect(controller.dismiss()).toBe(false);
  await controller.recover({...receipt,requestId:conversationId});expect(gateway.restore).not.toHaveBeenCalled();
  controller.observeOutcome(readingNoteId,"completed");expect(controller.dismiss()).toBe(false);
  controller.observeOutcome(turnId,"completed");expect(controller.dismiss()).toBe(true);
  expect(gateway.start.mock.calls).toEqual([[preview(),"deepSeekNonThinking"],[preview(),"deepSeekNonThinking"]]);
});
it("routes stop to the recovered reading turn and preserves a durable result before late replies",async()=>{
  const {controller,gateway,turns,admission}=setup();await controller.recover(receipt);
  const start=deferred<AgentStartResponse>(),stop=deferred<{schemaVersion:1;conversationId:string;turnId:string;cancellationRequested:boolean}>();
  gateway.start.mockReturnValueOnce(start.promise);turns.interrupt.mockReturnValueOnce(stop.promise);
  const starting=controller.confirm("deepSeekNonThinking");await controller.confirm("deepSeekNonThinking");
  const stopping=controller.stop();await controller.stop();expect(turns.interrupt).toHaveBeenCalledExactlyOnceWith({conversationId,turnId});
  controller.observeOutcome(turnId,"canceled");start.resolve(admission);stop.resolve({schemaVersion:1,conversationId,turnId,cancellationRequested:true});
  await Promise.all([starting,stopping]);expect(gateway.start).toHaveBeenCalledOnce();
  expect(controller.getSnapshot()).toMatchObject({phase:"started",outcome:"canceled",stopRequested:false,stopPending:false});
});
it("isolates disposed reading views without canceling the native task or replacing another view",async()=>{
  const old=setup(),next=setup(),waiting=deferred<AgentReadingNoteSubmissionPreview>();
  old.gateway.restore.mockReturnValueOnce(waiting.promise);
  const pending=old.controller.recover(receipt);old.controller.dispose();
  await next.controller.prepare(request());const state=next.controller.getSnapshot();
  waiting.resolve(preview());await pending;
  expect(next.controller.getSnapshot()).toBe(state);expect(old.controller.getSnapshot().phase).toBe("preparing");
  expect(old.turns.interrupt).not.toHaveBeenCalled();expect(next.gateway.start).not.toHaveBeenCalled();
});
