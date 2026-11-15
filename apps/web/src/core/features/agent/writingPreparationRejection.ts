import {decodeAgentSqlImmediateFailure, type AgentSqlImmediateDiagnostic, type AgentSqlImmediateCode, type AgentWritingPrepareRequest} from "@research-assistant/core/agent";

export type WritingPreparationRejection = string | {readonly code: "storage"; readonly reason: {
  readonly conversationId: string; readonly manuscriptId: string; readonly requestId: string; readonly operationId: string;
  readonly code: AgentSqlImmediateCode; readonly reason: AgentSqlImmediateDiagnostic;
}};
const units = ["invalidInput", "unbound", "contextUnavailable", "contextBudget", "modelUnavailable", "unsupportedModel", "confirmationMismatch", "conflict"];
const compactUnits = [...units, "hostUnavailable", "historyChanged", "busy"];
const materialization = {
  DATA_SQL_OPERATION_CONFLICT: ["writing_context_changed", "writing_input_changed", "writing_instructions_unauthorized"],
  DATA_SQL_INVALID: ["writing_binding_unavailable", "writing_context_invalid", "writing_context_budget_exceeded", "writing_input_invalid", "writing_instructions_scope_invalid",
    "writing_instructions_budget_exceeded", "writing_instructions_encoding_invalid"],
};
function object(value: unknown): Record<string, unknown> | null {return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;}
/** Only the exact fresh invocation's documented rolled-back failures are
 * rejection evidence. This decoder neither clears an intent nor changes R/W. */
export function writingPreparationRejection(cause: unknown, intent: AgentWritingPrepareRequest, compact = false): WritingPreparationRejection | null {
  const row = object(cause), unit = typeof cause === "string" ? cause : compact ? row?.code : null;
  if (typeof unit === "string" && (compact ? compactUnits : units).includes(unit)) return unit;
  const outer = object(row?.reason), sql = decodeAgentSqlImmediateFailure(cause), detail = sql?.diagnostic;
  if (row?.code !== "storage" || !outer || !sql || !detail || outer.conversationId !== intent.conversationId
    || outer.manuscriptId !== intent.context.manuscriptId || outer.requestId !== intent.requestId || outer.operationId !== intent.requestId
    || detail.operationId !== intent.requestId || !detail.safeCause
    || ["operationId", "epoch", "sqlFence", "phase", "sqliteCode", "sqliteExtendedCode", "safeCause"].some(field => !Object.hasOwn(detail, field))) return null;
  const allowed = detail.phase === "writing-prepare-scope" && sql.code === "DATA_SQL_INVALID" && detail.safeCause === "writing_context_scope_invalid"
    || detail.phase === "writing-prepare-materialize" && (sql.code === "DATA_SQL_OPERATION_CONFLICT" || sql.code === "DATA_SQL_INVALID")
      && materialization[sql.code].includes(detail.safeCause);
  if (!allowed) return null;
  return {code: "storage", reason: {conversationId: intent.conversationId, manuscriptId: intent.context.manuscriptId,
    requestId: intent.requestId, operationId: intent.requestId, code: sql.code, reason: detail}};
}
