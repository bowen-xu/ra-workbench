/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { agentDraftInput, insertAgentReference, type AgentSteeringHandoffIntent } from "@research-assistant/core/agent";
import { AgentPendingSteeringBubbles, pendingSteeringMessages } from "./AgentPendingSteeringBubbles";
import {PendingSteeringOrderStore} from "./PendingSteeringOrderStore";

const intent: AgentSteeringHandoffIntent = {
  operationId: "op", conversationId: "conversation", turnId: "turn", steeringInputId: "queued",
  queueInputId: "queue", expectedRevision: 1, expectedEpoch: "epoch", scope: {kind:"context"},
  queueIndex: 0, queuePaused: false, inputPaused: false, origin: null, text: "队列内容", userInput: [],
};

it("keeps direct and queued input at the tail until the matching turn and item appear", () => {
  const direct = [{conversationId:"conversation",turnId:"turn",inputId:"direct",text:"直接内容"}];
  const props = { conversationId: "conversation", messages: [], direct, handoffs: [intent, intent] };
  expect(pendingSteeringMessages(props).map(message => message.id)).toEqual(["turn/queued", "turn/direct"]);
  expect(pendingSteeringMessages({...props, messages: [{id:"other/direct",turnId:"other",role:"user",content:"另一轮"}]}).map(message => message.id))
    .toEqual(["turn/queued", "turn/direct"]);
  expect(pendingSteeringMessages({...props, messages: [{id:"turn/queued",conversationId:"conversation",turnId:"turn",role:"user",content:"队列内容",streaming:false}]}).map(message => message.id))
    .toEqual(["turn/direct"]);
  expect(pendingSteeringMessages({...props, conversationId:"other"})).toEqual([]);
});

it.each([
  {description:"partial text", patch:{content:"队列"}},
  {description:"incomplete item", patch:{streaming:true}},
  {description:"interrupted item", patch:{interrupted:true}},
  {description:"conflicting role", patch:{role:"assistant" as const}},
  {description:"conflicting conversation", patch:{conversationId:"other"}},
  {description:"conflicting turn", patch:{turnId:"other"}},
])("retains the original pending text for $description", ({patch}) => {
  const message = {id:"turn/queued",conversationId:"conversation",turnId:"turn",role:"user" as const,
    content:"队列内容",streaming:false,...patch};
  expect(pendingSteeringMessages({conversationId:"conversation",messages:[message],handoffs:[intent]}))
    .toEqual([expect.objectContaining({id:"turn/queued",role:"user",content:"队列内容"})]);
});

it("retains the pending text when completion or conversation identity is unavailable", () => {
  const message = {id:"turn/queued",turnId:"turn",role:"user" as const,content:"队列内容"};
  expect(pendingSteeringMessages({conversationId:"conversation",messages:[{...message,conversationId:"conversation"}],handoffs:[intent]})).toHaveLength(1);
  expect(pendingSteeringMessages({conversationId:"conversation",messages:[{...message,streaming:false}],handoffs:[intent]})).toHaveLength(1);
});

it("renders the real user message and preserves pending resource bindings", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const draft = insertAgentReference({text:"使用 ",bindings:[]}, {type:"mention",name:"工具",target:{kind:"tool",serverId:"server",environmentId:"test",revision:"v1"}});
  const direct = [{conversationId:"conversation",turnId:"turn",inputId:"direct",text:draft.text,userInput:agentDraftInput(draft)}];
  try {
    await act(async () => root.render(<AgentPendingSteeringBubbles messages={[]} conversationId="conversation" direct={direct} />));
    expect(host.querySelector('.writing-agent-message.user[data-pending-steering="true"]')).not.toBeNull();
    expect(host.querySelector('[aria-label="工具 工具"]')).not.toBeNull();
    expect(host.textContent).toContain("等待处理");
    await act(async () => root.render(<AgentPendingSteeringBubbles messages={[{id:"turn/direct",conversationId:"conversation",turnId:"turn",role:"user",content:draft.text,streaming:false}]} conversationId="conversation" direct={direct} />));
    expect(host.querySelector('.writing-agent-message.user')).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("keeps alternating direct and queue steers in their observed submission order", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const first={conversationId:"conversation",turnId:"turn",inputId:"first",text:"direct first"};
  const last={conversationId:"conversation",turnId:"turn",inputId:"last",text:"direct last"};
  const render=(direct:readonly typeof first[],handoffs:readonly AgentSteeringHandoffIntent[])=>root.render(
    <AgentPendingSteeringBubbles messages={[]} conversationId="conversation" direct={direct} handoffs={handoffs}/>);
  try {
    await act(async()=>render([first],[]));
    await act(async()=>render([first],[intent]));
    await act(async()=>render([first,last],[intent]));
    expect([...host.querySelectorAll("[data-pending-steering]")].map(item=>item.textContent))
      .toEqual(["direct first等待处理","队列内容等待处理","direct last等待处理"]);
  } finally {await act(async()=>root.unmount());host.remove();}
});

it("retains mixed order across unmounts without letting an older view erase it",async()=>{
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const store=new PendingSteeringOrderStore(),host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const first={conversationId:"conversation",turnId:"turn",inputId:"first",text:"direct first"};
  const render=(direct:readonly typeof first[],handoffs:readonly AgentSteeringHandoffIntent[])=>root.render(
    <AgentPendingSteeringBubbles messages={[]} conversationId="conversation" orderStore={store} direct={direct} handoffs={handoffs}/>);
  try{
    await act(async()=>render([first],[]));await act(async()=>render([first],[intent]));
    await act(async()=>root.render(null));
    // A second, stale view cannot remove the later handoff from the owner.
    store.commit("conversation","conversation",["turn/first"]);
    await act(async()=>render([first],[intent]));
    expect([...host.querySelectorAll("[data-pending-steering]")].map(item=>item.textContent))
      .toEqual(["direct first等待处理","队列内容等待处理"]);
    expect(store.read("conversation","different-conversation")).toEqual([]);
    store.forgetConversation("conversation");expect(store.read("conversation","conversation")).toEqual([]);
  }finally{await act(async()=>root.unmount());host.remove();}
});
