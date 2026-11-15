/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent, type AgentEventPayload } from "@research-assistant/core/agent";
import { projectConversationMessages } from "../projectConversationMessages";
import { WritingAgentTranscript } from "./WritingAgentTranscript";
import type { WritingAgentMessage } from "./WritingAgentMessage";

const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const conversationId = id(1), turnId = id(2), userId = id(3), firstId = id(4), nextId = id(5);
const attempt1 = id(6), attempt2 = id(7), attempt3 = id(8);
const model = {providerId: "fixture", modelId: "fixture", revision: null};
const start: AgentEventPayload[] = [
  {type: "turnStarted", turnId, contextSnapshotId: userId, model},
  {type: "itemStarted", turnId, itemId: userId, kind: {type: "userMessage"}},
  {type: "textDelta", turnId, itemId: userId, text: "Question"},
  {type: "itemCompleted", turnId, itemId: userId},
  {type: "phaseChanged", turnId, phase: "runningModel"},
];
const failed = (attemptId: string): AgentEventPayload => ({type: "modelRequestFailed", turnId, attemptId, error: "offline", retryable: true, category: null});
function messages(payloads: readonly AgentEventPayload[]): readonly WritingAgentMessage[] {
  return projectConversationMessages(applyAgentEvents(createAgentConversationProjection(conversationId), payloads.map((payload, index) =>
    decodeAgentEvent({schemaVersion: 1, conversationId, requestId: turnId, seq: index + 1, timestampMs: index + 1, payload}))));
}
let root: Root, container: HTMLDivElement;
beforeEach(() => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => {await act(async () => root.unmount()); container.remove();});
async function render(items: readonly WritingAgentMessage[], isRunning: boolean, searchMessageId?: string) {
  await act(async () => root.render(<div className="writing-agent-messages"><div className="writing-agent-transcript">
    <WritingAgentTranscript messages={items} isRunning={isRunning} searchMessageId={searchMessageId} />
  </div></div>));
}
function before(left: Element, right: Element): boolean {return Boolean(left.compareDocumentPosition(right) & Node.DOCUMENT_POSITION_FOLLOWING);}

it("places a durable retry between its interrupted summary and the next summary, including after history replay", async () => {
  const events: AgentEventPayload[] = [...start,
    {type: "modelRequestStarted", turnId, attemptId: attempt1, model},
    {type: "modelResponseStarted", turnId, attemptId: attempt1, responseId: "first", model: null},
    {type: "itemStarted", turnId, itemId: firstId, kind: {type: "reasoningSummary", parts: []}},
    {type: "reasoningSummaryDelta", turnId, itemId: firstId, summaryIndex: "0", text: "draft"},
    failed(attempt1),
    {type: "modelRequestStarted", turnId, attemptId: attempt2, model},
    {type: "modelResponseStarted", turnId, attemptId: attempt2, responseId: "next", model: null},
    {type: "itemAccepted", turnId, itemId: nextId, kind: {type: "reasoningSummary", parts: [{index: "0", startByte: 0, lenByte: 4}]}, text: "done", phase: null,
      source: {attemptId: attempt2, ordinal: 0, providerItemId: "summary", providerDataJson: null}},
    {type: "itemAccepted", turnId, itemId: id(9), kind: {type: "assistantMessage"}, text: "Final", phase: "finalAnswer", source: null},
    {type: "modelRequestFinished", turnId, attemptId: attempt2, inputTokens: null, outputTokens: null},
    {type: "turnFinished", turnId, outcome: "completed"},
  ];
  for (const reopen of [false, true]) {
    if (reopen) await act(async () => root.render(null));
    await render(messages(events), false);
    const summaries = container.querySelectorAll('.agent-reasoning-summary');
    const history = container.querySelector('[data-agent-activity="reconnect-history"]')!;
    const final = container.querySelector(`[data-agent-message-id="${turnId}/${id(9)}"]`)!;
    expect(summaries).toHaveLength(2);
    expect(before(summaries[0]!, history)).toBe(true); expect(before(history, summaries[1]!)).toBe(true);
    expect(history.closest('.writing-agent-turn-process')).not.toBeNull();
    expect(final.closest('.writing-agent-turn-process')).toBeNull();
    expect(container.querySelector<HTMLDetailsElement>('.writing-agent-turn-process')?.open).toBe(false);
  }
});

