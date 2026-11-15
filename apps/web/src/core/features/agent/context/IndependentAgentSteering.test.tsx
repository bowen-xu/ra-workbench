// @vitest-environment happy-dom
import {act, type ReactNode} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {WritingDiffDemoHistory} from "../../../../app/previews/agent/WritingDiffDemoHistory";
import {IndependentAgentSession} from "./IndependentAgentSession";
import {agentContextGateway,type AgentContextGateway,type AgentContextSteerRequest} from "../../../services/agentContextGateway";
import {AgentSteeringProvider} from "../AgentSteeringProvider";
import type {AgentQueueState,AgentContextQueueEditRequest} from "@research-assistant/core/agent";
import {agentDraftInput,insertAgentReference,type AgentUserInput} from "@research-assistant/core/agent";
import {selectAppStorage} from "../../../runtime/appStorage";
import {resetAgentConversationProjectionCacheForTests} from "../useAgentConversation";

vi.mock("../../../services/notifications",()=>({notify:{error:vi.fn(),info:vi.fn(),success:vi.fn(),warning:vi.fn(),dismiss:vi.fn()}}));
vi.mock("./AgentContextProvider",()=>({useAgentContextCandidates:()=>[],useAgentContextSuggestions:()=>[]}));
vi.mock("./AgentContextControl",()=>({AgentContextControl:()=>null}));
vi.mock("../presentation/WritingAgentPanel",()=>({WritingAgentPanel:(props:{draft:string;onDraftChange:(value:string)=>void;onSteer?:(text:string)=>void;onQueue?:(text:string)=>void;queueDisabled?:boolean;disabledReason?:string;submissionControls:ReactNode;composerAccessory:ReactNode})=><div><input value={props.draft} onChange={event=>props.onDraftChange(event.target.value)}/>{props.onSteer&&<button disabled={!!props.disabledReason} onClick={()=>props.onSteer?.(props.draft)}>steer</button>}{props.onQueue&&<button disabled={props.queueDisabled||!!props.disabledReason} onClick={()=>props.onQueue?.(props.draft)}>queue</button>}{props.submissionControls}{props.composerAccessory}</div>}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;

const conversationId="11111111-1111-4111-8111-111111111111",turnId="22222222-2222-4222-8222-222222222222";
let close:(()=>Promise<void>)|undefined;
afterEach(async()=>{await close?.();close=undefined;vi.restoreAllMocks();resetAgentConversationProjectionCacheForTests();localStorage.clear();});

async function setup(steer:NonNullable<AgentContextGateway["steer"]>, queued=false, owned=false,durable=true,
  queuedContent?:{text:string;userInput:readonly AgentUserInput[]}){
  selectAppStorage(localStorage);
  const history=new WritingDiffDemoHistory();
  const admission={scope:{manuscriptId:"independent",conversationId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId};
  history.start(admission,"Original question");
  const queuedId=crypto.randomUUID();
  let state:AgentQueueState={schemaVersion:1,conversationId,revision:0,paused:false,preparingInputId:null,inputs:queued?[{inputId:queuedId,text:queuedContent?.text??"Queued detail",...(queuedContent?{userInput:queuedContent.userInput}:{})}]:[]};
  const edit=vi.fn(async(request:AgentContextQueueEditRequest)=>{if(request.edit.type==="remove"){const inputId=request.edit.inputId;state={...state,revision:state.revision+1,inputs:state.inputs.filter(input=>input.inputId!==inputId)};}else if(request.edit.type==="add"){const input=request.edit;state={...state,revision:state.revision+1,inputs:[...state.inputs,{inputId:input.inputId,text:input.text,...(input.userInput?{userInput:input.userInput}:{})}]};}return state;});
  type Intent=import("@research-assistant/core/agent").AgentSteeringHandoffIntent;
  const epoch="77777777-7777-4777-8777-777777777777";
  let intent:Intent|null=null;
  let handoffHistoryOverride:"incompleteOrConflicting"|null=null;
  const proof=(family:import("@research-assistant/core/agent").AgentSteeringHandoffFamily)=>({schemaVersion:1 as const,operationId:intent?.operationId??queuedId,conversationId,family,epoch,sqlFence:"0",replayed:false,result:null});
  const handoff={read:vi.fn(async()=>({schemaVersion:1 as const,conversationId,epoch,items:intent?[{intent,proof:proof("agent.steering.handoff.prepare"),history:handoffHistoryOverride??((history.snapshot().events.get(conversationId)??[]).some(event=>event.payload.type==="itemCompleted"&&event.payload.itemId===intent?.steeringInputId)?"confirmed" as const:"unconfirmed" as const)}]:[]})),
    prepare:vi.fn(async(value:Intent)=>{intent=value;return proof("agent.steering.handoff.prepare");}),
    finish:vi.fn(async()=>{state={...state,revision:state.revision+1,inputs:state.inputs.filter(item=>item.inputId!==queuedId)};const result=proof("agent.steering.handoff.finish");intent=null;return result;}),
    release:vi.fn(async()=>{const result=proof("agent.steering.handoff.release");intent=null;return result;}),
    cancelUnregistered:vi.fn(async()=>proof("agent.steering.handoff.cancel")),resolution:vi.fn(async()=>"unconfirmed" as const)};
  const gateway:AgentContextGateway={sessions:history.gateway,steer,...(durable?{steeringHandoff:handoff}:{}),...(owned?{steeringOwner:agentContextGateway.steeringOwner!}:{}),queue:{read:async()=>state,edit},list:async()=>[],search:async()=>[],prepare:async()=>{throw Error("unexpected prepare");},restore:async()=>{throw Error("unexpected restore");},start:async()=>{throw Error("unexpected start");},interrupt:async()=>{}};
  localStorage.setItem(`research-assistant:independent-draft:${conversationId}`,"More detail");
  const container=document.createElement("div"),root=createRoot(container);document.body.append(container);
  close=async()=>{await act(async()=>root.unmount());container.remove();history.dispose();};
  const setVisible=async(visible:boolean)=>{await act(async()=>root.render(<AgentSteeringProvider>{visible&&<IndependentAgentSession id={conversationId} title="Test" gateway={gateway} open onOpenChange={()=>{}} headerControls={null} onStarted={()=>{}}/>}</AgentSteeringProvider>));};
  await setVisible(true);
  await vi.waitFor(()=>expect(container.textContent).toContain("steer"));
  return {container,history,admission,edit,queuedId,setVisible,handoff,readQueue:()=>state,setHandoffHistory:(value:"incompleteOrConflicting")=>{handoffHistoryOverride=value;}};
}

it("persists the queued handoff before steering and completes it from matching history",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,history,admission,edit,queuedId,handoff}=await setup(send,true,false,true);
  const button=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(button.disabled).toBe(false));
  await act(async()=>button.click());
  expect(handoff.prepare).toHaveBeenCalledOnce();expect(send).toHaveBeenCalledOnce();
  expect(container.querySelector('[data-pending-steering]')?.textContent).toContain("Queued detail");
  expect(container.querySelector('.agent-input-queue-body')?.textContent ?? "").not.toContain("Queued detail");
  expect(handoff.prepare.mock.invocationCallOrder[0]).toBeLessThan(send.mock.invocationCallOrder[0]!);
  expect(handoff.prepare.mock.calls[0]![0]).toMatchObject({queueInputId:queuedId,steeringInputId:send.mock.calls[0]![0].inputId,expectedEpoch:epochForTest()});
  expect(handoff.finish).not.toHaveBeenCalled();expect(edit).not.toHaveBeenCalled();
  await act(async()=>history.steer(admission,send.mock.calls[0]![0].inputId,"Queued detail"));
  await vi.waitFor(()=>expect(handoff.finish).toHaveBeenCalledOnce());
  expect(edit).not.toHaveBeenCalled();
});
it("allows a later message to be queued while an accepted handoff waits for history",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,handoff,readQueue}=await setup(send,true);
  const first=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(first.disabled).toBe(false));await act(async()=>first.click());
  const draft=container.querySelector<HTMLInputElement>("input")!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(draft,"Next message");draft.dispatchEvent(new Event("input",{bubbles:true}));});
  const queue=[...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="queue")!;
  await vi.waitFor(()=>expect(queue.disabled).toBe(false));await act(async()=>queue.click());
  await vi.waitFor(()=>expect(readQueue().inputs.map(input=>input.text)).toEqual(["Queued detail","Next message"]));
  expect(send).toHaveBeenCalledOnce();expect(handoff.finish).not.toHaveBeenCalled();
});
it("allows a later direct steer while an accepted handoff waits for history",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,handoff}=await setup(send,true);
  const first=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(first.disabled).toBe(false));await act(async()=>first.click());
  const draft=container.querySelector<HTMLInputElement>("input")!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(draft,"Another steer");draft.dispatchEvent(new Event("input",{bubbles:true}));});
  const steer=[...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="steer")!;
  await vi.waitFor(()=>expect(steer?.disabled).toBe(false));await act(async()=>steer.click());
  await vi.waitFor(()=>expect(send).toHaveBeenCalledTimes(2));
  expect(send.mock.calls[1]![0]).toMatchObject({conversationId,turnId,text:"Another steer"});
  expect(handoff.finish).not.toHaveBeenCalled();
});
it("accepts multiple direct steers before either enters history",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,history,admission}=await setup(send);
  const draft=container.querySelector<HTMLInputElement>("input")!;
  const steer=()=>[...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="steer")!;
  for(const text of ["first steer","second steer"]){
    await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(draft,text);draft.dispatchEvent(new Event("input",{bubbles:true}));});
    await act(async()=>steer().click());
  }
  expect(send.mock.calls.map(([request])=>request.text)).toEqual(["first steer","second steer"]);
  expect(send.mock.calls[0]![0].inputId).not.toBe(send.mock.calls[1]![0].inputId);
  expect([...container.querySelectorAll('[data-pending-steering="true"]')].map(item=>item.textContent)).toEqual([
    expect.stringContaining("first steer"),expect.stringContaining("second steer")]);
  await act(async()=>history.steer(admission,send.mock.calls[0]![0].inputId,"first steer"));
  await vi.waitFor(()=>expect(container.querySelectorAll('[data-pending-steering="true"]')).toHaveLength(1));
  expect(container.querySelector('[data-pending-steering="true"]')?.textContent).toContain("second steer");
});
it("preserves direct-then-queued steering order while both are waiting",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container}=await setup(send,true);
  const direct=[...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="steer")!;
  await act(async()=>direct.click());
  await vi.waitFor(()=>expect(send).toHaveBeenCalledTimes(1));
  const queued=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(queued.disabled).toBe(false));await act(async()=>queued.click());
  await vi.waitFor(()=>expect(send).toHaveBeenCalledTimes(2));
  expect([...container.querySelectorAll('[data-pending-steering="true"]')].map(item=>item.textContent)).toEqual([
    expect.stringContaining("More detail"),expect.stringContaining("Queued detail")]);
});
it("waits for a direct steer receipt before allowing a queued steer to overtake it",async()=>{
  let resolve!:(value:{schemaVersion:1;conversationId:string;turnId:string;inputId:string})=>void;
  const send=vi.fn((request:AgentContextSteerRequest)=>new Promise<{schemaVersion:1;conversationId:string;turnId:string;inputId:string}>(done=>{
    resolve=()=>done({schemaVersion:1,conversationId,turnId,inputId:request.inputId});
  }));
  const {container}=await setup(send,true);
  const direct=[...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="steer")!;
  await act(async()=>direct.click());await vi.waitFor(()=>expect(send).toHaveBeenCalledTimes(1));
  const queued=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  expect(queued.disabled).toBe(true);
  await act(async()=>resolve({schemaVersion:1,conversationId,turnId,inputId:send.mock.calls[0]![0].inputId}));
  await vi.waitFor(()=>expect(queued.disabled).toBe(false));
});
it("keeps later input blocked when a queued handoff has an unknown receipt",async()=>{
  const send=vi.fn(async()=>{throw Error("lost response");});
  const {container}=await setup(send,true);
  const first=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(first.disabled).toBe(false));await act(async()=>first.click());
  await vi.waitFor(()=>expect(container.textContent).toContain("无法确认引导是否已接收"));
  expect([...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="queue")?.disabled).toBe(true);
  expect([...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="steer")).toBeUndefined();
});
it("does not infer acceptance while a queued handoff send is still in flight",async()=>{
  let finish!:(value:{schemaVersion:1;conversationId:string;turnId:string;inputId:string})=>void;
  const send=vi.fn((_request:AgentContextSteerRequest)=>new Promise<{schemaVersion:1;conversationId:string;turnId:string;inputId:string}>(resolve=>{finish=resolve;}));
  const {container}=await setup(send,true);
  const first=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(first.disabled).toBe(false));await act(async()=>first.click());
  await vi.waitFor(()=>expect(send).toHaveBeenCalledOnce());
  expect([...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="queue")?.disabled).toBe(true);
  expect([...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="steer")).toBeUndefined();
  await act(async()=>finish({schemaVersion:1,conversationId,turnId,inputId:send.mock.calls[0]![0].inputId}));
  await vi.waitFor(()=>expect([...container.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent==="queue")?.disabled).toBe(false));
});
it("restores an unconsumed queued steer after the original turn ends",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,history,admission,handoff,edit}=await setup(send,true);
  const button=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(button.disabled).toBe(false));await act(async()=>button.click());
  expect(container.querySelector('.agent-input-queue-body')?.textContent ?? "").not.toContain("Queued detail");
  await act(async()=>history.finish(admission,"canceled"));
  await vi.waitFor(()=>expect(handoff.release).toHaveBeenCalledOnce());
  await vi.waitFor(()=>expect(container.querySelector('.agent-input-queue-body')?.textContent).toContain("Queued detail"));
  expect(container.querySelector('[data-pending-steering]')).toBeNull();expect(edit).not.toHaveBeenCalled();
});
it("retries an unknown queued steer with its original identity",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>{if(send.mock.calls.length===1)throw Error("lost response");return {schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId};});
  const {container,history,admission,handoff}=await setup(send,true);
  const queuedButton=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(queuedButton.disabled).toBe(false));
  await act(async()=>queuedButton.click());
  await vi.waitFor(()=>expect(container.textContent).toContain("无法确认引导是否已接收"));
  const retry=[...container.querySelectorAll("button")].find(button=>button.textContent==="重试原队列引导")!;
  await act(async()=>retry.click());
  expect(send).toHaveBeenCalledTimes(2);expect(send.mock.calls[1]).toEqual(send.mock.calls[0]);expect(handoff.prepare).toHaveBeenCalledOnce();
  await act(async()=>history.steer(admission,send.mock.calls[0]![0].inputId,"Queued detail"));
  await vi.waitFor(()=>expect(handoff.finish).toHaveBeenCalledOnce());
});
it.each(["paper","note","readingNote"] as const)("preserves a queued %s reference across an unknown steer and original retry",async kind=>{
  const resourceId=crypto.randomUUID();
  const resource=kind==="paper"?{kind,resourceId,attachmentId:crypto.randomUUID()}:{kind:"note" as const,resourceId};
  const draft=insertAgentReference({text:"Inspect ",bindings:[]},{type:"mention",name:kind,target:{kind:"file",sourceId:`context:${JSON.stringify(resource)}`,resourceId}});
  const userInput=agentDraftInput(draft);
  const send=vi.fn(async(request:AgentContextSteerRequest)=>{if(send.mock.calls.length===1)throw Error("lost response");return {schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId};});
  const {container,handoff,edit}=await setup(send,true,false,true,{text:draft.text,userInput});
  const button=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(button.disabled).toBe(false));await act(async()=>button.click());
  await vi.waitFor(()=>expect(container.textContent).toContain("无法确认引导是否已接收"));
  expect(handoff.prepare.mock.calls[0]![0]).toMatchObject({scope:{kind:"context"},text:draft.text,userInput});
  expect(send.mock.calls[0]![0]).toMatchObject({text:draft.text,userInput});
  await act(async()=>[...container.querySelectorAll<HTMLButtonElement>("button")].find(value=>value.textContent==="重试原队列引导")!.click());
  expect(send.mock.calls[1]).toEqual(send.mock.calls[0]);expect(edit).not.toHaveBeenCalled();expect(handoff.finish).not.toHaveBeenCalled();
});
it("fences an unregistered uncertain handoff after the turn ends without sending it",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,history,admission,handoff,edit}=await setup(send,true);
  handoff.prepare.mockRejectedValueOnce(Error("lost registration result"));
  const button=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(button.disabled).toBe(false));
  await act(async()=>button.click());expect(send).not.toHaveBeenCalled();
  await act(async()=>history.finish(admission,"canceled"));
  await vi.waitFor(()=>expect(handoff.cancelUnregistered).toHaveBeenCalledOnce());
  expect(send).not.toHaveBeenCalled();expect(edit).not.toHaveBeenCalled();
});
function epochForTest(){return "77777777-7777-4777-8777-777777777777";}

