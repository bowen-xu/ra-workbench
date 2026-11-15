/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingAgentTranscript } from "./WritingAgentTranscript";
import type { WritingAgentMessage } from "./WritingAgentMessage";
import { DeveloperModeProvider } from "../../settings/DeveloperMode";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent, type AgentEventPayload } from "@research-assistant/core/agent";
import { projectConversationMessages } from "../projectConversationMessages";

let container: HTMLDivElement;
let root: Root;
const startedAt = Date.parse("2026-10-09T00:00:00Z");
const prompt: WritingAgentMessage = { id: "turn/request", turnId: "turn", turnFinished: false,
  turnStartedAtMs: startedAt, createdAt: new Date(startedAt + 100).toISOString(), role: "user", content: "修改稿件" };
const tool: WritingAgentMessage = { ...prompt, id: "turn/tool", role: "assistant", kind: "tool-call",
  toolName: "read_source", content: "{}", streaming: true };
const progress: WritingAgentMessage = { ...prompt, id: "turn/progress", role: "assistant",
  phase: "commentary", content: "已读取稿件，继续修改。", streaming: false };
const reply: WritingAgentMessage = { ...prompt, id: "turn/final", role: "assistant",
  createdAt: new Date(startedAt + 3000).toISOString(), content: "修改完成。", streaming: false };
const completed = (messages: readonly WritingAgentMessage[]) => messages.map(message => ({ ...message,
  turnFinished: true, turnFinishedAtMs: startedAt + 7_800, streaming: false }));
const process = () => container.querySelector<HTMLDetailsElement>(".writing-agent-turn-process");
function expectExpandedProcess(): HTMLDetailsElement {
  const details = process();
  expect(details).not.toBeNull();
  expect(details?.dataset.flat).toBe("true");
  expect(details?.open).toBe(true);
  expect(details?.querySelector<HTMLElement>(":scope > summary")?.hidden).toBe(true);
  return details!;
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers(); vi.setSystemTime(startedAt);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); });
async function render(messages: readonly WritingAgentMessage[], running = true, searchMessageId?: string) {
  await act(async () => root.render(<WritingAgentTranscript messages={messages} isRunning={running}
    thinkingStartedAtMs={startedAt} searchMessageId={searchMessageId}
    turnAttachment={turnId => <div data-test-attachment={turnId}>文件变更</div>}
    messageActions={id => <button data-test-branch={id}>创建分支</button>} />));
}

it("keeps observation feedback inside its owning turn before the response footer", async () => {
  await act(async () => root.render(<WritingAgentTranscript messages={completed([prompt, reply])} isRunning={false}
    turnFeedback={id => id === "turn" ? <div role="alert">运行状态需要核对</div> : null} />));
  const turn = container.querySelector('[data-agent-turn-id="turn"]');
  expect(turn?.querySelector('[role="alert"]')?.textContent).toBe("运行状态需要核对");
});

