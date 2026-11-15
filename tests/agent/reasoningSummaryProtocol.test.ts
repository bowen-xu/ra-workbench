import { readFileSync } from "node:fs";
import { expect, it, vi } from "vitest";
import { agentReasoningSummaryParts, applyAgentEvents, createAgentConversationProjection, decodeAgentEvent } from "@research-assistant/core/agent";
import { createAgentConversationExportGateway, createAgentSessionGateway, startAgentHistoryFeed } from "@research-assistant/data/agent";
import { projectConversationMessages } from "../../apps/web/src/core/features/agent/projectConversationMessages";

const raw: unknown = JSON.parse(readFileSync(new URL("../fixtures/agent-reasoning-summary-v1.json", import.meta.url), "utf8"));
if (!Array.isArray(raw)) throw new Error("Expected shared public-summary AgentEvent[] fixture");
const events = raw.map(decodeAgentEvent), conversationId = events[0]!.conversationId;
const portable: unknown = JSON.parse(readFileSync(new URL("../fixtures/agent-reasoning-summary-export-v1.json", import.meta.url), "utf8"));

it("decodes the actual Rust summary protocol without rounding signed i64 progress identities", () => {
  expect(events).toEqual(raw);
  const indices = events.flatMap(event => event.payload.type === "reasoningSummaryDelta" || event.payload.type === "reasoningSummaryPartAdded" ? [event.payload.summaryIndex] : []);
  expect(indices).toContain("-9223372036854775808"); expect(indices).toContain("9223372036854775807");
});
it("decodes the actual Rust portable serializer, including interrupted signed-index parts without opaque state", async () => {
  const invoke = vi.fn(async () => portable);
  const exported = await createAgentConversationExportGateway(invoke).snapshot(conversationId);
  expect(exported).toEqual(portable);
  const items = exported.turns[0]!.items;
  expect(items).toHaveLength(4);
  const partial = items.find(item => item.interrupted)!;
  expect(partial.kind).toMatchObject({ type: "reasoningSummary", parts: [{ index: "-1", startByte: 0, lenByte: 11 }] });
  const summaries = items.flatMap(item => item.kind.type === "reasoningSummary" ? [
    { completed: item.completed, parts: agentReasoningSummaryParts(item.text, item.kind.parts) },
  ] : []);
  expect(summaries).toContainEqual({ completed: true, parts: [{ index: "9007199254740993", text: "稀疏" }] });
  expect(summaries).toContainEqual({ completed: false, parts: [{ index: "-1", text: "interrupted" }] });
  expect(items.every(item => !Object.hasOwn(item, "source") && !Object.hasOwn(item, "reasoningSummaryDraft"))).toBe(true);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_export_conversation_snapshot", { request: { conversationId } });
});
it("replays Rust events through the real history gateway and feed, retaining canonical and interrupted summary identity", async () => {
  const invoke = vi.fn(async (_command: string, args: Record<string, unknown>) => {
    const request = args.request as { afterSeq: number };
    const batch = events.slice(request.afterSeq, request.afterSeq + 5);
    const last = request.afterSeq + batch.length;
    return { schemaVersion: 1, conversationId, afterSeq: request.afterSeq, watermark: events.length, events: batch, nextAfterSeq: last < events.length ? last : null };
  });
  const gateway = createAgentSessionGateway(invoke);
  let state = createAgentConversationProjection(conversationId);
  const feed = startAgentHistoryFeed(gateway, { conversationId, afterSeq: 0, onBatch(batch) {
    state = applyAgentEvents(state, batch); if (state.seq === events.length) feed.stop();
  } });
  await expect(feed.done).resolves.toBe(events.length);
  expect(invoke.mock.calls.every(([command]) => command === "agent_read_history")).toBe(true);
  const turn = [...state.turns.values()][0]!;
  expect(turn.outcome).toBe("completed");
  const summaries = [...turn.items.values()].filter(item => item.kind.type === "reasoningSummary");
  expect(summaries.some(item => item.interrupted && !item.completed && (item.reasoningSummaryDraft?.size ?? 0) > 0)).toBe(true);
  expect(summaries.filter(item => item.completed).every(item => item.reasoningSummaryDraft === undefined)).toBe(true);
  const canonical = events.flatMap(event => event.payload.type === "itemAccepted" && event.payload.kind.type === "reasoningSummary" && event.payload.text ? [event.payload] : []);
  for (const item of canonical) expect(turn.items.get(item.itemId)).toMatchObject({ text: item.text, kind: item.kind, completed: true });
  const messages = projectConversationMessages(state);
  expect(messages.some(message => message.kind === "reasoning-summary" && message.interrupted)).toBe(true);
  expect(messages.every(message => message.kind === "reasoning-summary" && message.phase === undefined)).toBe(true);
  expect(messages.every(message => message.reasoningSummaryParts?.some(part => part.text.trim()))).toBe(true);
  expect(messages.map(message => message.id).length).toBe(new Set(messages.map(message => message.id)).size);
});