it("keeps a newer retyped draft when the original steer is acknowledged",async()=>{
  let finish!:(value:{schemaVersion:1;conversationId:string;turnId:string;inputId:string})=>void;
  const send=vi.fn((request:AgentContextSteerRequest)=>new Promise<{schemaVersion:1;conversationId:string;turnId:string;inputId:string}>(resolve=>{finish=()=>resolve({schemaVersion:1,conversationId,turnId,inputId:request.inputId});}));
  const {container}=await setup(send);
  await act(async()=>container.querySelector<HTMLButtonElement>("button")!.click());
  expect(send).toHaveBeenCalledOnce();
  const input=container.querySelector<HTMLInputElement>("input")!;
  for(const value of ["Changed","More detail"])await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(input,value);input.dispatchEvent(new Event("input",{bubbles:true}));});
  await act(async()=>finish({schemaVersion:1,conversationId,turnId,inputId:send.mock.calls[0]![0].inputId}));
  expect(input.value).toBe("More detail");
  expect(localStorage.getItem(`research-assistant:independent-draft:${conversationId}`)).toBe("More detail");
  const bubble=container.querySelector('[data-pending-steering="true"]');
  expect(bubble?.textContent).toContain("More detail");
  expect(bubble?.textContent).toContain("等待处理");
});

