import type { AgentTurnProjection } from "@research-assistant/core/agent";
import { agentFailureCode } from "./agentFailureReason";

/** Explicit allowlist: never serialize messages, call arguments or tool results. */
export function agentFailureDiagnostics(conversationId: string, turnId: string, turn: AgentTurnProjection): string {
  return JSON.stringify({
    schemaVersion: 1,
    appVersion: typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "unknown",
    conversationId,
    turnId,
    contextSnapshotId: turn.contextSnapshotId,
    model: turn.model,
    phase: turn.phase,
    outcome: turn.outcome,
    startedAtMs: turn.startedAtMs,
    finishedAtMs: turn.finishedAtMs,
    failure: turn.failure,
    ...(turn.failure?.error ? {failureCode: agentFailureCode(turn.failure.error)} : {}),
    pendingToolReceipts: [...(turn.pendingToolReceipts ?? [])],
    attempts: turn.attemptOrder.flatMap(attemptId => {
      const attempt = turn.attempts.get(attemptId);
      return attempt ? [{ attemptId, requestedModel: attempt.requestedModel, actualModel: attempt.actualModel,
        responseId: attempt.responseId, outcome: attempt.outcome, error: attempt.error,
        retryable: attempt.retryable, failureCategory: attempt.failureCategory }] : [];
    }),
    toolCalls: turn.itemOrder.flatMap(itemId => {
      const item = turn.items.get(itemId);
      return item?.kind.type === "toolCall" ? [{ itemId, name: item.kind.name,
        callId: item.kind.callId, providerCallId: item.kind.providerCallId,
        completed: item.completed, interrupted: item.interrupted }] : [];
    }),
  }, null, 2);
}