it("keeps the visible streamed draft stable through consecutive empty retries, then places them before the new sample", async () => {
  const events: AgentEventPayload[] = [...start,
    {type: "modelRequestStarted", turnId, attemptId: attempt1, model},
    {type: "itemStarted", turnId, itemId: firstId, kind: {type: "assistantMessage"}},
    {type: "itemPhaseChanged", turnId, itemId: firstId, phase: "finalAnswer", native: true},
    {type: "textDelta", turnId, itemId: firstId, text: "Already seen"},
  ];
  await render(messages(events), true);
  const original = container.querySelector(`[data-agent-message-id="${turnId}/${firstId}"]`)!;
  events.push(failed(attempt1), {type: "modelRequestStarted", turnId, attemptId: attempt2, model}, failed(attempt2));
  await render(messages(events), true);
  const retries = [...container.querySelectorAll('[data-agent-activity="reconnect-history"]')];
  expect(retries).toHaveLength(1); expect(retries[0]?.textContent).toContain("重连记录（2 条）");
  expect(retries.every(retry => before(original, retry))).toBe(true);
  expect(container.querySelector('[data-agent-activity="reconnecting"]')?.textContent).toContain("2/5");
  events.push({type: "modelRequestStarted", turnId, attemptId: attempt3, model},
    {type: "itemStarted", turnId, itemId: nextId, kind: {type: "assistantMessage"}},
    {type: "textDelta", turnId, itemId: nextId, text: "Already"});
  await render(messages(events), true);
  const replacement = container.querySelector(`[data-agent-message-id="${turnId}/${nextId}"]`)!;
  expect(replacement).toBe(original);
  expect(replacement.textContent).toContain("Already seen");
  const moved = [...container.querySelectorAll('[data-agent-activity="reconnect-history"]')];
  expect(moved.every(retry => before(retry, replacement))).toBe(true);
  expect(container.querySelector('[data-agent-activity="reconnecting"]')).toBeNull();
});

it("places consecutive failures before the first output when no earlier sample produced visible content", async () => {
  const events: AgentEventPayload[] = [...start,
    {type: "modelRequestStarted", turnId, attemptId: attempt1, model}, failed(attempt1),
    {type: "modelRequestStarted", turnId, attemptId: attempt2, model}, failed(attempt2),
    {type: "modelRequestStarted", turnId, attemptId: attempt3, model},
    {type: "itemStarted", turnId, itemId: nextId, kind: {type: "assistantMessage"}},
    {type: "itemPhaseChanged", turnId, itemId: nextId, phase: "finalAnswer", native: true},
    {type: "textDelta", turnId, itemId: nextId, text: "Final"},
  ];
  await render(messages(events), true);
  const retries = [...container.querySelectorAll('[data-agent-activity="reconnect-history"]')];
  const final = container.querySelector(`[data-agent-message-id="${turnId}/${nextId}"]`)!;
  expect(retries).toHaveLength(1); expect(retries[0]?.textContent).toContain("重连记录（2 条）");
  expect(retries.every(retry => before(retry, final))).toBe(true);
});

it("groups five consecutive retries before a terminal error and its actions, including after reopening", async () => {
  const retry = Array.from({length: 5}, (_, index) => ({attemptId: id(index + 20), reason: `原因 ${index + 1}`, failedSeq: index + 2, attemptIndex: index}));
  const items: WritingAgentMessage[] = [
    {id: "prompt", turnId: "turn", role: "user", content: "问题", turnFinished: true, reconnectHistory: retry},
    {id: "failure", turnId: "turn", role: "assistant", content: "请求失败", failure: true, turnFinished: true, turnOutcome: "failed", reconnectHistory: retry},
  ];
  for (const reopen of [false, true]) {
    if (reopen) await act(async () => root.render(null));
    await render(items, false);
    const history = container.querySelectorAll('[data-agent-activity="reconnect-history"]');
    const failure = container.querySelector('[data-agent-message-id="failure"]')!;
    const actions = failure.querySelector('[data-agent-message-actions]')!;
    expect(history).toHaveLength(1);
    expect(history[0]?.querySelector('summary')?.textContent).toContain("重连记录（5 条）");
    expect(history[0]?.querySelectorAll('li')).toHaveLength(5);
    expect(before(history[0]!, failure)).toBe(true);
    expect(before(history[0]!, actions)).toBe(true);
  }
});