it("removes queued input only after the matching history item is complete",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,history,admission,edit,queuedId,handoff}=await setup(send,true);
  const draft=container.querySelector<HTMLInputElement>("input")!;
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(draft,"Queued detail");draft.dispatchEvent(new Event("input",{bubbles:true}));});
  const button=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(button.disabled).toBe(false));
  await act(async()=>button.click());expect(send).toHaveBeenCalledOnce();expect(edit).not.toHaveBeenCalled();
  const bubble=container.querySelector('[data-pending-steering="true"]');
  expect(bubble?.textContent).toContain("Queued detail");
  expect(bubble?.textContent).toContain("等待处理");
  expect(container.querySelector('[aria-label="队列引导核对"]')).toBeNull();
  await act(async()=>history.steer(admission,send.mock.calls[0]![0].inputId,"Queued detail"));
  await vi.waitFor(()=>expect(handoff.finish).toHaveBeenCalledOnce());
  expect(container.querySelector('[data-pending-steering="true"]')).toBeNull();
  expect(handoff.prepare.mock.calls[0]![0].queueInputId).toEqual(queuedId);
  expect(localStorage.getItem(`research-assistant:independent-draft:${conversationId}`)).toBe("Queued detail");
});

it("offers recovery when incomplete queued steering remains after the turn ends",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,history,admission,handoff,setHandoffHistory}=await setup(send,true);
  const button=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(button.disabled).toBe(false));
  await act(async()=>button.click());
  setHandoffHistory("incompleteOrConflicting");
  await act(async()=>history.finish(admission,"canceled"));
  await vi.waitFor(()=>expect(container.querySelector('[aria-label="队列引导核对"]')).not.toBeNull());
  const recheck=[...container.querySelectorAll("button")].find(item=>item.textContent==="核对队列引导");
  expect(recheck).toBeDefined();
  const before=handoff.read.mock.calls.length;
  await act(async()=>recheck!.click());
  await vi.waitFor(()=>expect(handoff.read.mock.calls.length).toBeGreaterThan(before));
  expect(send).toHaveBeenCalledOnce();
  expect(handoff.finish).not.toHaveBeenCalled();
});

