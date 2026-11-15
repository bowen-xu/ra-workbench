import { expect, it, vi } from "vitest";
import { agentDraftInput, agentDraftFromInput, type AgentUserInput } from "@research-assistant/core/agent";
import { notify } from "../../services/notifications";
import { AGENT_PREPARATION_FAILURE, recoverAgentPreparationFailure, isAgentStartRejection } from "./agentPreparationFailure";

vi.mock("../../services/notifications", () => ({notify: {error: vi.fn()}}));
it("recognizes only the known native preadmission wire and legacy rejection forms", () => {
  for (const cause of ["modelUnavailable", "confirmationMismatch", "conversationUnavailable", "invalidInput", {code: "confirmationMismatch"},
    {code: "unsupportedModel"}, {code: "preparation", reason: "conflict"}]) expect(isAgentStartRejection(cause)).toBe(true);
  for (const cause of [{code: "unavailable"}, {code: "recoveryRequired"}, {code: "preparation", reason: "unavailable"},
    {code: "preparation", reason: "modelUnavailable"}, {code: "preparation", reason: {code: "conflict"}}, new Error("lost ACK")]) expect(isAgentStartRejection(cause)).toBe(false);
});

it("restores the unsent text and atomic references through the shared composer codec", () => {
  const text = "Explain source", userInput: readonly AgentUserInput[] = [
    {type: "text", text, textElements: [{startUtf16: 8, endUtf16: 14, itemIndex: 1}]},
    {type: "mention", name: "source", target: {kind: "file", sourceId: "context:note", resourceId: "11111111-1111-4111-8111-111111111111", revision: "one"}},
  ];
  const restore = vi.fn();
  const originalDraft = {text: "", bindings: []};
  recoverAgentPreparationFailure({text, userInput, originalDraft, read: () => originalDraft, restore});
  expect(restore).toHaveBeenCalledExactlyOnceWith(agentDraftFromInput(text, userInput));
  expect(agentDraftInput(restore.mock.calls[0]![0])).toEqual(userInput);
  expect(notify.error).toHaveBeenLastCalledWith(AGENT_PREPARATION_FAILURE);
});

it.each(["Unsent message", "A newer draft"])("retains existing composer content (%s)", text => {
  const restore = vi.fn();
  const originalDraft = {text, bindings: []};
  recoverAgentPreparationFailure({text: "Unsent message", originalDraft, read: () => originalDraft, restore});
  expect(restore).not.toHaveBeenCalled();
});

it("does not show a late failure from a detached preview", () => {
  vi.mocked(notify.error).mockClear();
  const originalDraft = {text: "Unsent", bindings: []};
  recoverAgentPreparationFailure({text: "Unsent", originalDraft, read: () => originalDraft, restore: vi.fn(), notifyFailure: false});
  expect(notify.error).not.toHaveBeenCalled();
});

it("does not repopulate a newer deliberately cleared draft", () => {
  const restore = vi.fn();
  recoverAgentPreparationFailure({text: "Old message", originalDraft: {text: "Old message", bindings: []},
    read: () => ({text: "", bindings: []}), restore});
  expect(restore).not.toHaveBeenCalled();
});
