/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  applyAgentEvents, createAgentConversationProjection, decodeAgentEvent,
  type AgentConversationProjection, type AgentEvent,
} from "@research-assistant/core/agent";
import completed from "../../../../../../../tests/fixtures/agent-compaction-completed-v1.json";
import failed from "../../../../../../../tests/fixtures/agent-compaction-failed-v1.json";
import canceled from "../../../../../../../tests/fixtures/agent-compaction-canceled-v1.json";
import postCanceled from "../../../../../../../tests/fixtures/agent-compaction-post-canceled-v1.json";
import postFailed from "../../../../../../../tests/fixtures/agent-compaction-post-failed-v1.json";
import { projectConversationMessages } from "../projectConversationMessages";
import type { ReadMessageData } from "../AgentMessageData";
import { DeveloperModeProvider } from "../../settings/DeveloperMode";
import { WritingAgentTranscript } from "./WritingAgentTranscript";

// Actual Service journals exported by Rust native HTTP/configured-hook tests.
// Consume their original order, identities, timestamps and payloads.
const captures = [
  { name: "completed", raw: completed, count: 22, labels: ["上下文已压缩"], installed: [true] },
  { name: "failed", raw: failed, count: 28, labels: ["上下文已压缩", "上下文压缩失败"], installed: [true, false] },
  { name: "canceled", raw: canceled, count: 20, labels: ["上下文压缩已中止"], installed: [false] },
  { name: "post-canceled", raw: postCanceled, count: 8, labels: ["上下文已压缩，任务已中止"], installed: [true] },
  { name: "post-failed", raw: postFailed, count: 8, labels: ["上下文已压缩，后续处理失败"], installed: [true] },
] as const;
function decode(raw: readonly unknown[]): AgentEvent[] {
  const events = raw.map(decodeAgentEvent);
  expect(events).toEqual(raw);
  expect(events.map(event => event.seq)).toEqual(events.map((_, index) => index + 1));
  return events;
}
function replay(events: readonly AgentEvent[]): AgentConversationProjection {
  return applyAgentEvents(createAgentConversationProjection(events[0]!.conversationId), events);
}
function operations(events: readonly AgentEvent[]) {
  return events.flatMap(event => event.payload.type === "itemStarted" && event.payload.kind.type === "contextCompaction"
    ? [{ turnId: event.payload.turnId, itemId: event.payload.itemId, start: event.seq - 1 }] : []);
}

let root: Root, host: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.useFakeTimers();
  Object.defineProperty(window, "localStorage", { configurable: true, value: { getItem: () => "false" } });
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function(this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function(this: HTMLDialogElement) { this.removeAttribute("open"); });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove();
  vi.restoreAllMocks(); vi.useRealTimers(); vi.unstubAllGlobals();
});
async function render(state: AgentConversationProjection, readMessageData?: ReadMessageData) {
  await act(async () => root.render(<DeveloperModeProvider><WritingAgentTranscript
    messages={projectConversationMessages(state)} isRunning={state.activeTurn !== null}
    activeTaskKind={state.activeTurn ? state.turns.get(state.activeTurn)?.taskKind : undefined}
    readMessageData={readMessageData}
  /></DeveloperModeProvider>));
}
const rows = () => [...host.querySelectorAll<HTMLElement>('[aria-label="上下文压缩"]')];

it.each(captures)("decodes and renders the complete original Rust $name ledger with segmented/idempotent replay", async capture => {
  const original = JSON.stringify(capture.raw);
  const events = decode(capture.raw);
  expect(events).toHaveLength(capture.count);
  const full = replay(events);
  let segmented = createAgentConversationProjection(events[0]!.conversationId);
  for (let offset = 0; offset < events.length; offset += 3) segmented = applyAgentEvents(segmented, events.slice(offset, offset + 3));
  expect(segmented).toEqual(full);
  expect(applyAgentEvents(segmented, [events[0]!, events.at(-1)!])).toBe(segmented);
  const messages = projectConversationMessages(full);
  const activities = messages.filter(message => message.kind === "context-compaction");
  const actual = operations(events);
  expect(activities.map(message => message.compaction?.operationId)).toEqual(actual.map(operation => operation.itemId));
  expect(activities.map(message => message.compaction?.windowId !== null)).toEqual(capture.installed);
  await render(full);
  expect(rows().map(row => row.querySelector('[role="status"]')?.textContent)).toEqual(capture.labels);
  for (const [index, operation] of actual.entries()) {
    const row = rows()[index]!;
    expect(row.dataset.agentMessageId).toBe(`${operation.turnId}/${operation.itemId}`);
    expect(row.dataset.agentTurnId).toBe(operation.turnId);
    expect(row.dataset.agentCompactionOperationId).toBe(operation.itemId);
    expect(row.dataset.agentCompactionInstalled).toBe(String(capture.installed[index]));
    expect(row.dataset.agentCompactionWindowId).toBe(capture.installed[index] ? operation.itemId : undefined);
    const turn = host.querySelector(`.writing-agent-turn[data-agent-turn-id="${operation.turnId}"]`)!;
    expect(turn.querySelector('.writing-agent-message:not(.failure), [data-latest-response]:not(.failure)')).toBeNull();
    expect(row.closest(".writing-agent-message")).toBeNull();
    expect(turn.querySelector('[data-agent-activity="thinking"], [data-agent-activity-record]')).toBeNull();
    expect(row.textContent).not.toMatch(/tokens?|80,?000|18,?000|128,?000/i);
  }
  expect(JSON.stringify(capture.raw)).toBe(original);
});

