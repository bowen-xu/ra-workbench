/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentQueueState, AgentSteeringHandoffIntent } from "@research-assistant/core/agent";
import type { AgentInputQueueState, AgentSteeringHandoffState } from "@research-assistant/data/agent";
import type { WritingAgentMessage } from "../presentation/WritingAgentMessage";
import { WritingAgentTaskStack } from "../presentation/WritingAgentTaskStack";
import { useHiddenHandoffQueueInputs } from "./useHiddenHandoffQueueInputs";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const intent:AgentSteeringHandoffIntent={operationId:"op",conversationId:"conversation",turnId:"turn",steeringInputId:"steer",queueInputId:"one",
  expectedRevision:0,expectedEpoch:"epoch",scope:{kind:"context"},queueIndex:0,queuePaused:false,inputPaused:false,origin:null,text:"First",userInput:[]};
const original:AgentQueueState={schemaVersion:1,conversationId:"conversation",revision:0,paused:false,preparingInputId:null,
  inputs:[{inputId:"one",text:"First"},{inputId:"two",text:"Second"},{inputId:"three",text:"Third"}]};
const waiting:AgentSteeringHandoffState={phase:"waiting",epoch:"epoch",items:[],intent,message:""};
const ready:AgentSteeringHandoffState={phase:"ready",epoch:"epoch",items:[],intent:null,message:""};
const confirmed:WritingAgentMessage={id:"turn/steer",conversationId:"conversation",turnId:"turn",role:"user",content:"First",streaming:false};
function snapshot(queue:AgentQueueState,readSequence:number,loading=false,error:string|null=null):AgentInputQueueState<AgentQueueState>{
  return {queue,loading,error,readSequence,editing:false,uncertain:false};
}

it("hides the duplicate without modifying the queue, then restores a released input only after a fresh read",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);const edit=vi.fn(async()=>true);
  const render=(handoff:AgentSteeringHandoffState,state:AgentInputQueueState<AgentQueueState>,messages:readonly WritingAgentMessage[]=[],identity="scope")=>
    root.render(<View identity={identity} handoff={handoff} state={state} messages={messages} onEdit={edit}/>);
  try{
    await act(async()=>render(waiting,snapshot(original,1)));
    expect(host.textContent).not.toContain("First");expect(host.textContent).toContain("Second");
    expect(original.inputs.map(input=>input.inputId)).toEqual(["one","two","three"]);
    expect(host.querySelector<HTMLButtonElement>('[aria-label="拖动调整第 1 条消息顺序"]')?.disabled).toBe(true);
    await act(async()=>render(ready,snapshot(original,1)));
    expect(host.textContent).not.toContain("First");
    await act(async()=>render(ready,snapshot(original,1,true)));
    expect(host.textContent).not.toContain("First");
    await act(async()=>render(ready,snapshot(original,1,false,"读取队列失败，请重试。")));
    expect(host.textContent).not.toContain("First");
    await act(async()=>render(ready,snapshot(original,2)));
    expect(host.textContent).toContain("First");
    expect(edit).not.toHaveBeenCalled();
    await act(async()=>render(waiting,snapshot(original,2),[],"other-scope"));
    await act(async()=>render(ready,snapshot(original,2),[],"different-scope"));
    expect(host.textContent).toContain("First");
  }finally{await act(async()=>root.unmount());host.remove();}
});

it("does not flash a confirmed item from a stale snapshot and removes an otherwise empty stack",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const only={...original,inputs:[original.inputs[0]!]};
  const render=(handoff:AgentSteeringHandoffState,state:AgentInputQueueState<AgentQueueState>,messages:readonly WritingAgentMessage[]=[])=>
    root.render(<View identity="scope" handoff={handoff} state={state} messages={messages} onEdit={async()=>true}/>);
  try{
    await act(async()=>render(waiting,snapshot(only,1)));
    expect(host.querySelector(".writing-agent-task-stack")).toBeNull();
    await act(async()=>render(ready,snapshot(only,2),[confirmed]));
    expect(host.querySelector(".writing-agent-task-stack")).toBeNull();
    await act(async()=>render(ready,snapshot({...only,revision:1,inputs:[]},3),[confirmed]));
    expect(host.querySelector(".writing-agent-task-stack")).toBeNull();
  }finally{await act(async()=>root.unmount());host.remove();}
});

it("keeps queue rows stable while preparation status is presented in the composer",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const queue={...original,preparingInputId:"one",inputs:[original.inputs[0]!]};
  const render=(preparingStatus:"sending"|"sent"|"recovery")=>root.render(<WritingAgentTaskStack queue={{
    state:snapshot(queue,1),preparingStatus,disabled:false,onEdit:async()=>true,onReload:()=>undefined,onRetry:()=>undefined,
  }}/>);
  try{
    await act(async()=>render("sending"));
    expect(host.textContent).toContain("First");expect(host.textContent).not.toContain("发送中…");
    await act(async()=>render("sent"));expect(host.querySelector(".writing-agent-task-stack")).toBeNull();
    await act(async()=>render("recovery"));expect(host.textContent).toContain("First");
    expect(host.textContent).not.toContain("消息发送状态待确认");
  }finally{await act(async()=>root.unmount());host.remove();}
});

function View({identity,handoff,state,messages,onEdit}:{identity:string;handoff:AgentSteeringHandoffState;state:AgentInputQueueState<AgentQueueState>;messages:readonly WritingAgentMessage[];onEdit:()=>Promise<boolean>}){
  const hidden=useHiddenHandoffQueueInputs(identity,"conversation",handoff,state,messages);
  return <WritingAgentTaskStack queue={{state,hiddenInputIds:hidden,disabled:false,onEdit,onReload:()=>undefined,onRetry:()=>undefined}}/>;
}