it("places response actions after later reconnect activity even without a later reply", async () => {
  const retry = [{attemptId: attempt1, reason: "offline", failedSeq: 5, attemptIndex: 0}];
  await render([
    {id: "prompt", turnId: "turn", role: "user", content: "问题", turnFinished: true},
    {id: "final", turnId: "turn", role: "assistant", phase: "finalAnswer", content: "回答", createdSeq: 3, turnFinished: true, reconnectHistory: retry},
  ], false);
  const response = container.querySelector('[data-agent-message-id="final"]')!;
  const history = container.querySelector('[data-agent-activity="reconnect-history"]')!;
  const actions = container.querySelector('.writing-agent-turn [data-agent-message-actions]')!;
  expect(before(response, history)).toBe(true);
  expect(before(history, actions)).toBe(true);
});

it("does not move late tool results into an activity group before the retry", async () => {
  const retry = [{attemptId: attempt1, reason: "offline", failedSeq: 3, attemptIndex: 0}];
  const common = {turnId: "turn", turnFinished: true, role: "assistant" as const, content: "", reconnectHistory: retry};
  await render([
    {...common, id: "prompt", role: "user", content: "question", createdSeq: 1},
    {...common, id: "call", kind: "tool-call", toolCallId: "call", toolName: "read", content: "{}", createdSeq: 2},
    {...common, id: "result", kind: "tool-result", toolCallId: "call", toolName: "read", content: "result", createdSeq: 4},
    {...common, id: "final", phase: "finalAnswer", content: "Final", createdSeq: 5},
  ], false);
  const groups = container.querySelectorAll('[data-agent-activity-record]');
  const history = container.querySelector('[data-agent-activity="reconnect-history"]')!;
  expect(groups).toHaveLength(2); expect(before(groups[0]!, history)).toBe(true); expect(before(history, groups[1]!)).toBe(true);
  expect(groups[0]!.querySelector('[data-tool-status="success"]')).not.toBeNull();
  expect(groups[0]!.querySelector('[data-tool-status="interrupted"]')).toBeNull();
});

it("keeps reused legacy tool identities from matching a later conversation turn", async () => {
  const retry = [{attemptId: attempt1, reason: "offline"}];
  await render([
    {id: "prompt-one", role: "user", content: "one", turnFinished: true, reconnectHistory: retry},
    {id: "call-one", role: "assistant", kind: "tool-call", toolCallId: "reused", toolName: "read", content: "{}", turnFinished: true},
    {id: "prompt-two", role: "user", content: "two", turnFinished: true},
    {id: "result-two", role: "assistant", kind: "tool-result", toolCallId: "reused", toolName: "read", content: "{}", turnFinished: true},
  ], false);
  const groups = container.querySelectorAll('[data-agent-activity-record]');
  expect(groups).toHaveLength(2);
  expect(groups[0]!.querySelector('[data-tool-status="interrupted"]')).not.toBeNull();
});

it("keeps provider item order and places separate retries around later user input and process content", async () => {
  const retry = [{attemptId: attempt1, reason: "first", failedSeq: 4, attemptIndex: 0},
    {attemptId: attempt2, reason: "second", failedSeq: 7, attemptIndex: 1}];
  const common = {turnId: "turn", turnFinished: true, role: "assistant" as const, content: "", reconnectHistory: retry};
  await render([
    {...common, id: "prompt", role: "user", content: "question", createdSeq: 1},
    {...common, id: "provider-first", phase: "commentary", content: "first output", createdSeq: 3},
    {...common, id: "provider-second", phase: "commentary", content: "second output", createdSeq: 2},
    {...common, id: "steer", role: "user", content: "steer", createdSeq: 5},
    {...common, id: "after-steer", phase: "commentary", content: "later output", createdSeq: 6},
    {...common, id: "final", phase: "finalAnswer", content: "Final", createdSeq: 8},
  ], false);
  const find = (name: string) => container.querySelector(`[data-agent-message-id="${name}"]`)!;
  const retries = container.querySelectorAll('[data-agent-activity="reconnect-history"]');
  expect(retries).toHaveLength(2);
  expect(before(find("provider-first"), find("provider-second"))).toBe(true);
  expect(before(find("provider-second"), retries[0]!)).toBe(true);
  expect(before(retries[0]!, find("steer"))).toBe(true);
  expect(before(find("after-steer"), retries[1]!)).toBe(true);
  expect(before(retries[1]!, find("final"))).toBe(true);
});

