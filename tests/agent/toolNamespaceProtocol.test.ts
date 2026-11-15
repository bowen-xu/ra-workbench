import { readFileSync } from "node:fs";
import { expect, it, vi } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentConversationExport, decodeAgentEvent, type AgentExportItemKind, type AgentItemKind } from "@research-assistant/core/agent";
import { createAgentConversationExportGateway, createAgentSessionGateway, startAgentHistoryFeed } from "@research-assistant/data/agent";
import { projectConversationMessages } from "../../apps/web/src/core/features/agent/projectConversationMessages";

const raw: unknown = JSON.parse(readFileSync(new URL("../fixtures/agent-tool-namespace-v1.json", import.meta.url), "utf8"));
if (!Array.isArray(raw)) throw new Error("Expected shared namespace AgentEvent[] fixture");
const events = raw.map(decodeAgentEvent);
const conversationId = events[0]!.conversationId;
const namespaces = ["foreign::", "", "命名🦀".repeat(100)];

function portableKind(kind: AgentItemKind): AgentExportItemKind {
  if (kind.type === "toolCall") return { type: kind.type, callId: kind.callId, name: kind.name,
    ...(kind.namespace === undefined ? {} : { namespace: kind.namespace }) };
  if (kind.type === "toolResult") return { type: kind.type, callId: kind.callId, isError: kind.isError ?? null };
  throw new Error("Expected shared fixture tool kind");
}

it("decodes the exact Rust namespace protocol fixture without limiting or normalizing identity", () => {
  expect(events).toEqual(raw);
  const calls = events.flatMap(event => event.payload.type === "itemAccepted" && event.payload.kind.type === "toolCall" ? [event.payload.kind] : []);
  expect(calls.map(kind => kind.namespace)).toEqual(namespaces);
  expect(calls.every(kind => kind.name === "research_list_sources")).toBe(true);
  expect(Array.from(calls[2]!.namespace!).length).toBeGreaterThan(256);
});

it("replays the shared fixture through the production history gateway, feed and transcript projection", async () => {
  const invoke = vi.fn(async () => ({ schemaVersion: 1, conversationId, afterSeq: 0, watermark: events.length, nextAfterSeq: null, events: raw }));
  const gateway = createAgentSessionGateway(invoke);
  let projection = createAgentConversationProjection(conversationId);
  const feed = startAgentHistoryFeed(gateway, { conversationId, afterSeq: 0, onBatch(batch) {
    projection = applyAgentEvents(projection, batch); feed.stop();
  } });
  await expect(feed.done).resolves.toBe(8);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_read_history", { request: { conversationId, afterSeq: 0, limit: 100 } });
  const turn = [...projection.turns.values()][0]!;
  expect(turn.outcome).toBe("completed");
  expect([...turn.items.values()].filter(item => item.kind.type === "toolCall").map(item => item.kind)).toEqual(
    events.flatMap(event => event.payload.type === "itemAccepted" && event.payload.kind.type === "toolCall" ? [event.payload.kind] : []));
  const messages = projectConversationMessages(projection).filter(message => message.kind === "tool-call");
  expect(messages.map(message => message.rawData)).toEqual(namespaces.map(namespace => expect.objectContaining({ kind: expect.objectContaining({ namespace }) })));
  expect(messages.every(message => message.toolName === "research_list_sources" && message.content === "{")).toBe(true);
});

it("retains namespaces through the portable export gateway while omitting provider continuation identity", async () => {
  const projection = applyAgentEvents(createAgentConversationProjection(conversationId), events);
  const turn = [...projection.turns.values()][0]!;
  const wire = { format: "research-assistant.conversation", schemaVersion: 1, watermark: events.length,
    conversation: { id: conversationId, title: "Synthetic namespace fixture", archived: false,
      createdAt: "2026-10-26T00:00:00.000Z", updatedAt: "2026-10-26T00:00:00.000Z" },
    turns: [{ id: events[0]!.payload.turnId, contextSnapshotId: turn.contextSnapshotId, model: turn.model,
      phase: turn.phase, outcome: turn.outcome, inputTokens: turn.inputTokens, outputTokens: turn.outputTokens, attempts: [],
      items: turn.itemOrder.map(id => { const item = turn.items.get(id)!; return { id, kind: portableKind(item.kind), text: item.text,
        completed: item.completed, interrupted: item.interrupted }; }) }],
  };
  const invoke = vi.fn(async () => wire);
  const exported = await createAgentConversationExportGateway(invoke).snapshot(conversationId);
  expect(exported).toEqual(decodeAgentConversationExport(wire));
  const calls = exported.turns[0]!.items.filter(item => item.kind.type === "toolCall");
  expect(calls.map(item => item.kind)).toEqual(namespaces.map(namespace => expect.objectContaining({ namespace, name: "research_list_sources" })));
  expect(calls.every(item => !Object.hasOwn(item.kind, "providerCallId"))).toBe(true);
});