it("shows saved child creation inspection in the production transcript after interruption", async () => {
  const conversationId="11111111-1111-4111-8111-111111111111", turnId="22222222-2222-4222-8222-222222222222";
  const callId="33333333-3333-4333-8333-333333333333", itemId="44444444-4444-4444-8444-444444444444";
  const payloads: AgentEventPayload[] = [
    {type:"turnStarted",turnId,contextSnapshotId:itemId,model:{providerId:"test",modelId:"test",revision:null}},
    {type:"itemAccepted",turnId,itemId,kind:{type:"toolCall",callId,name:"research_spawn_subtask",providerCallId:"original"},text:"{}",phase:null,source:null},
    {type:"childCreationPrepared",turnId,callId,version:1,childId:"55555555-5555-4555-8555-555555555555",operationId:"66666666-6666-4666-8666-666666666666",binding:{kind:"manuscript",resourceId:"original"}},
    {type:"turnFailed",turnId,category:"recovery",error:`tool_outcome_unknown: ${JSON.stringify({code:"CHILD_CREATION_UNCONFIRMED",conversationId,turnId,requestId:"66666666-6666-4666-8666-666666666666",resultId:"55555555-5555-4555-8555-555555555555",phase:"creation",safeCause:"unknown"})}`},
    {type:"turnFinished",turnId,outcome:"failed"},
  ];
  const projection = applyAgentEvents(createAgentConversationProjection(conversationId), payloads.map((payload,index)=>decodeAgentEvent({schemaVersion:1,conversationId,requestId:turnId,seq:index+1,timestampMs:1,payload})));
  await render(projectConversationMessages(projection), false);
  expect(container.textContent).toContain("核对子任务创建");
  expect(container.textContent).toContain("子任务创建结果尚待核对");
  expect(container.textContent).not.toContain("子任务的原创建操作需要核对");
  expect(container.textContent).not.toContain("Agent 请求失败");
  expect(container.querySelectorAll('[role="alert"]')).toHaveLength(1);
  expect(container.querySelectorAll('[aria-label="复制诊断信息"]')).toHaveLength(1);
  expect(container.querySelector('[aria-label="复制消息"]')).toBeNull();
  expect(container.querySelector('[role="alert"] [data-test-branch]')).toBeNull();
  const writeText = vi.fn(async (_text: string) => undefined);
  const clipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {configurable: true, value: {writeText}});
  try {
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click());
    expect(writeText.mock.lastCall?.[0]).toContain("CHILD_CREATION_UNCONFIRMED");
    expect(writeText.mock.lastCall?.[0]).toContain("operation=66666666-6666-4666-8666-666666666666");
  } finally {
    if (clipboard) Object.defineProperty(navigator, "clipboard", clipboard);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
});

it("shows the cleanup stage only on its matching child and retains the other creation recovery", async () => {
  const conversationId = "11111111-1111-4111-8111-111111111111", turnId = "22222222-2222-4222-8222-222222222222";
  const firstCall = "33333333-3333-4333-8333-333333333333", firstItem = "44444444-4444-4444-8444-444444444444";
  const firstChild = "55555555-5555-4555-8555-555555555555", firstOperation = "66666666-6666-4666-8666-666666666666";
  const secondCall = "77777777-7777-4777-8777-777777777777", secondItem = "88888888-8888-4888-8888-888888888888";
  const secondChild = "99999999-9999-4999-8999-999999999999", secondOperation = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  const submission = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
  const payloads: AgentEventPayload[] = [
    {type: "turnStarted", turnId, contextSnapshotId: firstItem, model: {providerId: "test", modelId: "test", revision: null}},
    {type: "itemAccepted", turnId, itemId: firstItem, kind: {type: "toolCall", callId: firstCall, name: "research_spawn_subtask", providerCallId: "first"}, text: "{}", phase: null, source: null},
    {type: "childCreationPrepared", turnId, callId: firstCall, version: 1, childId: firstChild, operationId: firstOperation, binding: {kind: "manuscript", resourceId: "original"}},
    {type: "itemAccepted", turnId, itemId: secondItem, kind: {type: "toolCall", callId: secondCall, name: "research_spawn_subtask", providerCallId: "second"}, text: "{}", phase: null, source: null},
    {type: "childCreationPrepared", turnId, callId: secondCall, version: 2, childId: secondChild, operationId: secondOperation, submissionRequestId: submission, binding: {kind: "manuscript", resourceId: "original"}},
    {type: "turnFailed", turnId, category: "recovery", error: `tool_outcome_unknown: ${JSON.stringify({code: "CHILD_CLEANUP_UNCONFIRMED", conversationId, turnId, requestId: submission, resultId: secondChild, phase: "cleanup", safeCause: "unknown"})}`},
    {type: "turnFinished", turnId, outcome: "failed"},
  ];
  const projection = applyAgentEvents(createAgentConversationProjection(conversationId), payloads.map((payload, index) => decodeAgentEvent({schemaVersion: 1, conversationId, requestId: turnId, seq: index + 1, timestampMs: 1, payload})));
  const messages = projectConversationMessages(projection);
  expect(messages.filter(message => message.failure)).toHaveLength(1);
  await render(messages, false);
  expect(container.textContent).toContain("子任务清理结果尚待核对");
  expect(container.textContent).toContain("子任务创建结果尚待核对");
  expect(container.textContent).toContain("核对子任务状态");
  expect(container.textContent).toContain("核对子任务创建");
  expect(container.textContent).not.toContain("Agent 请求失败");
  expect(container.querySelectorAll('[aria-label="复制诊断信息"]')).toHaveLength(2);
});