it("uses attempt identity when a new retry follows a cached item with no event sequence", async () => {
  const retry = [{attemptId: attempt1, reason: "offline", failedSeq: 4, attemptIndex: 0}];
  const common = {turnId: "turn", turnFinished: true, role: "assistant" as const, content: "", reconnectHistory: retry};
  await render([
    {...common, id: "prompt", role: "user", content: "question", createdSeq: 1},
    {...common, id: "old", kind: "reasoning-summary", attemptIndex: 0, reasoningSummaryParts: [{index: "0", text: "before"}]},
    {...common, id: "new", kind: "reasoning-summary", createdSeq: 6, attemptIndex: 1, reasoningSummaryParts: [{index: "0", text: "after"}]},
    {...common, id: "final", phase: "finalAnswer", content: "Final", createdSeq: 7},
  ], false);
  const old = container.querySelector('[data-agent-message-id="old"]')!;
  const next = container.querySelector('[data-agent-message-id="new"]')!;
  const marker = container.querySelector('[data-agent-activity="reconnect-history"]')!;
  expect(before(old, marker)).toBe(true); expect(before(marker, next)).toBe(true);
});

it("shows a reconnecting attempt even when the turn has no displayable items yet", async () => {
  const events: AgentEventPayload[] = [start[0]!, {type: "phaseChanged", turnId, phase: "runningModel"},
    {type: "modelRequestStarted", turnId, attemptId: attempt1, model}, failed(attempt1)];
  await render(messages(events), true);
  expect(container.querySelector('[data-agent-activity="reconnect-history"]')).not.toBeNull();
  expect(container.querySelector('[data-agent-activity="reconnecting"]')?.textContent).toContain("1/5");
  expect(container.querySelector('[data-agent-activity="thinking"]')).toBeNull();
});

