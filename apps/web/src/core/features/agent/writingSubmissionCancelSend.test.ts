import {expect, it, vi} from "vitest";
import type {AgentSubmissionPreview, AgentWritingPrepareRequest, AgentWritingStartRequest, AgentWritingStartResponse} from "@research-assistant/core/agent";
import {selectAppStorage} from "../../runtime/appStorage";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {acquireWritingSubmissionController, releaseWritingSubmissionController, WritingSubmissionController} from "./writingSubmissionController";

const id = "11111111-1111-4111-8111-111111111111";
const request: AgentWritingPrepareRequest = {conversationId:id, requestId:id, text:"Explain", context:{manuscriptId:id, files:[{relativePath:"main.tex",expectedDiskSha256:"hash"}]}};
const preview: AgentSubmissionPreview = {schemaVersion:1, conversationId:id, requestId:id, manuscriptId:id, contextSnapshotId:id, turnId:id,
  input:{text:"Explain", model:{providerId:"deepseek", modelId:"deepseek-v4-pro", revision:null}, reasoningEffort:null,
    context:[{sourceId:"file",kind:"manuscript",resourceId:"main.tex",revision:"hash",anchor:JSON.stringify({manuscriptId:id}),text:"original"}]}};
const admission: AgentWritingStartResponse = {schemaVersion:1, conversationId:id, requestId:id, turnId:id, alreadyStarted:false, outcome:null};
function deferred<T>() {let resolve!:(value:T)=>void, reject!:(reason:unknown)=>void; const promise=new Promise<T>((done, fail)=>{resolve=done;reject=fail;}); return {promise,resolve,reject};}
function setup() {
  const values=new Map<string,string>();
  const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
  selectAppStorage(storage);
  const gateway={prepareWriting:vi.fn(async()=>preview),prepareWritingStatus:vi.fn(async(intent:AgentWritingPrepareRequest)=>({schemaVersion:1 as const,intent,status:{type:"notFound" as const}})),
    restoreWriting:vi.fn(async()=>preview),startWriting:vi.fn(async()=>admission),
    startStatus:vi.fn(async(intent:AgentWritingStartRequest)=>({schemaVersion:1 as const,intent,status:{type:"started" as const,result:{...admission,alreadyStarted:true}}}))};
  const turns={interrupt:vi.fn(async()=>({schemaVersion:1 as const,conversationId:id,turnId:id,cancellationRequested:true}))};
  return {storage,gateway,turns,owner:new WritingSubmissionController(gateway,turns,undefined,{storage})};
}
async function ready(owner:Pick<WritingSubmissionController,"prepare"|"observeWorkspacePreparation">) {await owner.prepare(request);await owner.observeWorkspacePreparation(id,true);}
function persistence(storage:Storage, flush:()=>Promise<void>) {
  registerStoragePersistence(storage,{backend:"IndexedDB",pending:false,error:undefined,resetting:false,flush,
    retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
}

it("fences an in-flight preparation and retires its original preview without starting",async()=>{
  const f=setup(), pending=deferred<AgentSubmissionPreview>(); f.gateway.prepareWriting.mockReturnValueOnce(pending.promise);
  const work=f.owner.prepare(request); await vi.waitFor(()=>expect(f.gateway.prepareWriting).toHaveBeenCalledOnce());
  expect(await f.owner.cancelPendingSend(id)).toBe(true); expect(f.owner.isCancelPending(id)).toBe(true);
  pending.resolve(preview);await work;
  expect(f.gateway.startWriting).not.toHaveBeenCalled();expect(f.owner.getSnapshot().writingPreparation?.phase).toBe("retired");
  expect(f.owner.isCancelPending(id)).toBe(false);
});

it("interrupts the exact turn after an in-flight start is admitted",async()=>{
  const f=setup(),pending=deferred<AgentWritingStartResponse>();await ready(f.owner);f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  const work=f.owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  expect(await f.owner.cancelPendingSend(id)).toBe(true);expect(f.turns.interrupt).not.toHaveBeenCalled();
  pending.resolve(admission);await work;await vi.waitFor(()=>expect(f.turns.interrupt).toHaveBeenCalledOnce());
  expect(f.turns.interrupt).toHaveBeenCalledWith({conversationId:id,turnId:id});
  await vi.waitFor(()=>expect(f.owner.isCancelPending(id)).toBe(false));
});

it("keeps the original stop intent after an unknown start and never starts again while reconciling",async()=>{
  const f=setup(),pending=deferred<AgentWritingStartResponse>();await ready(f.owner);f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  const work=f.owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  await f.owner.cancelPendingSend(id);pending.reject(new Error("lost reply"));await work;
  await vi.waitFor(()=>expect(f.turns.interrupt).toHaveBeenCalledOnce());
  expect(f.gateway.startStatus).toHaveBeenCalledOnce();
  expect(f.gateway.startWriting).toHaveBeenCalledOnce();
});

it("retires an explicitly rejected start rather than retrying or interrupting it",async()=>{
  const f=setup(),pending=deferred<AgentWritingStartResponse>();await ready(f.owner);f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  const work=f.owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  await f.owner.cancelPendingSend(id);pending.reject("modelUnavailable");await work;
  expect(f.gateway.startWriting).toHaveBeenCalledOnce();expect(f.turns.interrupt).not.toHaveBeenCalled();
  expect(f.owner.getSnapshot().writingPreparation?.phase).toBe("retired");expect(f.owner.isCancelPending(id)).toBe(false);
});

it("retains the original owner when its view closes during a pending start",async()=>{
  const f=setup(),key=`stop:${crypto.randomUUID()}`,pending=deferred<AgentWritingStartResponse>();
  const owner=acquireWritingSubmissionController(key,f.gateway,f.turns,undefined,{storage:f.storage});
  await ready(owner);f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  const work=owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  await owner.cancelPendingSend(id);releaseWritingSubmissionController(key,owner);
  const reopened=acquireWritingSubmissionController(key,f.gateway,f.turns,undefined,{storage:f.storage});expect(reopened).toBe(owner);
  pending.resolve(admission);await work;await vi.waitFor(()=>expect(f.turns.interrupt).toHaveBeenCalledOnce());
  releaseWritingSubmissionController(key,reopened);
});

it("restores a persisted stop intent and reconciles an unknown start without resending",async()=>{
  const f=setup(),pending=deferred<AgentWritingStartResponse>();await ready(f.owner);f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  void f.owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  await f.owner.cancelPendingSend(id);f.owner.dispose();
  const recovered=new WritingSubmissionController(f.gateway,f.turns,undefined,{storage:f.storage,scope:{conversationId:id,manuscriptId:id}});
  expect(recovered.isCancelPending(id)).toBe(true);
  await vi.waitFor(()=>expect(f.turns.interrupt).toHaveBeenCalledOnce());
  expect(f.gateway.startStatus).toHaveBeenCalledOnce();
  expect(f.gateway.startWriting).toHaveBeenCalledOnce();pending.resolve(admission);
});

it("waits for a preparation save in flight and retires its original receipt without starting",async()=>{
  const f=setup(),save=deferred<void>();let held=false;
  persistence(f.storage,async()=>{if(!held && f.storage.getItem(`research-assistant:writing-prepare:v1:${id}:${id}`)) {held=true;await save.promise;}});
  const work=f.owner.prepare(request);await vi.waitFor(()=>expect(held).toBe(true));
  await f.owner.cancelPendingSend(id);expect(f.gateway.prepareWriting).not.toHaveBeenCalled();
  save.resolve();await work;
  expect(f.gateway.startWriting).not.toHaveBeenCalled();expect(f.gateway.prepareWriting).toHaveBeenCalledOnce();
  expect(f.owner.getSnapshot().writingPreparation?.phase).toBe("retired");expect(f.owner.isCancelPending()).toBe(false);
});

it("retires a preparation proven not sent after its local save fails",async()=>{
  const f=setup();let fail=true;
  persistence(f.storage,async()=>{if(fail && f.storage.getItem(`research-assistant:writing-prepare:v1:${id}:${id}`)) {fail=false;throw new Error("save failed");}});
  await f.owner.prepare(request);expect(f.gateway.prepareWriting).not.toHaveBeenCalled();
  expect(await f.owner.cancelPendingSend(id)).toBe(true);
  expect(f.gateway.prepareWritingStatus).toHaveBeenCalledOnce();expect(f.gateway.prepareWriting).not.toHaveBeenCalled();
  expect(f.owner.getSnapshot().writingPreparation?.phase).toBe("retired");expect(f.owner.isCancelPending()).toBe(false);
});

it("allows an explicit retry of the same interrupt after its first failure",async()=>{
  const f=setup(),pending=deferred<AgentWritingStartResponse>();await ready(f.owner);
  f.turns.interrupt.mockRejectedValueOnce(new Error("interrupt failed"));f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  const work=f.owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  await f.owner.cancelPendingSend(id);pending.resolve(admission);await work;
  await vi.waitFor(()=>expect(f.owner.getSnapshot().sendStopFailed).toBe(true));
  expect(f.owner.isCancelPending(id)).toBe(true);
  expect(await f.owner.cancelPendingSend(id)).toBe(true);
  await vi.waitFor(()=>expect(f.turns.interrupt).toHaveBeenCalledTimes(2));
  expect(f.gateway.startWriting).toHaveBeenCalledOnce();expect(f.owner.isCancelPending(id)).toBe(false);
});

it("retains an unresolved queue stop hold across reopening and cannot clear a different input",async()=>{
  const f=setup(),scope={conversationId:id,manuscriptId:id},other="22222222-2222-4222-8222-222222222222";
  await f.owner.holdQueueInput(scope,id);
  expect(f.owner.getSnapshot().heldQueueInputId).toBe(id);
  f.owner.dispose();
  const reopened=new WritingSubmissionController(f.gateway,f.turns,undefined,{storage:f.storage,scope});
  expect(reopened.getSnapshot().heldQueueInputId).toBe(id);
  expect(await reopened.clearQueueInputHold(scope,other)).toBe(false);
  expect(reopened.getSnapshot().heldQueueInputId).toBe(id);
  expect(await reopened.clearQueueInputHold(scope,id)).toBe(true);
  expect(reopened.getSnapshot().heldQueueInputId).toBeNull();
  const next=new WritingSubmissionController(f.gateway,f.turns,undefined,{storage:f.storage,scope});
  expect(next.getSnapshot().heldQueueInputId).toBeUndefined();
});

it("marks an unreadable persisted queue stop hold unavailable instead of silently dispatching",()=>{
  const f=setup(),scope={conversationId:id,manuscriptId:id};
  f.storage.setItem(`research-assistant:writing-queue-stop:v1:${id}:${id}`,"{invalid");
  const reopened=new WritingSubmissionController(f.gateway,f.turns,undefined,{storage:f.storage,scope});
  expect(reopened.getSnapshot().queueStopHoldUnavailable).toBe(true);
});

it("does not clear an invalid queue hold when its local recovery still fails",async()=>{
  const f=setup(),scope={conversationId:id,manuscriptId:id};
  f.storage.setItem(`research-assistant:writing-queue-stop:v1:${id}:${id}`,"{invalid");
  const reopened=new WritingSubmissionController(f.gateway,f.turns,undefined,{storage:f.storage,scope});
  expect(await reopened.retryQueueStopHold(scope)).toBe(false);
  expect(reopened.getSnapshot().queueStopHoldUnavailable).toBe(true);
});

it("does not start a fresh submission when the original stop intent is unreadable",async()=>{
  const f=setup(),scope={conversationId:id,manuscriptId:id};
  f.storage.setItem(`research-assistant:writing-send-stop:v1:${id}:${id}`,"{invalid");
  const reopened=new WritingSubmissionController(f.gateway,f.turns,undefined,{storage:f.storage,scope});
  expect(reopened.getSnapshot().sendStopIntentUnavailable).toBe(true);
  await reopened.prepare(request);await reopened.confirm("deepSeekNonThinking");
  expect(f.gateway.prepareWriting).not.toHaveBeenCalled();expect(f.gateway.startWriting).not.toHaveBeenCalled();
  expect(await reopened.retrySendStopIntent(scope)).toBe(false);
});

it("retries the same stop intent when its first durable save fails",async()=>{
  const f=setup(),pending=deferred<AgentWritingStartResponse>(),scope={conversationId:id,manuscriptId:id};
  await ready(f.owner);f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  const work=f.owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  let fail=true;persistence(f.storage,async()=>{if(fail) throw new Error("disk unavailable");});
  await expect(f.owner.cancelPendingSend(id)).rejects.toThrow("disk unavailable");
  expect(f.owner.getSnapshot().sendStopIntentUnavailable).toBe(true);
  expect(f.owner.isCancelPending(id)).toBe(true);fail=false;
  expect(await f.owner.retrySendStopIntent(scope)).toBe(true);
  pending.resolve(admission);await work;await vi.waitFor(()=>expect(f.turns.interrupt).toHaveBeenCalledOnce());
  await vi.waitFor(()=>expect(f.owner.isCancelPending(id)).toBe(false));
  expect(f.gateway.startWriting).toHaveBeenCalledOnce();
});

it("retries local stop-record cleanup without interrupting or starting twice",async()=>{
  const f=setup(),pending=deferred<AgentWritingStartResponse>(),scope={conversationId:id,manuscriptId:id};
  await ready(f.owner);f.gateway.startWriting.mockReturnValueOnce(pending.promise);
  const work=f.owner.confirm("deepSeekNonThinking");await vi.waitFor(()=>expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  const key=`research-assistant:writing-send-stop:v1:${id}:${id}`;let fail=true;
  persistence(f.storage,async()=>{if(fail&&!f.storage.getItem(key)) throw new Error("delete flush");});
  await f.owner.cancelPendingSend(id);pending.resolve(admission);await work;
  await vi.waitFor(()=>expect(f.owner.getSnapshot().sendStopIntentUnavailable).toBe(true));
  expect(f.owner.isCancelPending(id)).toBe(true);expect(f.turns.interrupt).toHaveBeenCalledOnce();
  fail=false;expect(await f.owner.retrySendStopIntent(scope)).toBe(true);
  expect(f.owner.isCancelPending(id)).toBe(false);expect(f.owner.getSnapshot().sendStopIntentUnavailable).toBe(false);
  expect(f.turns.interrupt).toHaveBeenCalledOnce();expect(f.gateway.startWriting).toHaveBeenCalledOnce();
});

it("retains a failed queue hold until it has been saved and explicitly cleared",async()=>{
  const f=setup(),scope={conversationId:id,manuscriptId:id};let fail=true;
  persistence(f.storage,async()=>{if(fail) throw new Error("disk unavailable");});
  await expect(f.owner.holdQueueInput(scope,id)).rejects.toThrow("disk unavailable");
  expect(f.owner.getSnapshot()).toMatchObject({heldQueueInputId:id,queueStopHoldUnavailable:true});
  fail=false;expect(await f.owner.retryQueueStopHold(scope)).toBe(true);
  expect(f.owner.getSnapshot()).toMatchObject({heldQueueInputId:id,queueStopHoldUnavailable:false});
  expect(await f.owner.clearQueueInputHold(scope,id)).toBe(true);
  expect(f.owner.getSnapshot()).toMatchObject({heldQueueInputId:null,queueStopHoldUnavailable:false});
});