it.each([[59, "59秒"], [192, "3分12秒"], [194200, "2日5时56分40秒"]] as const)(
  "uses the same compact elapsed label for live and historical turns (%s seconds)", async (seconds, label) => {
    await render(completed([prompt, tool, reply]).map(message => ({...message, turnFinishedAtMs: startedAt + seconds * 1000})), false);
    expect(process()?.querySelector("summary")?.textContent).toBe(`已完成 · ${label}`);
    vi.setSystemTime(startedAt + seconds * 1000);
    await render([prompt, tool]);
    expect(process()?.querySelector("summary")?.textContent).toBe(`工作中 · ${label}`);
  },
);

it("keeps developer inspection in the response footer and hides it until the turn finishes", async () => {
  const key = "research-assistant.developer-mode";
  const previous = Object.getOwnPropertyDescriptor(window, "localStorage");
  Object.defineProperty(window, "localStorage", { configurable: true,
    value: { getItem: (name: string) => name === key ? "true" : null } });
  async function renderDeveloper(messages: readonly WritingAgentMessage[], running: boolean) {
    await act(async () => root.render(<DeveloperModeProvider><WritingAgentTranscript
      messages={messages} isRunning={running} thinkingStartedAtMs={startedAt}
    /></DeveloperModeProvider>));
  }
  try {
    for (const streaming of [true, false]) {
      // Reconnection can pause text updates without finishing the response.
      await renderDeveloper([prompt, { ...tool, streaming: false }, { ...reply, content: "已准备 1", streaming }], true);
      const partial = container.querySelector('[data-agent-message-id="turn/final"]')!;
      expect(partial.textContent).toContain("已准备 1");
      expect(partial.querySelector('.writing-agent-message-meta')).toBeNull();
      expect(container.querySelector('[data-agent-message-id="turn/request"] .agent-message-data-trigger')).not.toBeNull();
      expect(container.querySelector('[data-agent-activity-record] .agent-message-data-trigger')).not.toBeNull();
    }
    await renderDeveloper(completed([prompt, tool, progress, reply]), false);
    const footer = container.querySelector('[data-agent-message-id="turn/final"] .writing-agent-message-meta')!;
    expect(footer.querySelector('time')).not.toBeNull();
    expect(footer.querySelector('[aria-label="查看原始数据"]')).not.toBeNull();
    expect(footer.querySelector('[aria-label="复制消息"]')).not.toBeNull();
    expect(container.querySelector('[data-agent-message-id="turn/progress"] .writing-agent-message-meta')).toBeNull();
  } finally {
    if (previous) Object.defineProperty(window, "localStorage", previous);
    else Reflect.deleteProperty(window, "localStorage");
  }
});

it("keeps actions persistent only for the latest response and places its time after the buttons", async () => {
  await render(completed([prompt, reply]), false);
  const response = container.querySelector('[data-agent-message-id="turn/final"]')!;
  const footer = response.querySelector('.writing-agent-message-meta')!;
  expect(response.getAttribute("data-latest-response")).toBe("true");
  expect(footer.lastElementChild?.tagName).toBe("TIME");
  expect(footer.firstElementChild?.className).toBe("writing-agent-message-actions");
  expect(footer.querySelector('[aria-label="复制消息"]')).not.toBeNull();
  expect(footer.querySelector('[data-test-branch="turn/final"]')).not.toBeNull();
  expect(container.querySelector('[data-agent-message-id="turn/request"] .writing-agent-message-meta')?.firstElementChild?.tagName).toBe("TIME");

  const nextPrompt = { ...prompt, id: "next/request", turnId: "next" };
  await render([...completed([prompt, reply]), nextPrompt]);
  expect(container.querySelector('[data-latest-response="true"]')).toBeNull();
  const nextReply = { ...reply, id: "next/final", turnId: "next" };
  await render(completed([prompt, reply, nextPrompt, nextReply]), false);
  expect(container.querySelectorAll('[data-latest-response="true"]')).toHaveLength(1);
  expect(container.querySelector('[data-latest-response="true"]')?.getAttribute("data-agent-message-id")).toBe("next/final");
});

