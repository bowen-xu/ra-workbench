import { expect, it } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent } from "@research-assistant/core/agent";
import { projectConversationMessages } from "./projectConversationMessages";

it.each(["", "foreign::", "命名空间".repeat(100)])("keeps namespace %j in raw transcript facts without rewriting the bare display name", namespace => {
  const conversationId = "11111111-1111-4111-8111-111111111111", turnId = "22222222-2222-4222-8222-222222222222";
  const itemId = "33333333-3333-4333-8333-333333333333", callId = "44444444-4444-4444-8444-444444444444";
  const payloads = [
    { type: "turnStarted", turnId, contextSnapshotId: itemId, model: { providerId: "openai", modelId: "gpt-5.6-terra", revision: null } },
    { type: "itemAccepted", turnId, itemId, kind: { type: "toolCall", callId, name: "research_list_sources", namespace }, text: "{}", phase: null, source: null },
  ];
  const events = payloads.map((payload, index) => decodeAgentEvent({ schemaVersion: 1, conversationId, seq: index + 1, timestampMs: 1, requestId: turnId, payload }));
  const projection = applyAgentEvents(createAgentConversationProjection(conversationId), events);
  const message = projectConversationMessages(projection)[0];
  expect(message).toMatchObject({ kind: "tool-call", toolName: "research_list_sources", toolCallId: callId,
    rawData: { kind: { type: "toolCall", name: "research_list_sources", namespace } } });
});
