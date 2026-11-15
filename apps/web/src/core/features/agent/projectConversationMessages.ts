import { agentTurnFailureReason, agentTurnReconnecting, agentTurnReconnectAttempt, agentTurnUnconfirmedReceipts, agentTurnUnconfirmedChildCreations, agentReasoningSummaryParts, type AgentConversationProjection, type AgentTurnProjection } from "@research-assistant/core/agent";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import { agentFailureCode, agentFailureReason } from "./agentFailureReason";
import { agentFailureDiagnostics } from "./agentFailureDiagnostics";
import { childCreationFailureStage } from "./childCreationFailure";

function messageCreatedAt(timestampMs: number | undefined): string | undefined {
  if (timestampMs === undefined) return undefined;
  const date = new Date(timestampMs);
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}

const turnMessages = new WeakMap<AgentTurnProjection, {conversationId: string; idle: boolean; messages: readonly WritingAgentMessage[]}>();

/** Immutable turns retain their display identities across unrelated deltas.
 * Weak keys do not retain closed conversations or evicted authority snapshots. */
export function projectConversationMessages(projection: AgentConversationProjection | null): readonly WritingAgentMessage[] {
  if (!projection) return [];
  return projection.turnOrder.flatMap(turnId => {
    const turn = projection.turns.get(turnId);
    if (!turn) return [];
    const idle = projection.activeTurn === null && !!turn.childCreationIntents?.size;
    const cached = turnMessages.get(turn);
    if (cached?.conversationId === projection.conversationId && cached.idle === idle) return cached.messages;
    const messages = projectTurnMessages({...projection, turnOrder: [turnId]});
    turnMessages.set(turn, {conversationId: projection.conversationId, idle, messages});
    return messages;
  });
}

