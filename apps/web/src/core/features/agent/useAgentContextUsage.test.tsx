/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentConversationProjection, AgentTurnProjection, AgentAttemptProjection, AgentModelIdentity } from "@research-assistant/core/agent";
import { useAgentContextUsage } from "./useAgentContextUsage";
const model = {providerId:"fixture",modelId:"model",revision:null};
function projection(inputTokens: number, seq = 1): AgentConversationProjection {
  const attempt:AgentAttemptProjection = {requestedModel:model,actualModel:model,responseId:"response",outcome:"completed",inputTokens,outputTokens:54,error:null,retryable:null,failureCategory:null};
  const turn:AgentTurnProjection = {model,contextSnapshotId:"context",phase:"runningModel",outcome:null,itemOrder:[],items:new Map(),attemptOrder:["attempt"],attempts:new Map([["attempt",attempt]]),inputTokens:900000,outputTokens:54};
  return {conversationId:"conversation",seq,activeTurn:"turn",turnOrder:["turn"],turns:new Map([["turn",turn]])};
}
it("refreshes finished sample measurements while ignoring text deltas and clears on conversation change", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const reader = vi.fn(async (_conversation: string, _model: AgentModelIdentity, _signal:AbortSignal) => ({inputTokens:reader.mock.calls.length*1000,contextWindowTokens:128000}));
  let visible:ReturnType<typeof useAgentContextUsage>;
  function Probe({value,selected}:{value:AgentConversationProjection;selected?:AgentModelIdentity}) { visible=useAgentContextUsage(value,reader,selected); return null; }
  const root=createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Probe value={projection(1000)} />));
    expect(visible?.inputTokens).toBe(1000);
    await act(async () => root.render(<Probe value={projection(1000,30)} />));
    expect(reader).toHaveBeenCalledTimes(1);
    await act(async () => root.render(<Probe value={projection(2000,31)} />));
    expect(reader).toHaveBeenCalledTimes(2);
    expect(visible?.inputTokens).toBe(2000);
    await act(async () => root.render(<Probe value={projection(2000)} selected={{...model,modelId:"other-model"}} />));
    expect(visible).toBeUndefined();
    expect(reader).toHaveBeenCalledTimes(2);
    const changed={...projection(2000),conversationId:"other"};
    await act(async () => root.render(<Probe value={changed} />));
    expect(reader.mock.calls.at(-1)?.[0]).toBe("other");
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals(); }
});

it("hides a removed turn's measurement while reading the surviving sample in the same model", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let finishRead!: (usage: {inputTokens:number;contextWindowTokens:number}) => void;
  const reader = vi.fn()
    .mockResolvedValueOnce({inputTokens:8000,contextWindowTokens:128000})
    .mockImplementationOnce(() => new Promise(resolve => { finishRead = resolve; }));
  let visible:ReturnType<typeof useAgentContextUsage>;
  function Probe({value}:{value:AgentConversationProjection}) { visible=useAgentContextUsage(value,reader); return null; }
  const root=createRoot(document.createElement("div"));
  const surviving=projection(1000,20);
  const removed={...projection(8000,30),activeTurn:"removed",turnOrder:["removed"],turns:new Map([["removed",projection(8000).turns.get("turn")!]])};
  try {
    await act(async () => root.render(<Probe value={removed} />));
    expect(visible?.inputTokens).toBe(8000);
    await act(async () => root.render(<Probe value={surviving} />));
    expect(reader).toHaveBeenCalledTimes(2);
    expect(visible).toBeUndefined();
    await act(async () => root.render(<Probe value={{...surviving,seq:21}} />));
    expect(reader).toHaveBeenCalledTimes(2);
    expect(visible).toBeUndefined();
    await act(async () => finishRead({inputTokens:1000,contextWindowTokens:128000}));
    expect(visible?.inputTokens).toBe(1000);
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals(); }
});

it("invalidates the old window immediately when the same attempt installs a compaction, without delta polling", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let finishRead!: (usage: {inputTokens:number;contextWindowTokens:number}) => void;
  const reader = vi.fn()
    .mockResolvedValueOnce({inputTokens:8000,contextWindowTokens:128000})
    .mockImplementationOnce(() => new Promise(resolve => { finishRead = resolve; }));
  let visible:ReturnType<typeof useAgentContextUsage>;
  function Probe({value}:{value:AgentConversationProjection}) { visible=useAgentContextUsage(value,reader); return null; }
  function withCompaction(completed: boolean, seq: number): AgentConversationProjection {
    const value = projection(1000, seq);
    const turn = value.turns.get("turn")!;
    return {...value,turns:new Map([["turn",{...turn,itemOrder:["operation"],items:new Map([["operation",{
      kind:{type:"contextCompaction" as const},text:"",completed,interrupted:false,
    }]])}]])};
  }
  const root=createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Probe value={withCompaction(false,10)} />));
    expect(visible?.inputTokens).toBe(8000);
    await act(async () => root.render(<Probe value={withCompaction(false,11)} />));
    expect(reader).toHaveBeenCalledTimes(1);
    await act(async () => root.render(<Probe value={withCompaction(true,12)} />));
    expect(reader).toHaveBeenCalledTimes(2);
    expect(visible).toBeUndefined();
    await act(async () => root.render(<Probe value={withCompaction(true,13)} />));
    expect(reader).toHaveBeenCalledTimes(2);
    expect(visible).toBeUndefined();
    await act(async () => finishRead({inputTokens:1200,contextWindowTokens:128000}));
    expect(visible?.inputTokens).toBe(1200);
    await act(async () => root.render(<Probe value={withCompaction(true,20)} />));
    expect(reader).toHaveBeenCalledTimes(2);
  } finally { await act(async () => root.unmount()); vi.unstubAllGlobals(); }
});