it("observes actual native installation before terminal completion without promoting usage or the receipt", async () => {
  const events = decode(completed);
  const operation = operations(events)[0]!;
  const install = events.findIndex(event => event.payload.type === "itemCompleted" && event.payload.itemId === operation.itemId);
  expect(install).toBeGreaterThan(operation.start);
  await render(replay(events.slice(0, operation.start)));
  expect(rows()).toHaveLength(0);
  expect(host.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  await render(replay(events.slice(0, install)));
  expect(rows()[0]?.querySelector('[role="status"]')?.textContent).toBe("正在压缩上下文");
  expect(rows()[0]?.dataset.agentCompactionInstalled).toBe("false");
  await render(replay(events.slice(0, install + 1)));
  expect(rows()[0]?.querySelector('[role="status"]')?.textContent).toBe("上下文已压缩，正在完成后续处理");
  expect(rows()[0]?.dataset.agentCompactionOutcome).toBe("running");
  expect(rows()[0]?.dataset.agentCompactionWindowId).toBe(operation.itemId);
  expect(host.querySelector('[data-agent-activity="thinking"]')).toBeNull();
});

it("keeps the earlier installed native operation when a later distinct candidate fails", async () => {
  const events = decode(failed);
  const [first, later] = operations(events);
  expect(first).toBeDefined(); expect(later).toBeDefined();
  await render(replay(events.slice(0, later!.start)));
  const installed = rows()[0]!;
  expect(installed.dataset.agentCompactionWindowId).toBe(first!.itemId);
  await render(replay(events));
  expect(rows()).toHaveLength(2);
  expect(rows()[0]).toBe(installed);
  expect(rows()[0]?.dataset.agentCompactionOutcome).toBe("completed");
  expect(rows()[1]?.dataset.agentCompactionOutcome).toBe("failed");
  expect(rows()[1]?.dataset.agentCompactionWindowId).toBeUndefined();
  expect(installed.compareDocumentPosition(rows()[1]!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(replay(events).turns.get(first!.turnId)?.outcome).toBe("completed");
});

it("preserves the actual installed/Failed storage-recovery ledger without inventing an Interrupted or diagnostic event", async () => {
  // The Rust producer separately asserts TaskRegistry RecoveryRequired and
  // checkpoint persistence/reopen. This capture proves only the durable facts.
  const events = decode(postFailed);
  const operation = operations(events)[0]!;
  const state = replay(events), turn = state.turns.get(operation.turnId)!;
  expect(events.some(event => event.payload.type === "turnFailed")).toBe(false);
  expect(events.at(-1)?.payload).toEqual({ type: "turnFinished", turnId: operation.turnId, outcome: "failed" });
  expect(turn.outcome).toBe("failed");
  expect(turn.failure).toBeUndefined();
  expect(turn.items.get(operation.itemId)).toMatchObject({ completed: true, interrupted: false });
  const activity = projectConversationMessages(state).find(message => message.kind === "context-compaction")!;
  expect(activity.compaction).toEqual({ operationId: operation.itemId, windowId: operation.itemId });
  expect(activity.turnOutcome).toBe("failed");
  await render(state);
  expect(rows()[0]?.dataset.agentCompactionInstalled).toBe("true");
  expect(rows()[0]?.dataset.agentCompactionOutcome).toBe("failed");
  expect(rows()[0]?.querySelector('[role="status"]')?.textContent).toBe("上下文已压缩，后续处理失败");
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("本轮未能完成");
});

it("lazily passes the exact native item to the existing inspector with its captured item-only events", async () => {
  Object.defineProperty(window, "localStorage", { configurable: true, value: { getItem: () => "true" } });
  const events = decode(completed);
  const state = replay(events);
  const message = projectConversationMessages(state).find(message => message.kind === "context-compaction")!;
  const itemEvents = events.filter(event => "itemId" in event.payload && event.payload.itemId === message.compaction?.operationId);
  const read = vi.fn<ReadMessageData>(async message => message.role === "user"
    ? { userInput: [] } : { events: itemEvents, frozenSubmission: null });
  await render(state, read);
  // Existing user-source metadata may read references on mount. The compaction
  // operation itself must not read inspection data before its explicit action.
  expect(read.mock.calls.filter(([candidate]) => candidate.kind === "context-compaction")).toHaveLength(0);
  await act(async () => rows()[0]!.querySelector<HTMLButtonElement>('[aria-label="查看原始数据"]')!.click());
  expect(read.mock.calls.filter(([candidate]) => candidate.kind === "context-compaction")).toEqual([[message]]);
  expect(JSON.parse(document.querySelector("dialog pre")!.textContent!).record.events).toEqual(itemEvents);
  expect(itemEvents.some(event => event.payload.type === "turnFinished")).toBe(false);
  await act(async () => document.querySelector<HTMLButtonElement>('dialog [aria-label="关闭"]')!.click());
});