function projectTurnMessages(projection: AgentConversationProjection): readonly WritingAgentMessage[] {
  return (projection?.turnOrder ?? []).flatMap(turnId => {
    const turn = projection!.turns.get(turnId);
    if (!turn) return [];
    const reconnectHistory = turn.attemptOrder.flatMap((attemptId, attemptIndex) => {
      const attempt = turn.attempts.get(attemptId);
      return attempt?.retryable && attempt.error ? [{ attemptId, reason: agentFailureReason(attempt.error), attemptIndex,
        ...(attempt.failedSeq === undefined ? {} : {failedSeq: attempt.failedSeq}) }] : [];
    });
    const toolNames = new Map([...turn.items.values()].flatMap((item) => item.kind.type === "toolCall"
      ? [[item.kind.callId, item.kind.name] as const] : []));
    const latestAssistant = [...turn.itemOrder].reverse().find(id => turn.items.get(id)?.kind.type === "assistantMessage");
    const latestAttempt = latestAssistant ? turn.items.get(latestAssistant)?.attemptId : undefined;
    const presentations = new Map<string, { key: string; text: string; phase: WritingAgentMessage["phase"] }>();
    let previous: { key: string; text: string; phase: WritingAgentMessage["phase"]; interrupted: boolean; attemptId: string | undefined } | undefined;
    for (const itemId of turn.itemOrder) {
      const item = turn.items.get(itemId);
      if (item?.kind.type !== "assistantMessage") continue;
      // Retry samples share only a display identity. Compare with the latest
      // displayed draft, so a later sample cannot revive an older divergent one.
      const replacedDraft = previous?.interrupted && previous.attemptId !== item.attemptId ? previous : undefined;
      const key = replacedDraft?.key ?? `${turnId}/${itemId}`;
      const text = replacedDraft && !item.completed && replacedDraft.text.startsWith(item.text)
        ? replacedDraft.text : item.text;
      // Until the retry declares its phase, retain the old display slot too.
      const phase = item.phase ?? replacedDraft?.phase;
      presentations.set(itemId, { key, text, phase });
      previous = { key, text, phase, interrupted: item.interrupted, attemptId: item.attemptId };
    }
    const items: WritingAgentMessage[] = turn.itemOrder.flatMap((itemId) => {
      const item = turn.items.get(itemId);
      if (!item) return [];
      // Provider continuation material is a history fact, never an empty chat
      // message or a source of text for the generic transcript.
      if (item.kind.type === "providerNative") return [];
      const summary = item.kind.type === "reasoningSummary" ? item.completed
        ? agentReasoningSummaryParts(item.text, item.kind.parts)
        : [...(item.reasoningSummaryDraft ?? [])].map(([index, text]) => ({ index, text })) : undefined;
      if (summary && !summary.some(part => part.text.trim())) return [];
      // A new sample replaces the failed draft; keep completed messages/tools.
      if (item.kind.type === "assistantMessage" && item.interrupted
        && ((item.attemptId === undefined ? itemId !== latestAssistant : item.attemptId !== latestAttempt) || turn.outcome === "completed")) return [];
      // A completed assistant item with no text is the protocol envelope for a
      // tool-only round (or an empty retry attempt), not a user-visible message.
      if (item.kind.type === "assistantMessage" && item.completed && !item.interrupted && item.text.trim() === "") return [];
      const suffix = item.kind.type !== "reasoningSummary" && item.kind.type !== "contextCompaction" && item.interrupted && turn.outcome !== null ? "\n\n（未完成）" : "";
      const presentation = presentations.get(itemId);
      const text = presentation?.text ?? item.text;
      const kind = item.kind.type === "toolCall" ? "tool-call" as const : item.kind.type === "toolResult" ? "tool-result" as const
        : item.kind.type === "reasoningSummary" ? "reasoning-summary" as const
        : item.kind.type === "contextCompaction" ? "context-compaction" as const : undefined;
      const createdAt = messageCreatedAt(item.createdAtMs);
      const attemptId = item.attemptId ?? item.source?.attemptId;
      const attemptIndex = attemptId === undefined ? -1 : turn.attemptOrder.indexOf(attemptId);
      return [{
        id: `${turnId}/${itemId}`,
        ...(presentation ? { renderKey: presentation.key } : {}),
        conversationId: projection!.conversationId,
        rawData: { turnId, itemId, ...item },
        turnId,
        ...(item.createdSeq === undefined ? {} : {createdSeq: item.createdSeq}),
        ...(attemptIndex < 0 ? {} : {attemptIndex}),
        taskKind: turn.taskKind ?? "regular",
        turnFinished: turn.outcome !== null,
        reconnecting:agentTurnReconnecting(turn),
        reconnectAttempt: agentTurnReconnectAttempt(turn),
        reconnectHistory,
        ...(turn.outcome === null ? {} : { turnOutcome: turn.outcome }),
        ...(turn.startedAtMs === undefined ? {} : { turnStartedAtMs: turn.startedAtMs }),
        ...(turn.finishedAtMs === undefined ? {} : { turnFinishedAtMs: turn.finishedAtMs }),
        role: item.kind.type === "userMessage" ? "user" as const : "assistant" as const,
        content: text + suffix,
        ...(createdAt === undefined ? {} : { createdAt }),
        ...(presentation?.phase !== undefined ? { phase: presentation.phase } : {}),
        ...(kind ? { kind } : {}),
        ...(item.kind.type === "contextCompaction" ? { compaction: { operationId: itemId, windowId: item.completed ? itemId : null } } : {}),
        ...(summary ? { reasoningSummaryParts: summary } : {}),
        ...(item.kind.type === "toolCall" ? { toolName: item.kind.name, toolCallId: item.kind.callId } : {}),
        ...(item.kind.type === "toolResult" ? {
          toolCallId: item.kind.callId,
          ...(toolNames.has(item.kind.callId) ? { toolName: toolNames.get(item.kind.callId)! } : {}),
          ...(item.kind.isError === true ? { toolError: true } : {}),
        } : {}),
        streaming: !item.completed && !item.interrupted,
        interrupted: item.interrupted,
      }];
    });
    if (!items.length && reconnectHistory.length) items.push({
      id: `${turnId}/reconnect-anchor`, turnId, conversationId: projection!.conversationId, role: "assistant", content: "",
      kind: "reconnect-anchor", reconnectHistory, reconnecting: agentTurnReconnecting(turn),
      reconnectAttempt: agentTurnReconnectAttempt(turn), turnFinished: turn.outcome !== null,
      ...(turn.outcome === null ? {} : {turnOutcome: turn.outcome}),
      ...(turn.startedAtMs === undefined ? {} : {turnStartedAtMs: turn.startedAtMs}),
      ...(turn.finishedAtMs === undefined ? {} : {turnFinishedAtMs: turn.finishedAtMs}),
    });
    const receipts = agentTurnUnconfirmedReceipts(turn);
    const withReceiptWarning: WritingAgentMessage[] = receipts.length === 0 ? items : [...items, {
      id: `${turnId}/receipt-recovery`, turnId, conversationId: projection!.conversationId,
      role: "assistant", failure: true, turnFinished: true, turnOutcome: turn.outcome!,
      unconfirmedToolReceiptIds: receipts,
      content: `工具结果的原操作回执尚未确认，不能继续此对话或回滚；请先核对原操作，不要重复执行。\n\n轮次：${turnId}\n待确认调用：${receipts.join("、")}`,
      failureDiagnosticText: agentFailureDiagnostics(projection!.conversationId, turnId, turn),
    }];
    const childCreations = projection!.activeTurn === null ? agentTurnUnconfirmedChildCreations(turn) : [];
    const failureReason = agentTurnFailureReason(turn);
    if (childCreations.length) withReceiptWarning.push({
      id: `${turnId}/child-creation-recovery`, turnId, conversationId: projection!.conversationId,
      role: "assistant", failure: true, turnFinished: true, turnOutcome: turn.outcome!,
      content: "子任务的原创建操作需要核对；请勿重复执行。",
      failureDiagnosticText: agentFailureDiagnostics(projection!.conversationId, turnId, turn),
      childCreationRecovery: childCreations.map(item => {
        const failureStage = childCreationFailureStage(failureReason, item, projection!.conversationId, turnId);
        return {...item, conversationId: projection!.conversationId, turnId, watermark: projection!.seq,
          ...(failureStage ? {failureStage} : {})};
      }),
    });
    const failureCode = failureReason === undefined ? undefined : agentFailureCode(failureReason);
    const failedAttempt = [...turn.attemptOrder].reverse().find(id => turn.attempts.get(id)?.error === failureReason);
    // The exact saved child failure is already presented with its recovery action
    // and full turn diagnostics. Do not hide an unrelated failure in this turn.
    const childFailureShown = childCreations.some(child => childCreationFailureStage(failureReason, child, projection!.conversationId, turnId));
    return failureReason === undefined || childFailureShown ? withReceiptWarning : [...withReceiptWarning, {
      id: `${turnId}/failure`, turnId, taskKind: turn.taskKind ?? "regular", turnFinished: turn.outcome !== null,
      conversationId: projection!.conversationId,
      failureDiagnosticText: agentFailureDiagnostics(projection!.conversationId, turnId, turn),
      reconnectHistory,
      ...(turn.outcome === null ? {} : { turnOutcome: turn.outcome }),
      ...(turn.startedAtMs === undefined ? {} : { turnStartedAtMs: turn.startedAtMs }),
      ...(turn.finishedAtMs === undefined ? {} : { turnFinishedAtMs: turn.finishedAtMs }),
      rawData: { turnId, attemptId: failedAttempt, failureCode, failure: turn.failure, reason: failureReason, model: turn.model },
      role: "assistant" as const, content: `Agent 请求失败：${failureCode === "agent_failure" ? "本轮未能完成，请复制诊断信息排查。" : agentFailureReason(failureReason)}`, failure: true,
    }];
  });
}

