import {expect,it,vi} from "vitest";
import {AgentInputQueueRegistry} from "@research-assistant/data/agent";
import type {AgentQueueState,AgentQueueCommand} from "@research-assistant/core/agent";
import {recoverRejectedSteer,prioritizeRejectedSteerLease} from "./rejectedSteerQueue";
const scope={conversationId:"11111111-1111-4111-8111-111111111111"},original="22222222-2222-4222-8222-222222222222",rejected="33333333-3333-4333-8333-333333333333";
function fixture(lost:boolean|"resumeInput"=false){
  let queue:AgentQueueState={schemaVersion:1,conversationId:scope.conversationId,revision:0,paused:lost==="resumeInput",preparingInputId:null,inputs:[{inputId:original,text:"ordinary"}]};
  const read=vi.fn(async()=>structuredClone(queue));
  const edit=vi.fn(async(request:typeof scope&AgentQueueCommand)=>{
    if(lost===true||lost===request.edit.type)throw new Error("lost queue receipt");
    const change=request.edit;
    if(change.type==="add")queue={...queue,revision:queue.revision+1,inputs:[...queue.inputs,{inputId:change.inputId,text:change.text,userInput:change.userInput??[]}]};
    if(change.type==="reorder")queue={...queue,revision:queue.revision+1,inputs:change.inputIds.flatMap(id=>queue.inputs.filter(input=>input.inputId===id))};
    return structuredClone(queue);
  });
  const registry=new AgentInputQueueRegistry({read,edit},value=>value.conversationId,()=>crypto.randomUUID());
  return {registry,edit,read,get:()=>queue};
}
it("reattaches the original scoped queue after host detach and prioritizes frozen rejected text",async()=>{
  const f=fixture(),first=f.registry.acquire(scope)!;
  await first.controller.reload();first.release();
  const acknowledge=vi.fn();
  expect(await recoverRejectedSteer(f.registry.acquire(scope),{inputId:rejected,text:"rejected"},acknowledge)).toBe(true);
  expect(f.get().inputs.map(input=>input.text)).toEqual(["rejected","ordinary"]);expect(acknowledge).toHaveBeenCalledOnce();
  const before=f.get().inputs.length;
  expect(await prioritizeRejectedSteerLease(f.registry.acquire(scope),rejected)).toBe(true);
  expect(f.get().inputs).toHaveLength(before);f.registry.dispose();
});
it("leaves a failed queue transfer attached to its receipt without clearing the original draft",async()=>{
  const f=fixture(true),acknowledge=vi.fn();
  expect(await recoverRejectedSteer(f.registry.acquire(scope),{inputId:rejected,text:"keep draft"},acknowledge)).toBe(false);
  expect(acknowledge).not.toHaveBeenCalled();
  const attached=f.registry.acquire(scope)!;
  expect(attached.controller.getSnapshot().uncertain).toBe(true);
  expect(f.get().inputs.map(input=>input.text)).toEqual(["ordinary"]);attached.release();f.registry.dispose();
});
it("keeps an unconfirmed resume on its queue receipt rather than reporting a paused head as dispatched",async()=>{
  const f=fixture("resumeInput");
  expect(await prioritizeRejectedSteerLease(f.registry.acquire(scope),original)).toBe(false);
  expect(f.edit).toHaveBeenCalled();
  const requests=f.edit.mock.calls.map(([request])=>request);
  expect(new Set(requests.map(request=>request.requestId)).size).toBe(1);
  for(const request of requests)expect(request.edit).toEqual({type:"resumeInput",inputId:original});
  expect(f.get().paused).toBe(true);
  expect(f.get().inputs.map(input=>input.text)).toEqual(["ordinary"]);
  const lease=f.registry.acquire(scope)!;
  expect(lease.controller.getSnapshot().uncertain).toBe(true);
  lease.release();f.registry.dispose();
});

it("blocks only a fresh rejected-steer append while preserving already accepted prioritization",async()=>{
  const f=fixture(),acknowledge=vi.fn();
  expect(await recoverRejectedSteer(f.registry.acquire(scope),{inputId:rejected,text:"keep"},acknowledge,()=>true)).toBe(false);
  expect(f.edit).not.toHaveBeenCalled();expect(acknowledge).not.toHaveBeenCalled();
  expect(await prioritizeRejectedSteerLease(f.registry.acquire(scope),original)).toBe(true);
  f.registry.dispose();
});
