import { describe, expect, it } from "vitest";
import {
  applyAgentEvents, createAgentConversationProjection, decodeAgentEvent,
  type AgentConversationProjection, type AgentEvent, type AgentEventPayload,
} from "@research-assistant/core/agent";
import { projectConversationMessages } from "./projectConversationMessages";

const id = (value: number): string => `00000000-0000-0000-0000-${value.toString(16).padStart(12, "0")}`;
const conversationId = id(1);
const model = { providerId: "fixture", modelId: "compact", revision: null };
function ledger() {
  const facts: AgentEvent[] = [];
  let state = createAgentConversationProjection(conversationId);
  function emit(payload: AgentEventPayload): AgentConversationProjection {
    const event = decodeAgentEvent({ schemaVersion: 1, conversationId, requestId: id(4), seq: facts.length + 1, timestampMs: facts.length + 1, payload });
    state = applyAgentEvents(state, [event]);
    facts.push(event);
    return state;
  }
  return {
    facts, emit, state: () => state,
    start: (turn: number, compact = true) => emit({ type: "turnStarted", turnId: id(turn), contextSnapshotId: id(turn + 100), model, taskKind: compact ? "compact" : "regular" }),
    activity: (turn: number, operation: number) => emit({ type: "itemStarted", turnId: id(turn), itemId: id(operation), kind: { type: "contextCompaction" } }),
    finish: (turn: number, outcome: "completed" | "failed" | "canceled" | "interrupted") => emit({ type: "turnFinished", turnId: id(turn), outcome }),
  };
}
function activities(state: AgentConversationProjection) {
  return projectConversationMessages(state).filter(message => message.kind === "context-compaction");
}

describe("durable context compaction transcript projection", () => {
  it("shows a manual operation without fabricating user text, installation or task completion from usage", () => {
    const log = ledger();
    log.start(10);
    log.activity(10, 11);
    log.emit({ type: "phaseChanged", turnId: id(10), phase: "runningModel" });
    log.emit({ type: "modelRequestStarted", turnId: id(10), attemptId: id(12), model });
    log.emit({ type: "modelCompactionFinished", turnId: id(10), attemptId: id(12), inputTokens: 80, outputTokens: 4 });
    log.emit({ type: "usage", turnId: id(10), inputTokens: 80, outputTokens: 4 });
    const messages = projectConversationMessages(log.state());
    expect(messages).toHaveLength(1);
    expect(messages[0]).toMatchObject({ id: `${id(10)}/${id(11)}`, turnId: id(10), conversationId,
      kind: "context-compaction", role: "assistant", content: "", streaming: true, turnFinished: false,
      compaction: { operationId: id(11), windowId: null } });
    expect(messages[0]).not.toHaveProperty("turnOutcome");
    expect(messages[0]?.rawData).toMatchObject({ itemId: id(11), kind: { type: "contextCompaction" }, completed: false });
  });

  it("keeps an installed window independent of a later persisted task diagnostic", () => {
    const log = ledger();
    log.start(10);
    log.activity(10, 11);
    const installed = log.emit({ type: "itemCompleted", turnId: id(10), itemId: id(11) });
    expect(activities(installed)[0]).toMatchObject({ content: "", streaming: false, turnFinished: false,
      compaction: { operationId: id(11), windowId: id(11) } });
    expect(activities(installed)[0]).not.toHaveProperty("turnOutcome");
    log.emit({ type: "turnFailed", turnId: id(10), error: "later task failed", category: "admission" });
    log.finish(10, "failed");
    expect(activities(log.state())[0]).toMatchObject({ content: "", turnFinished: true, turnOutcome: "failed",
      compaction: { operationId: id(11), windowId: id(11) } });
    expect(projectConversationMessages(log.state()).at(-1)?.failure).toBe(true);
    expect(activities(installed)[0]?.turnFinished).toBe(false);
  });

  it("retains canceled and recovered unfinished operations without marking a window installed", () => {
    for (const outcome of ["canceled", "interrupted"] as const) {
      const log = ledger();
      log.start(10);
      log.activity(10, 11);
      log.finish(10, outcome);
      const message = activities(log.state())[0];
      expect(message).toMatchObject({ content: "", streaming: false, interrupted: true, turnFinished: true, turnOutcome: outcome,
        compaction: { operationId: id(11), windowId: null } });
      const replay = applyAgentEvents(createAgentConversationProjection(conversationId), log.facts);
      expect(projectConversationMessages(replay)).toEqual(projectConversationMessages(log.state()));
    }
  });

  it("places automatic operations at their durable item and turn positions", () => {
    const log = ledger();
    log.start(10, false);
    log.emit({ type: "itemStarted", turnId: id(10), itemId: id(11), kind: { type: "userMessage" } });
    log.emit({ type: "textDelta", turnId: id(10), itemId: id(11), text: "user" });
    log.emit({ type: "itemCompleted", turnId: id(10), itemId: id(11) });
    log.activity(10, 12);
    log.emit({ type: "itemCompleted", turnId: id(10), itemId: id(12) });
    log.emit({ type: "itemAccepted", turnId: id(10), itemId: id(13), kind: { type: "assistantMessage" }, text: "answer", phase: null, source: null });
    log.finish(10, "completed");
    log.start(20);
    log.activity(20, 21);
    log.emit({ type: "itemCompleted", turnId: id(20), itemId: id(21) });
    log.finish(20, "completed");
    // The explicit turn order is authoritative even if a cache rebuild changes
    // Map insertion order; itemOrder remains authoritative inside each turn.
    const state = { ...log.state(), turns: new Map([...log.state().turns].reverse()) };
    expect(projectConversationMessages(state).map(message => [message.turnId, message.kind ?? "text", message.content])).toEqual([
      [id(10), "text", "user"], [id(10), "context-compaction", ""], [id(10), "text", "answer"], [id(20), "context-compaction", ""],
    ]);
    expect(activities(state).map(message => message.compaction?.operationId)).toEqual([id(12), id(21)]);
  });

  it("removes rolled-back activities through the common projection while keeping every source fact", () => {
    const log = ledger();
    log.start(10, false);
    log.emit({ type: "itemStarted", turnId: id(10), itemId: id(11), kind: { type: "userMessage" } });
    log.emit({ type: "itemCompleted", turnId: id(10), itemId: id(11) });
    log.finish(10, "completed");
    log.start(20);
    log.activity(20, 21);
    log.emit({ type: "itemCompleted", turnId: id(20), itemId: id(21) });
    const before = log.finish(20, "completed");
    const prefix = [...log.facts];
    log.emit({ type: "conversationRolledBack", turnId: id(20), userTurns: 1 });
    expect(projectConversationMessages(log.state())).toEqual([]);
    expect(activities(before)).toHaveLength(1);
    expect(log.facts.slice(0, prefix.length)).toEqual(prefix);
    expect(projectConversationMessages(applyAgentEvents(createAgentConversationProjection(conversationId), log.facts))).toEqual([]);
    log.start(30);
    log.activity(30, 31);
    expect(activities(log.state()).map(message => message.compaction?.operationId)).toEqual([id(31)]);
  });
});
