/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  applyAgentEvents, createAgentConversationProjection, decodeAgentEvent,
  type AgentConversationProjection, type AgentEventPayload,
} from "@research-assistant/core/agent";
import { projectConversationMessages } from "../projectConversationMessages";
import type { ReadMessageData } from "../AgentMessageData";
import { DeveloperModeProvider } from "../../settings/DeveloperMode";
import { WritingAgentTranscript } from "./WritingAgentTranscript";

const id = (value: number): string => `00000000-0000-0000-0000-${value.toString(16).padStart(12, "0")}`;
const model = { providerId: "fixture", modelId: "compact", revision: null };
const start: AgentEventPayload = { type: "turnStarted", turnId: id(10), contextSnapshotId: id(12), model, taskKind: "compact" };
const begun: AgentEventPayload = { type: "itemStarted", turnId: id(10), itemId: id(11), kind: { type: "contextCompaction" } };
const installed: AgentEventPayload = { type: "itemCompleted", turnId: id(10), itemId: id(11) };
function project(payloads: readonly AgentEventPayload[]) {
  return applyAgentEvents(createAgentConversationProjection(id(1)), payloads.map((payload, index) => decodeAgentEvent({
    schemaVersion: 1, conversationId: id(1), requestId: id(4), seq: index + 1, timestampMs: index + 1, payload,
  })));
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
    readMessageData={readMessageData} messageActions={messageId => <button data-test-reply={messageId}>reply</button>}
  /></DeveloperModeProvider>));
}
const row = () => host.querySelector<HTMLElement>('[aria-label="上下文压缩"]')!;
function noFakeReply() {
  expect(host.querySelector(".writing-agent-message, [data-latest-response], .writing-agent-message-meta, [data-test-reply]")).toBeNull();
  expect(host.querySelector('[data-agent-activity="thinking"], [data-agent-activity-record]')).toBeNull();
  expect(host.textContent).not.toMatch(/80,?000|18,?000|128,?000|tokens?/i);
}

it("keeps manual admission/installation pending without an empty assistant reply or generic thinking", async () => {
  await render(project([start]));
  expect(row()).toBeNull();
  expect(host.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  await render(project([start, begun]));
  expect(row().querySelector('[role="status"]')?.textContent).toBe("正在压缩上下文");
  expect(row().dataset.agentCompactionOperationId).toBe(id(11));
  expect(row().dataset.agentCompactionInstalled).toBe("false");
  expect(row().dataset.agentMessageId).toBe(`${id(10)}/${id(11)}`);
  expect(row().closest('[data-agent-turn-id]')?.getAttribute("data-agent-turn-id")).toBe(id(10));
  noFakeReply();
  await render(project([start, begun, installed]));
  expect(row().querySelector('[role="status"]')?.textContent).toBe("上下文已压缩，正在完成后续处理");
  expect(row().dataset.agentCompactionWindowId).toBe(id(11));
  expect(row().dataset.agentCompactionOutcome).toBe("running");
  const process = row().closest("details") as HTMLDetailsElement;
  expect(process.dataset.flat).toBe("true");
  expect(process.open).toBe(true);
  expect(process.querySelector<HTMLElement>(":scope > summary")?.hidden).toBe(true);
  noFakeReply();
});

it("retains the installed window beside an independent later task diagnostic or cancellation", async () => {
  await render(project([start, begun, installed,
    { type: "turnFailed", turnId: id(10), error: "later task failed", category: "admission" },
    { type: "turnFinished", turnId: id(10), outcome: "failed" },
  ]));
  expect(row().querySelector('[role="status"]')?.textContent).toBe("上下文已压缩，后续处理失败");
  expect(row().dataset.agentCompactionInstalled).toBe("true");
  expect(row().dataset.agentCompactionOutcome).toBe("failed");
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  await render(project([start, begun, installed, { type: "turnFinished", turnId: id(10), outcome: "canceled" }]));
  expect(row().querySelector('[role="status"]')?.textContent).toBe("上下文已压缩，任务已中止");
  expect(row().dataset.agentCompactionInstalled).toBe("true");
  noFakeReply();
});

it("shows pre-install cancellation without claiming installation or token savings", async () => {
  await render(project([start, begun, { type: "turnFinished", turnId: id(10), outcome: "canceled" }]));
  expect(row().querySelector('[role="status"]')?.textContent).toBe("上下文压缩已中止");
  expect(row().dataset.agentCompactionInstalled).toBe("false");
  expect(row().dataset.agentCompactionWindowId).toBeUndefined();
  noFakeReply();
});

it("keeps automatic compaction in item order and resumes normal thinking after installation", async () => {
  const regular = { ...start, taskKind: "regular" as const };
  const user: AgentEventPayload[] = [
    { type: "itemStarted", turnId: id(10), itemId: id(15), kind: { type: "userMessage" } },
    { type: "textDelta", turnId: id(10), itemId: id(15), text: "synthetic user" },
    { type: "itemCompleted", turnId: id(10), itemId: id(15) },
  ];
  await render(project([regular, ...user, begun]));
  const prompt = host.querySelector('[data-agent-message-id="' + id(10) + '/' + id(15) + '"]')!;
  expect(prompt.compareDocumentPosition(row()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(host.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  await render(project([regular, ...user, begun, installed]));
  expect(host.querySelector('[data-agent-activity="thinking"]')).not.toBeNull();
});

it("removes a rolled-back activity naturally and replay never restores an orphan", async () => {
  const facts: AgentEventPayload[] = [
    { ...start, taskKind: "regular" },
    { type: "itemStarted", turnId: id(10), itemId: id(15), kind: { type: "userMessage" } },
    { type: "itemCompleted", turnId: id(10), itemId: id(15) },
    { type: "turnFinished", turnId: id(10), outcome: "completed" },
    { ...start, turnId: id(20) },
    { ...begun, turnId: id(20) }, { ...installed, turnId: id(20) },
    { type: "turnFinished", turnId: id(20), outcome: "completed" },
  ];
  await render(project(facts));
  expect(row()).not.toBeNull();
  const after = [...facts, { type: "conversationRolledBack", turnId: id(20), userTurns: 1 } as const];
  await render(project(after));
  expect(host.querySelector(".agent-compaction-activity")).toBeNull();
  await render(project(after));
  expect(host.querySelector(".agent-compaction-activity")).toBeNull();
});

it("reads the exact durable operation through the shared raw-data reader only on demand", async () => {
  Object.defineProperty(window, "localStorage", { configurable: true, value: { getItem: () => "true" } });
  const state = project([start, begun, installed]);
  const message = projectConversationMessages(state)[0]!;
  const record = { events: [begun, installed], frozenSubmission: { contextSnapshotId: id(12) } };
  const read = vi.fn(async () => record);
  await render(state, read);
  expect(read).not.toHaveBeenCalled();
  await act(async () => row().querySelector<HTMLButtonElement>('[aria-label="查看原始数据"]')!.click());
  expect(read).toHaveBeenCalledExactlyOnceWith(message);
  expect(JSON.parse(document.querySelector("dialog pre")!.textContent!).record).toEqual(record);
  await act(async () => document.querySelector<HTMLButtonElement>('dialog [aria-label="关闭"]')!.click());
});
