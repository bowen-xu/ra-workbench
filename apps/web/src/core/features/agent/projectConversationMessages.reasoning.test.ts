import { expect, it } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent, type AgentEventPayload } from "@research-assistant/core/agent";
import { projectConversationMessages } from "./projectConversationMessages";

const conversationId = "11111111-1111-4111-8111-111111111111", turnId = "22222222-2222-4222-8222-222222222222";
const itemId = "33333333-3333-4333-8333-333333333333", attemptId = "44444444-4444-4444-8444-444444444444";
const model = { providerId: "fixture", modelId: "fixture", revision: null };
const start: AgentEventPayload[] = [{ type: "turnStarted", turnId, contextSnapshotId: itemId, model },
  { type: "phaseChanged", turnId, phase: "runningModel" }, { type: "modelRequestStarted", turnId, attemptId, model },
  { type: "modelResponseStarted", turnId, attemptId, responseId: "response", model: null }];
function project(tail: AgentEventPayload[]) {
  const projection = applyAgentEvents(createAgentConversationProjection(conversationId), [...start, ...tail].map((payload, index) => decodeAgentEvent({ schemaVersion: 1, conversationId, seq: index + 1, timestampMs: 1, requestId: turnId, payload })));
  return projectConversationMessages(projection);
}
it("projects public canonical parts as an independent process kind without a fabricated assistant phase", () => {
  const messages = project([{ type: "itemAccepted", turnId, itemId, kind: { type: "reasoningSummary", parts: [{ index: "0", startByte: 0, lenByte: 6 }, { index: "1", startByte: 6, lenByte: 4 }] },
    text: "思考🦀", phase: null, source: { attemptId, ordinal: 0, providerItemId: "reasoning", providerDataJson: '{"opaque":"never render me"}' } }]);
  expect(messages).toHaveLength(1);
  expect(messages[0]).toMatchObject({ kind: "reasoning-summary", content: "思考🦀", reasoningSummaryParts: [{ index: "0", text: "思考" }, { index: "1", text: "🦀" }], rawData: { itemId, source: { ordinal: 0 } } });
  expect(messages[0]?.phase).toBeUndefined();
});
it("keeps canceled draft parts with exact opaque indices and explicit unfinished state", () => {
  const messages = project([{ type: "itemStarted", turnId, itemId, kind: { type: "reasoningSummary", parts: [] } },
    { type: "reasoningSummaryDelta", turnId, itemId, summaryIndex: "-9223372036854775808", text: "first" },
    { type: "reasoningSummaryDelta", turnId, itemId, summaryIndex: "9223372036854775807", text: "second" },
    { type: "turnFinished", turnId, outcome: "canceled" }]);
  expect(messages[0]).toMatchObject({ kind: "reasoning-summary", interrupted: true, streaming: false, content: "firstsecond", reasoningSummaryParts: [
    { index: "-9223372036854775808", text: "first" }, { index: "9223372036854775807", text: "second" }] });
});
it("hides empty public summaries even if native metadata contains tempting text", () => {
  expect(project([{ type: "itemAccepted", turnId, itemId, kind: { type: "reasoningSummary", parts: [] }, text: "", phase: null,
    source: { attemptId, ordinal: 0, providerItemId: "reasoning", providerDataJson: '{"summary":[{"text":"not common text"}]}' } }])).toEqual([]);
});