it("keeps two accepted final answers around the retries and later summary, with one shared disclosure after replay", async () => {
  const events: AgentEventPayload[] = [...start,
    {type: "modelRequestStarted", turnId, attemptId: attempt1, model},
    {type: "itemAccepted", turnId, itemId: firstId, kind: {type: "assistantMessage"}, text: "First final", phase: "finalAnswer", source: null},
    failed(attempt1),
    {type: "modelRequestStarted", turnId, attemptId: attempt2, model},
    {type: "modelResponseStarted", turnId, attemptId: attempt2, responseId: "second", model: null},
    {type: "itemAccepted", turnId, itemId: nextId, kind: {type: "reasoningSummary", parts: [{index: "0", startByte: 0, lenByte: 4}]}, text: "note", phase: null,
      source: {attemptId: attempt2, ordinal: 0, providerItemId: "summary", providerDataJson: null}},
    failed(attempt2),
    {type: "modelRequestStarted", turnId, attemptId: attempt3, model},
    {type: "modelResponseStarted", turnId, attemptId: attempt3, responseId: "third", model: null},
    {type: "itemAccepted", turnId, itemId: id(9), kind: {type: "assistantMessage"}, text: "Second final", phase: "finalAnswer", source: null},
    {type: "modelRequestFinished", turnId, attemptId: attempt3, inputTokens: null, outputTokens: null},
    {type: "turnFinished", turnId, outcome: "completed"},
  ];
  for (const reopen of [false, true]) {
    if (reopen) await act(async () => root.render(null));
    await render(messages(events), false);
    const first = container.querySelector(`[data-agent-message-id="${turnId}/${firstId}"]`)!;
    const second = container.querySelector(`[data-agent-message-id="${turnId}/${id(9)}"]`)!;
    const summary = container.querySelector('.agent-reasoning-summary')!;
    const retries = container.querySelectorAll('[data-agent-activity="reconnect-history"]');
    const disclosure = container.querySelector<HTMLDetailsElement>('.writing-agent-turn-process')!;
    const laterProcess = summary.closest<HTMLElement>('.writing-agent-turn-content')!;
    expect(retries).toHaveLength(2);
    expect(before(first, retries[0]!)).toBe(true); expect(before(retries[0]!, summary)).toBe(true);
    expect(before(summary, retries[1]!)).toBe(true); expect(before(retries[1]!, second)).toBe(true);
    expect(first.closest('.writing-agent-turn-content')).toBeNull(); expect(second.closest('.writing-agent-turn-content')).toBeNull();
    expect(disclosure.open).toBe(false); expect(laterProcess.hidden).toBe(true);
    const scroller = container.querySelector<HTMLElement>('.writing-agent-messages')!;
    scroller.scrollTop = 50; const preserve = vi.fn(); scroller.addEventListener('writing-agent-preserve-position', preserve);
    await act(async () => disclosure.querySelector('summary')!.click());
    expect(disclosure.open).toBe(true); expect(laterProcess.hidden).toBe(false);
    expect(preserve).toHaveBeenCalledOnce(); expect(scroller.scrollTop).toBe(50);
    expect(container.querySelector(`[data-agent-message-id="${turnId}/${firstId}"]`)).toBe(first);
    expect(container.querySelector(`[data-agent-message-id="${turnId}/${id(9)}"]`)).toBe(second);
    await act(async () => disclosure.querySelector('summary')!.click());
    expect(laterProcess.hidden).toBe(true);
    await render(messages(events), false, `${turnId}/${nextId}`);
    expect(disclosure.open).toBe(true); expect(laterProcess.hidden).toBe(false);
    scroller.removeEventListener('writing-agent-preserve-position', preserve);
  }
});

it("keeps the first accepted final node stable while retries and a second streamed final are inserted", async () => {
  const events: AgentEventPayload[] = [...start,
    {type: "modelRequestStarted", turnId, attemptId: attempt1, model},
    {type: "itemAccepted", turnId, itemId: firstId, kind: {type: "assistantMessage"}, text: "First final", phase: "finalAnswer", source: null},
  ];
  await render(messages(events), true);
  const original = container.querySelector(`[data-agent-message-id="${turnId}/${firstId}"]`)!;
  events.push(failed(attempt1)); await render(messages(events), true);
  expect(container.querySelector(`[data-agent-message-id="${turnId}/${firstId}"]`)).toBe(original);
  expect(before(original, container.querySelector('[data-agent-activity="reconnect-history"]')!)).toBe(true);
  events.push({type: "modelRequestStarted", turnId, attemptId: attempt2, model},
    {type: "modelResponseStarted", turnId, attemptId: attempt2, responseId: "second", model: null},
    {type: "itemAccepted", turnId, itemId: nextId, kind: {type: "reasoningSummary", parts: [{index: "0", startByte: 0, lenByte: 4}]}, text: "note", phase: null,
      source: {attemptId: attempt2, ordinal: 0, providerItemId: "summary", providerDataJson: null}},
    failed(attempt2), {type: "modelRequestStarted", turnId, attemptId: attempt3, model},
    {type: "itemStarted", turnId, itemId: id(9), kind: {type: "assistantMessage"}},
    {type: "itemPhaseChanged", turnId, itemId: id(9), phase: "finalAnswer", native: true},
    {type: "textDelta", turnId, itemId: id(9), text: "Second final"});
  await render(messages(events), true);
  const second = container.querySelector(`[data-agent-message-id="${turnId}/${id(9)}"]`)!;
  const retries = container.querySelectorAll('[data-agent-activity="reconnect-history"]');
  expect(container.querySelector(`[data-agent-message-id="${turnId}/${firstId}"]`)).toBe(original);
  expect(retries).toHaveLength(2); expect(before(original, retries[0]!)).toBe(true);
  expect(before(retries[1]!, second)).toBe(true);
});
