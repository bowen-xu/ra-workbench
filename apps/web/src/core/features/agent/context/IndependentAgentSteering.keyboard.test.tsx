// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import type {AgentContextQueueEditRequest,AgentQueueState} from "@research-assistant/core/agent";
import {WritingDiffDemoHistory} from "../../../../app/previews/agent/WritingDiffDemoHistory";
import {IndependentAgentSession} from "./IndependentAgentSession";
import type {AgentContextGateway,AgentContextSteerRequest} from "../../../services/agentContextGateway";
import {selectAppStorage} from "../../../runtime/appStorage";
import {resetAgentConversationProjectionCacheForTests} from "../useAgentConversation";

vi.mock("../../../services/notifications",()=>({notify:{error:vi.fn(),info:vi.fn(),success:vi.fn(),warning:vi.fn(),dismiss:vi.fn()}}));
vi.mock("./AgentContextProvider",()=>({useAgentContextCandidates:()=>[],useAgentContextSuggestions:()=>[]}));
vi.mock("./AgentContextControl",()=>({AgentContextControl:()=>null}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
const conversationId="11111111-1111-4111-8111-111111111111",turnId="22222222-2222-4222-8222-222222222222";
let close:(()=>Promise<void>)|undefined;
afterEach(async()=>{await close?.();close=undefined;resetAgentConversationProjectionCacheForTests();localStorage.clear();});

it("routes Enter to the queue and Ctrl+Enter to the active turn in the production composer",async()=>{
  selectAppStorage(localStorage);
  const history=new WritingDiffDemoHistory();
  history.start({scope:{manuscriptId:"independent",conversationId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId},"First question");
  let queue:AgentQueueState={schemaVersion:1,conversationId,revision:0,paused:false,preparingInputId:null,inputs:[]};
  const edit=vi.fn(async(request:AgentContextQueueEditRequest)=>{if(request.edit.type==="add")queue={...queue,revision:queue.revision+1,inputs:[...queue.inputs,{inputId:request.edit.inputId,text:request.edit.text}]};return queue;});
  const steer=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const steeringHandoff:NonNullable<AgentContextGateway["steeringHandoff"]>={read:async()=>({schemaVersion:1,conversationId,epoch:"33333333-3333-4333-8333-333333333333",items:[]}),prepare:async()=>{throw Error("unexpected handoff");},finish:async()=>{throw Error("unexpected handoff");},release:async()=>{throw Error("unexpected handoff");},cancelUnregistered:async()=>{throw Error("unexpected handoff");},resolution:async()=>"unconfirmed"};
  const gateway:AgentContextGateway={sessions:history.gateway,steer,steeringHandoff,queue:{read:async()=>queue,edit},list:async()=>[],search:async()=>[],prepare:async()=>{throw Error("unexpected prepare");},restore:async()=>{throw Error("unexpected restore");},start:async()=>{throw Error("unexpected start");},interrupt:async()=>{}};
  const container=document.createElement("div"),root=createRoot(container);document.body.append(container);
  close=async()=>{await act(async()=>root.unmount());container.remove();history.dispose();};
  await act(async()=>root.render(<IndependentAgentSession id={conversationId} title="Synthetic" gateway={gateway} open onOpenChange={()=>{}} headerControls={null} onStarted={()=>{}}/>));
  const input=await vi.waitFor(()=>{const node=container.querySelector<HTMLTextAreaElement>("textarea");expect(node).not.toBeNull();return node!;});
  await vi.waitFor(()=>expect(input.disabled).toBe(false));
  await vi.waitFor(()=>expect(container.textContent).not.toContain("正在核对队列引导"));
  const type=async(text:string)=>act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,text);input.dispatchEvent(new Event("input",{bubbles:true}));});
  await type("Queued question");
  await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
  await vi.waitFor(()=>expect(edit).toHaveBeenCalledOnce());
  expect(edit.mock.calls[0]![0].edit).toMatchObject({type:"add",text:"Queued question"});expect(steer).not.toHaveBeenCalled();
  await type("Clarify now");
  await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",ctrlKey:true,bubbles:true,cancelable:true})));
  await vi.waitFor(()=>expect(steer).toHaveBeenCalledOnce());
  expect(steer.mock.calls[0]![0]).toMatchObject({conversationId,turnId,text:"Clarify now"});expect(edit).toHaveBeenCalledOnce();
});

it("does not turn an ordinary queued Enter into a steer when handoff recovery is unavailable",async()=>{
  selectAppStorage(localStorage);
  const history=new WritingDiffDemoHistory();
  history.start({scope:{manuscriptId:"independent",conversationId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId},"First question");
  const queue:AgentQueueState={schemaVersion:1,conversationId,revision:0,paused:false,preparingInputId:null,inputs:[]};
  const edit=vi.fn(async()=>queue),steer=vi.fn(async(request:AgentContextSteerRequest)=>({schemaVersion:1 as const,conversationId,turnId,inputId:request.inputId}));
  const gateway:AgentContextGateway={sessions:history.gateway,steer,queue:{read:async()=>queue,edit},list:async()=>[],search:async()=>[],prepare:async()=>{throw Error("unexpected prepare");},restore:async()=>{throw Error("unexpected restore");},start:async()=>{throw Error("unexpected start");},interrupt:async()=>{}};
  const container=document.createElement("div"),root=createRoot(container);document.body.append(container);
  close=async()=>{await act(async()=>root.unmount());container.remove();history.dispose();};
  await act(async()=>root.render(<IndependentAgentSession id={conversationId} title="Synthetic" gateway={gateway} open onOpenChange={()=>{}} headerControls={null} onStarted={()=>{}}/>));
  const input=await vi.waitFor(()=>{const node=container.querySelector<HTMLTextAreaElement>("textarea");expect(node).not.toBeNull();return node!;});
  await vi.waitFor(()=>expect(container.textContent).toContain("暂时无法检查引导状态"));
  await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"keep this draft");input.dispatchEvent(new Event("input",{bubbles:true}));});
  await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
  expect(edit).not.toHaveBeenCalled();expect(steer).not.toHaveBeenCalled();expect(input.value).toBe("keep this draft");
});