it("keeps compaction status outside the completed turn and before subsequent turns", async () => {
  const activity = { id: "compact/1", conversationId: "conversation", afterMessageId: reply.id, state: "completed" as const };
  await act(async () => root.render(<WritingAgentTranscript isRunning={false}
    messages={completed([prompt, reply])} commandActivities={[activity]} />));
  const row = container.querySelector('.agent-compaction-activity')!;
  expect(row.textContent).toBe("上下文已压缩");
  expect(row.closest('.writing-agent-turn, .writing-agent-message')).toBeNull();
  const next = { ...prompt, id: "next/request", turnId: "next" };
  await act(async () => root.render(<WritingAgentTranscript isRunning
    messages={[...completed([prompt, reply]), next]} commandActivities={[activity]} />));
  expect(container.querySelector('.agent-compaction-activity')).toBe(row);
  expect(row.compareDocumentPosition(container.querySelector('[data-agent-message-id="next/request"]')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(row.querySelector('time, button, details')).toBeNull();
});

it("places the working timer at the process opening and the thinking indicator at its tail", async () => {
  await render([prompt]);
  expect(container.querySelector('[data-agent-activity="thinking"]')?.textContent).toBe("思考中");
  expect(process()).toBeNull();
  expect(container.querySelector('[data-agent-activity="thinking"] [data-shimmer-active="true"]')).not.toBeNull();
  expect(container.querySelector('[data-agent-activity="thinking"] svg')).toBeNull();
  await act(async () => vi.advanceTimersByTime(2_000));
  await render([prompt, tool]);
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  expect(container.querySelector('[data-agent-activity="working"]')?.textContent).toBe("工作中 · 2秒");
  expect(process()?.open).toBe(true);
  expect(process()?.querySelector(":scope > summary [data-shimmer-active=\"true\"]")).toBeNull();
  expect(process()?.querySelector(".writing-agent-turn-status svg")).toBeNull();
  expect(process()?.querySelector(".writing-agent-turn-chevron")).toBeNull();
  expect(process()?.querySelector(":scope > summary")?.className).toBe("writing-agent-turn-header");
  expect(process()!.querySelector("summary")!.compareDocumentPosition(container.querySelector('[data-agent-activity-record]')!)
    & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  await render([prompt, { ...tool, streaming: false }, progress]);
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  expect(process()?.open).toBe(true);
});

it("does not introduce a working disclosure for a single reply", async () => {
  await render([prompt]);
  await render([prompt, { ...reply, phase: "finalAnswer", streaming: true }]);
  expect(process()).toBeNull();
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  expect(container.querySelector('[data-agent-message-id="turn/final"]')?.textContent).toContain("修改完成。");
  await render(completed([prompt, reply]), false);
  expect(process()).toBeNull();
  expect(container.querySelector(".writing-agent-turn-header")).toBeNull();
  expect(container.querySelector('[data-test-branch="turn/final"]')).not.toBeNull();
});

it("keeps every final-answer item outside the process and never promotes terminal commentary", async () => {
  await render(completed([prompt, { ...reply, phase: "finalAnswer" },
    { ...reply, id: "turn/final-2", phase: "finalAnswer", content: "第二段结论。" }, progress]), false);
  for (const id of ["turn/final", "turn/final-2"]) {
    expect(container.querySelector(`[data-agent-message-id="${id}"]`)?.closest("details")).toBeNull();
  }
  expect(process()?.querySelector('[data-agent-message-id="turn/progress"]')).not.toBeNull();
  expect(container.querySelectorAll("[data-test-attachment]")).toHaveLength(1);
  expect(container.querySelector('[data-test-branch="turn/final-2"]')).not.toBeNull();
});

it("retains a commentary-only completed turn inside its process", async () => {
  await render(completed([prompt, progress]), false);
  expect(process()?.querySelector('[data-agent-message-id="turn/progress"]')).not.toBeNull();
});

it("keeps a later terminal failure visible beside already accepted final answers", async () => {
  await render(completed([prompt, { ...reply, phase: "finalAnswer" },
    { ...reply, id: "turn/failure", failure: true, turnOutcome: "failed", content: "工具结果尚未确认。" }]), false);
  const failure = container.querySelector('[data-agent-message-id="turn/failure"]');
  expect(failure).not.toBeNull();
  expect(failure?.closest("details")).toBeNull();
});

it("retains a final-only failure status, partial answer and reconnect history after reopening", async () => {
  const stopped: readonly WritingAgentMessage[] = completed([
    { ...prompt, reconnectHistory: [{ attemptId: "retry-1", reason: "连接中断，已用尽重试次数。" }] },
    { ...reply, phase: "finalAnswer", content: "已接收的部分回答（未完成）" },
    { ...reply, id: "turn/failure", failure: true, content: "模拟故障：与模型服务的网络连接中断。" },
  ]).map(message => ({ ...message, turnOutcome: "failed" }));
  for (const reopen of [false, true]) {
    if (reopen) await act(async () => { root.render(null); vi.advanceTimersByTime(90_000); });
    await render(stopped, false);
    expect(process()?.querySelector(".writing-agent-turn-status")?.textContent).toBe("未完成 · 7秒");
    expect(process()?.open).toBe(false);
    expect(container.querySelector('[data-agent-message-id="turn/final"]')?.textContent).toContain("已接收的部分回答（未完成）");
    expect(container.querySelectorAll('[data-agent-message-id="turn/failure"][role="alert"]')).toHaveLength(1);
    expect(container.querySelectorAll('[data-agent-activity="reconnect-history"]')).toHaveLength(1);
    expect(container.querySelector('[data-agent-activity="reconnect-history"]')?.textContent).toContain("连接中断，已用尽重试次数。");
    expect(container.querySelector('[data-agent-activity="thinking"], [data-agent-activity="reconnecting"]')).toBeNull();
  }
});

it("returns to thinking only after all tools settle while awaiting further feedback", async () => {
  const call = { ...tool, toolCallId: "read" };
  const other = { ...tool, id: "turn/other", toolCallId: "search", streaming: false };
  const result: WritingAgentMessage = { ...call, id: "turn/result", kind: "tool-result", streaming: false };
  const otherResult: WritingAgentMessage = { ...other, id: "turn/other-result", kind: "tool-result" };
  await render([prompt, { ...call, streaming: false }]);
  expect(container.querySelector('[data-agent-activity="working"]')).not.toBeNull();
  await render([prompt, { ...call, streaming: false }, other, result]);
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  await act(async () => vi.advanceTimersByTime(4_000));
  await render([prompt, { ...call, streaming: false }, other, result, otherResult]);
  expect(container.querySelector('[data-agent-activity="thinking"]')?.textContent).toBe("思考中");
  expect(process()?.querySelector(':scope > summary [data-shimmer-active="true"]')).toBeNull();
  expect(process()?.querySelector(":scope > summary")?.textContent).toBe("工作中 · 4秒");
  const thinking = container.querySelector('[data-agent-activity="thinking"]')!;
  expect(thinking.closest("summary")).toBeNull();
  expect(thinking).toBe(process()?.querySelector(".writing-agent-turn-content")?.lastElementChild);
  expect(thinking.querySelector('[data-shimmer-active="true"]')).not.toBeNull();
  expect(container.querySelectorAll('[data-agent-activity="working"]')).toHaveLength(1);
  expect(container.querySelectorAll('[data-agent-activity="thinking"]')).toHaveLength(1);
  expect(process()?.open).toBe(true);
  const settled = [prompt, { ...call, streaming: false }, other, result, otherResult];
  await render([...settled, { ...progress, streaming: true }]);
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  expect(container.querySelector('[data-agent-activity="working"]')?.textContent).toBe("工作中 · 4秒");
  await render([...settled, progress]);
  expect(container.querySelector('[data-agent-activity="thinking"]')?.textContent).toBe("思考中");
  await render([...settled, progress, { ...reply, phase: "finalAnswer" }]);
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  await render(completed([...settled, progress, reply]), false);
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  expect(process()?.querySelector(':scope > summary')?.textContent).toBe("已完成 · 7秒");
});

it("pairs receipts with their calls across interleaved commentary", async () => {
  const first = { ...tool, toolCallId: "first", toolName: "research_list_sources", streaming: false };
  const second = { ...first, id: "turn/second", toolCallId: "second" };
  const receipt = (call: WritingAgentMessage) => ({ ...call, id: `${call.id}/result`, kind: "tool-result" as const, content: "{}" });
  await render([prompt, first, progress, second, receipt(first)]);
  let groups = container.querySelectorAll('[data-agent-activity-record]');
  expect(groups).toHaveLength(2);
  expect(groups[0]?.textContent).toContain("已列出文件");
  expect(groups[1]?.textContent).toContain("正在列出文件");
  expect(groups[0]!.compareDocumentPosition(container.querySelector('[data-agent-message-id="turn/progress"]')!)
    & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  await render(completed([prompt, first, progress, second, receipt(first), receipt(second), reply]), false);
  groups = container.querySelectorAll('[data-agent-activity-record]');
  expect(groups).toHaveLength(2);
  expect([...groups].every(group => group.textContent?.includes("已列出文件"))).toBe(true);
  expect(process()?.textContent).not.toContain("列出文件中断");
});

it("prevents mouse and keyboard folding while running, then collapses and freezes the completed process", async () => {
  await render([prompt, tool, progress]);
  const summary = process()!.querySelector("summary")!;
  expect(summary.getAttribute("aria-disabled")).toBe("true");
  expect(summary.tabIndex).toBe(-1);
  expect(summary.getAttribute("aria-label")).toBe("工作中 · 0秒");
  const click = new MouseEvent("click", { bubbles: true, cancelable: true });
  await act(async () => { summary.dispatchEvent(click); });
  expect(click.defaultPrevented).toBe(true);
  for (const key of ["Enter", " "]) {
    const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
    await act(async () => { summary.dispatchEvent(event); });
    expect(event.defaultPrevented).toBe(true);
  }
  await act(async () => { process()!.open = false; process()!.dispatchEvent(new Event("toggle")); });
  expect(process()?.open).toBe(true);
  await render([prompt, tool, progress, { ...reply, streaming: true }]);
  expect(process()?.open).toBe(true);
  await act(async () => { process()!.open = true; process()!.dispatchEvent(new Event("toggle")); });
  await render(completed([prompt, tool, progress, reply]), false);
  expect(process()?.open).toBe(false);
  expect(process()?.querySelector(":scope > summary")?.textContent).toBe("已完成 · 7秒");
  expect(process()?.querySelector(".writing-agent-turn-chevron")).not.toBeNull();
  expect(process()?.querySelector("summary")?.hasAttribute("aria-disabled")).toBe(false);
  expect(process()?.querySelector('[data-shimmer-active="true"]')).toBeNull();
  expect(process()?.querySelector('[data-agent-message-id="turn/progress"]')).not.toBeNull();
  expect(process()?.querySelector('[data-agent-activity-record]')).not.toBeNull();
  const final = container.querySelector('[data-agent-message-id="turn/final"]')!;
  expect(final.closest("details")).toBeNull();
  expect(final.querySelector('[data-test-attachment]')).not.toBeNull();
  expect(final.querySelector('[data-test-branch="turn/final"]')).not.toBeNull();
  expect(final.querySelector('[data-agent-message-actions]')).not.toBeNull();
  await act(async () => vi.advanceTimersByTime(10_000));
  expect(process()?.querySelector(":scope > summary")?.textContent).toBe("已完成 · 7秒");
  await act(async () => { process()!.open = true; process()!.dispatchEvent(new Event("toggle")); });
  expect(process()?.open).toBe(true);
});

it("separates text-only intermediate replies from the last reply and keeps each new turn independent", async () => {
  await render([prompt, progress, reply]);
  expect(process()?.open).toBe(true);
  await render(completed([prompt, progress, reply]), false);
  expect(process()?.querySelector('[data-agent-message-id="turn/progress"]')).not.toBeNull();
  expect(process()?.querySelector('[data-agent-message-id="turn/final"]')).toBeNull();
  const next = { ...prompt, id: "next/request", turnId: "next", turnStartedAtMs: startedAt + 10_000 };
  await render([...completed([prompt, progress, reply]), next, { ...tool, id: "next/tool", turnId: "next", turnStartedAtMs: next.turnStartedAtMs }]);
  const disclosures = container.querySelectorAll<HTMLDetailsElement>(".writing-agent-turn-process");
  expect(disclosures[0]!.open).toBe(false); expect(disclosures[1]!.open).toBe(true);
});

it("keeps completed attachments in their own turn during and after subsequent replies", async () => {
  const first = completed([prompt, reply]);
  await render(first, false);
  const card = container.querySelector('[data-test-attachment="turn"]');
  const next = { ...prompt, id: "next/request", turnId: "next" };
  await render([...first, next]);
  expect(container.querySelector('[data-test-attachment="turn"]')).toBe(card);
  expect(card?.closest("article")?.getAttribute("data-agent-message-id")).toBe(reply.id);
  expect(card!.compareDocumentPosition(container.querySelector('[data-agent-message-id="next/request"]')!)
    & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const nextReply = { ...reply, id: "next/reply", turnId: "next", streaming: true };
  await render([...first, next, nextReply]);
  expect(container.querySelector('[data-test-attachment="next"]')).toBeNull();
  expect(container.querySelector('[data-test-attachment="turn"]')).toBe(card);
  await render([...first, ...completed([next, nextReply])], false);
  expect(container.querySelector('[data-test-attachment="turn"]')).toBe(card);
  expect(container.querySelector('[data-test-attachment="next"]')?.closest("article")?.getAttribute("data-agent-message-id")).toBe(nextReply.id);
});

it("loads completed history with its recorded duration and reveals process messages for search", async () => {
  await render(completed([prompt, tool, progress, reply]), false);
  expect(process()?.open).toBe(false);
  expect(process()?.querySelector("summary")?.textContent).toBe("已完成 · 7秒");
  await render(completed([prompt, tool, progress, reply]), false, progress.id);
  expect(process()?.open).toBe(true);
  await act(async () => { process()!.open = false; process()!.dispatchEvent(new Event("toggle")); });
  await render(completed([prompt, tool, progress, reply]), false, progress.id);
  expect(process()?.open).toBe(false);
});

it("keeps canceled tool-only rounds visible with a frozen status and no disclosure", async () => {
  await render([prompt, tool]);
  await act(async () => vi.advanceTimersByTime(3_100));
  await render([prompt, tool].map(message => ({ ...message, turnFinished: true, turnOutcome: "canceled",
    turnFinishedAtMs: startedAt + 3_100, streaming: false })), false);
  const status = container.querySelector('[data-agent-activity="aborted"]')!;
  expectExpandedProcess();
  expect(status.textContent).toBe("已中止 · 3秒");
  expect(status.querySelector("svg")).toBeNull();
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  await act(async () => vi.advanceTimersByTime(5_000));
  expect(status.textContent).toBe("已中止 · 3秒");
  expect(container.querySelector('[data-agent-activity-record] [data-tool-status="interrupted"]')).not.toBeNull();
  expect(container.querySelector('[data-agent-activity-record]')?.textContent).toContain("读取资料");
  await act(async () => status.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  expectExpandedProcess();
  expect(container.querySelector('[data-agent-activity-record]')).not.toBeNull();
});

it.each(["canceled", "interrupted"] as const)("retains %s status before any feedback and when loading history", async outcome => {
  await render([prompt]);
  await act(async () => vi.advanceTimersByTime(4_200));
  const stopped = [{ ...prompt, turnFinished: true, turnOutcome: outcome, turnFinishedAtMs: startedAt + 4_200 }];
  await render(stopped, false);
  expect(container.querySelector('[data-agent-activity="aborted"]')?.textContent).toBe("已中止 · 4秒");
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  const process = expectExpandedProcess();
  await act(async () => { process.open = false; process.dispatchEvent(new Event("toggle")); });
  expect(process.open).toBe(true);
  expect(container.querySelector('[data-shimmer-active="true"]')).toBeNull();
  await act(async () => { root.render(null); vi.advanceTimersByTime(90_000); });
  await render(stopped, false);
  expect(container.querySelector('[data-agent-activity="aborted"]')?.textContent).toBe("已中止 · 4秒");
});

it("shows a retained receipt warning with the interrupted transcript after remount", async () => {
  const conversationId="11111111-1111-4111-8111-111111111111",turnId="22222222-2222-4222-8222-222222222222";
  const callId="33333333-3333-4333-8333-333333333333",itemId="44444444-4444-4444-8444-444444444444";
  const payloads: AgentEventPayload[] = [
    {type:"turnStarted",turnId,contextSnapshotId:itemId,model:{providerId:"fixture",modelId:"fixture",revision:null}},
    {type:"itemAccepted",turnId,itemId,kind:{type:"toolCall",callId,name:"read"},text:"{}",phase:null,source:null},
    {type:"toolReceiptPending",turnId,callId},
    {type:"turnFinished",turnId,outcome:"interrupted"},
  ];
  const history=payloads.map((payload,index)=>decodeAgentEvent({schemaVersion:1,conversationId,requestId:turnId,seq:index+1,timestampMs:startedAt+index,payload}));
  const messages=()=>projectConversationMessages(applyAgentEvents(createAgentConversationProjection(conversationId),history));
  await render(messages(),false);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain("回执尚未确认");
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(callId);
  expect(container.querySelector('[data-agent-activity="aborted"]')?.textContent).toContain("已中止");
  await act(async()=>root.render(null));await render(messages(),false);
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(callId);
});

it("keeps durable per-message time across clock changes, rerender and cold history remount", async () => {
  const conversationId="11111111-1111-4111-8111-111111111111",turnId="22222222-2222-4222-8222-222222222222";
  const user="33333333-3333-4333-8333-333333333333",assistant="44444444-4444-4444-8444-444444444444";
  const entries:readonly [number,AgentEventPayload][]=[
    [0,{type:"turnStarted",turnId,contextSnapshotId:user,model:{providerId:"fixture",modelId:"synthetic",revision:null}}],
    [100,{type:"itemStarted",turnId,itemId:user,kind:{type:"userMessage"}}],
    [200,{type:"textDelta",turnId,itemId:user,text:"Durable original user message"}],
    [300,{type:"itemCompleted",turnId,itemId:user}],
    [3100,{type:"itemAccepted",turnId,itemId:assistant,kind:{type:"assistantMessage"},text:"Durable original answer",phase:"finalAnswer",source:null}],
    [7800,{type:"turnFinished",turnId,outcome:"completed"}],
  ];
  const history=entries.map(([offset,payload],index)=>decodeAgentEvent({schemaVersion:1,conversationId,requestId:turnId,
    seq:index+1,timestampMs:startedAt+offset,payload}));
  const messages=()=>projectConversationMessages(applyAgentEvents(createAgentConversationProjection(conversationId),history));
  const times=()=>[...container.querySelectorAll("time")].map(time=>({dateTime:time.dateTime,label:time.textContent}));
  await render(messages(),false);
  const original=times();
  expect(original.map(time=>time.dateTime)).toEqual([new Date(startedAt+100).toISOString(),new Date(startedAt+3100).toISOString()]);
  vi.setSystemTime(startedAt+600_000);await render(messages(),false);expect(times()).toEqual(original);
  await act(async()=>root.render(null));vi.setSystemTime(startedAt+1_200_000);
  await render(messages(),false);expect(times()).toEqual(original);
  expect(container.querySelectorAll('[data-latest-response="true"]')).toHaveLength(1);
});

it.each([undefined,"invalid-date",""])("omits unknown message time while retaining content and actions (%s)",async createdAt=>{
  const messages=completed([prompt,reply]).map(message=>{const next={...message};
    if(createdAt===undefined)delete next.createdAt;else next.createdAt=createdAt;return next;});
  vi.setSystemTime(startedAt+600_000);await render(messages,false);
  expect(container.querySelectorAll("time")).toHaveLength(0);
  for(const message of messages){
    const article=container.querySelector(`[data-agent-message-id="${message.id}"]`);
    expect(article?.textContent).toContain(message.content);
    expect(article?.querySelector('[aria-label="复制消息"]')).not.toBeNull();
    expect(article?.querySelector(`[data-test-branch="${message.id}"]`)).not.toBeNull();
  }
});