it("acknowledges a later direct draft when queued history arrives before its reply",async()=>{
  let finish!:(value:{schemaVersion:1;conversationId:string;turnId:string;inputId:string})=>void;
  const send=vi.fn((request:AgentContextSteerRequest)=>send.mock.calls.length===1
    ? new Promise<{schemaVersion:1;conversationId:string;turnId:string;inputId:string}>(resolve=>{finish=resolve;})
    : Promise.resolve({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const {container,history,admission,handoff}=await setup(send,true);
  const queuedButton=container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
  await vi.waitFor(()=>expect(queuedButton.disabled).toBe(false));
  await act(async()=>queuedButton.click());
  await act(async()=>history.steer(admission,send.mock.calls[0]![0].inputId,"Queued detail"));
  await vi.waitFor(()=>expect(handoff.finish).toHaveBeenCalledOnce());
  const direct=[...container.querySelectorAll("button")].find(button=>button.textContent==="steer")!;
  await act(async()=>direct.click());
  await vi.waitFor(()=>expect(send).toHaveBeenCalledTimes(2));
  expect(localStorage.getItem(`research-assistant:independent-draft:${conversationId}`)).toBeNull();
  await act(async()=>finish({schemaVersion:1,conversationId,turnId,inputId:send.mock.calls[0]![0].inputId}));
});

it("retains an uncertain request and retries the same identity",async()=>{
  const send=vi.fn(async(request:AgentContextSteerRequest)=>{if(send.mock.calls.length===1)throw Error("lost receipt");return {schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId};});
  const {container,history,admission}=await setup(send);
  await act(async()=>container.querySelector<HTMLButtonElement>("button")!.click());
  expect(container.textContent).toContain("接收结果未确认");
  const retry=[...container.querySelectorAll("button")].find(button=>button.textContent==="重试原补充说明")!;
  await act(async()=>retry.click());expect(send).toHaveBeenCalledTimes(2);expect(send.mock.calls[1]).toEqual(send.mock.calls[0]);
  await act(async()=>history.steer(admission,send.mock.calls[0]![0].inputId,"More detail"));
  await vi.waitFor(()=>expect(container.querySelector('[data-pending-steering="true"]')).toBeNull());
});

it("reattaches an unknown request after switching away from the conversation",async()=>{
  let fail!:(reason:unknown)=>void;
  const send=vi.spyOn(agentContextGateway,"steer").mockImplementationOnce(()=>new Promise((_resolve,reject)=>{fail=reject;})).mockImplementation(async request=>({schemaVersion:1,conversationId,turnId,inputId:request.inputId}));
  const {container,setVisible}=await setup(send,false,true);
  await act(async()=>container.querySelector<HTMLButtonElement>("button")!.click());
  await setVisible(false);await act(async()=>fail(Error("lost receipt")));await setVisible(true);
  await vi.waitFor(()=>expect(container.textContent).toContain("接收结果未确认"));
  const retry=[...container.querySelectorAll("button")].find(button=>button.textContent==="重试原补充说明")!;
  await act(async()=>retry.click());expect(send.mock.calls[1]).toEqual(send.mock.calls[0]);
  expect(localStorage.getItem(`research-assistant:independent-draft:${conversationId}`)).toBeNull();
});
