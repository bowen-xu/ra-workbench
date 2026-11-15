// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import type {AgentQueueState} from "@research-assistant/core/agent";
import type {AgentContextGateway} from "../../../services/agentContextGateway";
import {selectAppStorage} from "../../../runtime/appStorage";
import {WritingDiffDemoHistory} from "../../../../app/previews/agent/WritingDiffDemoHistory";
import {resetAgentConversationProjectionCacheForTests} from "../useAgentConversation";
import {IndependentAgentSession} from "./IndependentAgentSession";

const presentation = vi.hoisted(() => ({status: "recovery" as "sending" | "recovery"}));
vi.mock("../queuePreparingPresentation", async importOriginal => {
  const original = await importOriginal<typeof import("../queuePreparingPresentation")>();
  return {...original, independentQueuePreparingPresentation: (...args: Parameters<typeof original.independentQueuePreparingPresentation>) =>
    presentation.status === "sending" && args[0] ? "sending" : original.independentQueuePreparingPresentation(...args)};
});
vi.mock("../../../services/notifications", () => ({notify:{error:vi.fn(),info:vi.fn(),success:vi.fn(),warning:vi.fn(),dismiss:vi.fn()}}));
vi.mock("./AgentContextProvider", () => ({useAgentContextCandidates:()=>[],useAgentContextSuggestions:()=>[]}));
vi.mock("./AgentContextControl", () => ({AgentContextControl:()=>null}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const conversationId = "11111111-1111-4111-8111-111111111111";
const inputId = "22222222-2222-4222-8222-222222222222";
let close: (() => Promise<void>) | undefined;
afterEach(async () => {await close?.(); close=undefined; resetAgentConversationProjectionCacheForTests(); localStorage.clear();});

it.each(["sending", "recovery"] as const)("presents queue %s inside the production composer and preserves its draft", async status => {
  presentation.status = status;
  selectAppStorage(localStorage);
  localStorage.setItem(`research-assistant:independent-draft:${conversationId}`, "草稿保留");
  const history = new WritingDiffDemoHistory();
  const queue: AgentQueueState = {schemaVersion:1, conversationId, revision:1, paused:false, preparingInputId:inputId,
    inputs:[{inputId, text:"排队消息"}]};
  const read=vi.fn(async()=>queue), recoverQueue=vi.fn(async()=>{throw Error("队首暂时不可用");});
  const gateway: AgentContextGateway = {sessions:history.gateway, queue:{read,edit:async()=>queue}, list:async()=>[],search:async()=>[],
    prepare:async()=>{throw Error("unexpected prepare");},restore:async()=>{throw Error("unexpected restore");},
    start:async()=>{throw Error("unexpected start");},interrupt:async()=>{}, ...(status === "recovery" ? {recoverQueue} : {})};
  const container=document.createElement("div"), root=createRoot(container); document.body.append(container);
  close=async()=>{await act(async()=>root.unmount());container.remove();history.dispose();};
  await act(async()=>root.render(<IndependentAgentSession id={conversationId} title="Synthetic" gateway={gateway} open onOpenChange={()=>{}} headerControls={null} onStarted={()=>{}}/>));
  const text = status === "sending" ? "发送中…" : "消息发送状态待确认";
  await vi.waitFor(()=>expect(container.querySelector('[aria-label="输入框准备状态"]')?.textContent).toBe(text));
  const composer=container.querySelector(".writing-agent-composer")!, input=composer.querySelector<HTMLTextAreaElement>("textarea")!;
  expect(composer.querySelector('[aria-label="输入框准备状态"]')?.textContent).toBe(text);
  expect(container.querySelector('[aria-label="队列消息"]')?.textContent).not.toContain(text);
  expect(input.disabled).toBe(true);
  expect(input.value).toBe("草稿保留");
  if(status === "recovery") {
    await vi.waitFor(()=>expect(recoverQueue).toHaveBeenCalledOnce());
    const button=[...composer.querySelectorAll("button")].find(item=>item.textContent === "处理消息")!;
    await vi.waitFor(()=>expect(button.disabled).toBe(false));
    const priorReads=read.mock.calls.length;
    await act(async()=>button.click());
    await vi.waitFor(()=>expect(read.mock.calls.length).toBeGreaterThan(priorReads));
    await vi.waitFor(()=>expect(recoverQueue).toHaveBeenCalledTimes(2));
    expect(input.value).toBe("草稿保留");
  }
});
