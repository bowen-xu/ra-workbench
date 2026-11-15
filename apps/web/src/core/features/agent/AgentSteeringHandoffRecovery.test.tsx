/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {AgentSteeringHandoffState} from "@research-assistant/data/agent";
import {AgentSteeringHandoffRecovery} from "./AgentSteeringHandoffRecovery";
import {copyText} from "./copyText";
vi.mock("./copyText", () => ({copyText: vi.fn(async () => true)}));

it("shows a readable recovery action while retaining the complete safe diagnostic", async () => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const onCheck = vi.fn(), onRetry = vi.fn();
  const state: AgentSteeringHandoffState = {phase:"error", epoch:null, items:[], intent:{operationId:"request-full-identity",
    conversationId:"conversation",turnId:"turn",steeringInputId:"steer",queueInputId:"queue",expectedRevision:1,expectedEpoch:null,
    scope:{kind:"context"},queueIndex:0,queuePaused:false,inputPaused:false,origin:null,text:"内容",userInput:[]},
    message:"队列引导尚未核对，原消息已保留。 [code=DATA_SQL_OPERATION_CONFLICT；operation=operation-full；cause=handoff_admission_still_present]"};
  try {
    await act(async () => root.render(<AgentSteeringHandoffRecovery available state={state} disabled={false} canRetry={false} queueContainsInput onCheck={onCheck} onRetry={onRetry}/>));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("无法确认引导是否已接收");
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("消息仍显示在队列中");
    expect(host.querySelector("details")).toBeNull();
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click());
    expect(copyText).toHaveBeenCalledWith(expect.stringContaining("code=DATA_SQL_OPERATION_CONFLICT"));
    expect(copyText).toHaveBeenCalledWith(expect.stringContaining("operation=operation-full"));
    expect(copyText).toHaveBeenCalledWith(expect.stringContaining("cause=handoff_admission_still_present"));
    expect(copyText).toHaveBeenCalledWith(expect.stringContaining("request-full-identity"));
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "核对队列引导")!.click());
    expect(onCheck).toHaveBeenCalledOnce(); expect(onRetry).not.toHaveBeenCalled();
    await act(async () => root.render(<AgentSteeringHandoffRecovery available state={{...state,phase:"uncertain"}} disabled={false} canRetry={false} queueContainsInput={false} onCheck={onCheck} onRetry={onRetry}/>));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain("请检查对话和队列中的消息");
    const retry = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "重试原队列引导");
    expect(retry).toBeDefined(); expect(retry?.disabled).toBe(true);
  } finally {await act(async () => root.unmount()); host.remove();}
});
