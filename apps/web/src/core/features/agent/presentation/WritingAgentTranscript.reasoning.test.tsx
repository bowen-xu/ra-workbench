/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingAgentTranscript } from "./WritingAgentTranscript";
import { assistantMetadataIds } from "./WritingAgentMessageMetadata";
import type { WritingAgentMessage } from "./WritingAgentMessage";
import { DeveloperModeProvider } from "../../settings/DeveloperMode";

let root: Root, host: HTMLDivElement;
const prompt: WritingAgentMessage = { id: "turn/user", turnId: "turn", turnFinished: true, turnOutcome: "completed", role: "user", content: "question" };
const summary: WritingAgentMessage = { ...prompt, id: "turn/summary", role: "assistant", kind: "reasoning-summary", content: "公开思考第一部分第二部分",
  reasoningSummaryParts: [{ index: "-9223372036854775808", text: "公开思考第一部分" }, { index: "9223372036854775807", text: "第二部分" }],
  rawData: { itemId: "summary", text: "public only", source: { providerDataJson: '{"opaque":"not visible"}' } } };
const final: WritingAgentMessage = { ...prompt, id: "turn/final", role: "assistant", phase: "finalAnswer", content: "最终回答" };
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.useFakeTimers();
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute("open"); });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.useRealTimers(); });
async function render(messages: readonly WritingAgentMessage[], running = false, searchMessageId?: string) {
  await act(async () => root.render(<WritingAgentTranscript messages={messages} isRunning={running} searchMessageId={searchMessageId}
    messageActions={id => <button data-test-reply-actions={id}>reply action</button>} />));
}
it("keeps a summary-only completed turn in the process and out of assistant-final fallback and reply footers", async () => {
  await render([prompt, summary]);
  const row = host.querySelector('[aria-label="公开思考摘要"]')!;
  expect(row.closest(".writing-agent-turn-process")).not.toBeNull();
  expect(row.textContent).not.toContain("思考摘要");
  expect(row.querySelectorAll("[data-summary-part-index]")).toHaveLength(2);
  expect(row.textContent).toContain("公开思考第一部分"); expect(row.textContent).not.toContain("not visible");
  expect(row.querySelector("time, [data-test-reply-actions], .writing-agent-message-meta")).toBeNull();
  expect(host.querySelector("[data-agent-activity-record]")).toBeNull();
  expect(host.querySelector('[data-latest-response="true"]')).toBeNull();
  expect(assistantMetadataIds([prompt, summary], false).size).toBe(0);
});
it("keeps the final outside the process and pairs a tool receipt across a summary", async () => {
  const call: WritingAgentMessage = { ...summary, kind: "tool-call", id: "turn/call", toolName: "research_list_sources", toolCallId: "call", content: "{}" };
  const result: WritingAgentMessage = { ...call, id: "turn/result", kind: "tool-result", content: "{}" };
  await render([prompt, call, summary, result, final, { ...summary, id: "turn/later-summary" }]);
  const groups = host.querySelectorAll("[data-agent-activity-record]");
  expect(groups).toHaveLength(1);
  expect(groups[0]?.querySelectorAll('[data-tool-status="success"]')).toHaveLength(1);
  expect(groups[0]!.compareDocumentPosition(host.querySelector(".agent-reasoning-summary")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const answer = host.querySelector('[data-agent-message-id="turn/final"]')!;
  expect(answer.closest(".writing-agent-turn-process")).toBeNull();
  expect(answer.querySelector("[data-test-reply-actions]")).not.toBeNull();
  expect(answer.getAttribute("data-latest-response")).toBe("true");
  expect(host.querySelectorAll(".agent-reasoning-summary")).toHaveLength(2);
  expect([...assistantMetadataIds([prompt, final, summary], false)]).toEqual(["turn/final"]);
});
it("shows live shimmer and preserves interrupted parts without a visible label or fake answer", async () => {
  const active = [prompt, summary].map(({ turnOutcome: _outcome, ...message }) => ({ ...message, turnFinished: false }));
  await render(active.map(message => message.kind === "reasoning-summary" ? { ...message, streaming: true } : message), true);
  expect(host.querySelector('.agent-reasoning-summary [data-shimmer-active="true"]')).not.toBeNull();
  await render([{ ...prompt, turnOutcome: "canceled" }, { ...summary, interrupted: true, streaming: false, turnOutcome: "canceled" }]);
  const row = host.querySelector(".agent-reasoning-summary")!;
  expect(row.textContent).not.toContain("未完成");
  expect(row.getAttribute("aria-label")).toBe("未完成的公开思考摘要");
  expect(row.querySelector('[data-shimmer-active="true"]')).toBeNull();
  expect(host.querySelector('[data-agent-activity="aborted"]')).not.toBeNull();
});
it("reveals summary search hits inside the existing process disclosure and hides empty parts", async () => {
  await render([prompt, summary]);
  expect(host.querySelector<HTMLDetailsElement>(".writing-agent-turn-process")?.open).toBe(false);
  await render([prompt, summary], false, summary.id);
  expect(host.querySelector<HTMLDetailsElement>(".writing-agent-turn-process")?.open).toBe(true);
  expect(host.querySelector(".agent-reasoning-summary.search-match")?.getAttribute("data-agent-message-id")).toBe(summary.id);
  await render([prompt, { ...summary, content: "", reasoningSummaryParts: [{ index: "0", text: "" }] }]);
  expect(host.querySelector(".agent-reasoning-summary")).toBeNull();
});
it("reuses the production raw-data inspector lazily for the exact summary item", async () => {
  const previous = Object.getOwnPropertyDescriptor(window, "localStorage");
  Object.defineProperty(window, "localStorage", { configurable: true, value: { getItem: () => "true" } });
  const read = vi.fn(async () => ({ events: ["exact durable summary event"] }));
  try {
    await act(async () => root.render(<DeveloperModeProvider><WritingAgentTranscript messages={[prompt, summary, final]} isRunning={false} readMessageData={read} /></DeveloperModeProvider>));
    expect(read).not.toHaveBeenCalled();
    await act(async () => host.querySelector<HTMLButtonElement>('.agent-reasoning-summary [aria-label="查看原始数据"]')!.click());
    expect(read).toHaveBeenCalledExactlyOnceWith(summary);
    expect(JSON.parse(document.querySelector("dialog pre")!.textContent!).record).toEqual({ events: ["exact durable summary event"] });
    await act(async () => document.querySelector<HTMLButtonElement>('dialog [aria-label="关闭"]')!.click());
  } finally { if (previous) Object.defineProperty(window, "localStorage", previous); else Reflect.deleteProperty(window, "localStorage"); }
});
