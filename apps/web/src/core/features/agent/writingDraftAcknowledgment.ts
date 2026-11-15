import {decodeConversationDraftAcknowledgmentToken, type ConversationDraftAcknowledgmentToken} from "./conversationDraftStore";
import type {ConversationDraftStore} from "@research-assistant/workbench-sdk/agent-drafts";
import {agentDraftFromInput, type AgentInputDraft, type AgentUserInput, type AgentWritingQueueScope} from "@research-assistant/core/agent";

/** Application-local provenance. Never send this token to native or a model. */
export interface WritingDraftAcknowledgmentPort {
  flush(): Promise<void>;
  retry(): Promise<void>;
  consume(token: ConversationDraftAcknowledgmentToken, scope: AgentWritingQueueScope): Promise<void>;
}
export interface WritingDraftReceipt {
  readonly draft: AgentInputDraft;
  readonly acknowledge: () => void;
  readonly draftAcknowledgment?: ConversationDraftAcknowledgmentToken | null;
}
export function captureWritingDraftReceipt(resources: {draft(): AgentInputDraft; acknowledgmentToken(text: string): ConversationDraftAcknowledgmentToken | null},
  capture: (text: string) => () => void, text: string): WritingDraftReceipt {
  const draftAcknowledgment = resources.acknowledgmentToken(text);
  return {draft: structuredClone(resources.draft()), acknowledge: capture(text), draftAcknowledgment};
}
export function unownedWritingDraftReceipt(text: string, input: readonly AgentUserInput[] = []): WritingDraftReceipt {
  return {draft: agentDraftFromInput(text, input), acknowledge: () => undefined, draftAcknowledgment: null};
}
export function decodeWritingDraftAcknowledgment(value: unknown, scope: AgentWritingQueueScope): ConversationDraftAcknowledgmentToken | null {
  if (value == null) return null;
  const token = decodeConversationDraftAcknowledgmentToken(value);
  const [kind, hostId, conversationId] = JSON.parse(token.originKey) as [string, string, string | null];
  if (kind !== "manuscript" || hostId !== scope.manuscriptId || (conversationId !== null && conversationId !== scope.conversationId))
    throw new Error("WRITING_PREPARE_DRAFT_ACK_SCOPE_MISMATCH");
  return token;
}
export function writingDraftAcknowledgments(store: ConversationDraftStore): WritingDraftAcknowledgmentPort {
  return {flush: () => store.flush(), retry: () => store.retryPersistence(), consume: async (value, scope) => {
    const token = decodeWritingDraftAcknowledgment(value, scope);
    if (!token) return;
    await store.consumeAcknowledgment(token, {hostKind: "manuscript", hostId: scope.manuscriptId, conversationId: scope.conversationId});
  }};
}
