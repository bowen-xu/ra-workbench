/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { agentDraftInput, insertAgentReference } from "@research-assistant/core/agent";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import { AgentPendingSubmissionBubble, hasConfirmedSubmissionMessage, type PendingAgentSubmission } from "./AgentPendingSubmissionBubble";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("requires the matching conversation and turn's complete first user message", () => {
  const pending: PendingAgentSubmission = { id: "pending", conversationId: "conversation", turnId: "turn", text: "相同内容" };
  const message: WritingAgentMessage = { id: "turn/item", conversationId: "conversation", turnId: "turn",
    role: "user", content: "相同内容", streaming: false };
  expect(hasConfirmedSubmissionMessage(pending, [])).toBe(false);
  expect(hasConfirmedSubmissionMessage({ ...pending, turnId: undefined }, [message])).toBe(false);
  expect(hasConfirmedSubmissionMessage(pending, [{ ...message, turnId: "other" }])).toBe(false);
  expect(hasConfirmedSubmissionMessage(pending, [{ ...message, conversationId: "other" }])).toBe(false);
  expect(hasConfirmedSubmissionMessage(pending, [{ ...message, streaming: true }])).toBe(false);
  expect(hasConfirmedSubmissionMessage(pending, [{ ...message, interrupted: true }])).toBe(false);
  expect(hasConfirmedSubmissionMessage(pending, [{ ...message, content: "插话" }, message])).toBe(false);
  expect(hasConfirmedSubmissionMessage(pending, [message])).toBe(true);
});

it("renders the production user bubble with references until the actual message replaces it", async () => {
  const draft = insertAgentReference({ text: "使用 ", bindings: [] }, {
    type: "mention", name: "工具", target: { kind: "tool", serverId: "server", environmentId: "test", revision: "v1" },
  });
  const pending: PendingAgentSubmission = { id: "pending", conversationId: "conversation", turnId: "turn",
    text: draft.text, userInput: agentDraftInput(draft) };
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const original: WritingAgentMessage = { id: "turn/user", conversationId: "conversation", turnId: "turn",
    role: "user", content: draft.text, streaming: false };
  try {
    expect(hasConfirmedSubmissionMessage(pending, [{ ...original, userInput: [] }])).toBe(false);
    expect(hasConfirmedSubmissionMessage(pending, [{ ...original, userInput: agentDraftInput(draft) }])).toBe(true);
    await act(async () => root.render(<AgentPendingSubmissionBubble pending={pending} messages={[]} />));
    expect(host.querySelector('[data-pending-submission="true"]')).not.toBeNull();
    expect(host.querySelector('[aria-label="工具 工具"]')).not.toBeNull();
    await act(async () => root.render(<AgentPendingSubmissionBubble pending={pending} messages={[{ ...original, turnId: "old" }]} />));
    expect(host.querySelector('[data-pending-submission="true"]')).not.toBeNull();
    await act(async () => root.render(<AgentPendingSubmissionBubble pending={pending} messages={[original]} />));
    expect(host.querySelector('[data-pending-submission="true"]')).toBeNull();
    await act(async () => root.render(<AgentPendingSubmissionBubble pending={null} messages={[]} />));
    expect(host.querySelector('[data-pending-submission="true"]')).toBeNull();
  } finally { await act(async () => root.unmount()); host.remove(); }
});
