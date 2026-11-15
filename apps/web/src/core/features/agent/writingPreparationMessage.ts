import {writingPreparationSourceMessage} from "../../services/aiAssistanceErrorMessage";
import {decodeAgentSqlImmediateFailure} from "@research-assistant/core/agent";
import {diagnosticError} from "../../services/diagnosticError";
import type {WritingPreparationSnapshot} from "./writingPreparation";
import {writingDispatchAdmissionMessage} from "./writingDispatchAdmission";
/** Display-only domain diagnostics. They are never evidence of non-admission. */
export function writingPreparationDomainFields(cause: unknown): readonly string[] | null {
  if (!cause || typeof cause !== "object" || !("code" in cause) || cause.code !== "storage" || !("reason" in cause)
    || !cause.reason || typeof cause.reason !== "object") return null;
  const row = cause.reason as Record<string, unknown>;
  if (row.code !== "DATA_SQL_DOMAIN") return null;
  const fields = ["code=DATA_SQL_DOMAIN"];
  for (const [key, label] of [["conversationId", "C"], ["manuscriptId", "M"], ["requestId", "R"], ["operationId", "operationId"]] as const) {
    const value = row[key]; if (typeof value === "string" && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu.test(value)) fields.push(`${label}=${value}`);
  }
  if (typeof row.reason === "string" && ["invalid conversation catalog input", "conversation not found", "conversation changed since it was read",
    "conversation has an unfinished turn", "conversation exceeds the validated history budget", "conversation catalog unavailable", "conversation metadata is corrupt"].includes(row.reason)) fields.push(`reason=${row.reason}`);
  return fields;
}
export function writingPreparationMessage(state: WritingPreparationSnapshot): string {
  const admission = writingDispatchAdmissionMessage(state.cause); if (admission) return admission;
  const sql = decodeAgentSqlImmediateFailure(state.cause), safe = diagnosticError(state.cause);
  const domain = writingPreparationDomainFields(state.cause);
  const value = typeof state.cause === "string" ? state.cause : state.cause instanceof Error ? state.cause.message : null;
  const code = value && (/^(?:WRITING_PREPARE|AGENT_DRAFT)_[A-Z_]+$/u.test(value) || ["invalidInput", "unbound", "contextUnavailable", "contextBudget", "modelUnavailable",
    "unsupportedModel", "confirmationMismatch", "conflict", "AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST", "AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT"].includes(value))
    ? value : domain ? "DATA_SQL_DOMAIN" : sql?.code ?? (typeof safe.code === "string" ? safe.code : "WRITING_PREPARE_UNCONFIRMED");
  const intent = state.intent, fields = [`code=${code}`, ...(intent ? [`C=${intent.conversationId}`, `M=${intent.context.manuscriptId}`, `R=${intent.requestId}`] : [])];
  if (domain) fields.push(...domain.filter(field => !fields.includes(field)));
  const detail = state.startStatus?.type === "unconfirmed" ? state.startStatus.detail : state.status?.type === "unconfirmed" ? state.status.detail : sql?.diagnostic;
  if (detail) for (const field of ["operationId", "phase", "epoch", "sqlFence", "sqliteCode", "sqliteExtendedCode", "safeCause"] as const)
    if (detail[field] != null) fields.push(`${field}=${detail[field]}`);
  if (state.result) fields.push(`contextSnapshot=${state.result.contextSnapshotId}`, `T=${state.result.turnId}`);
  if (typeof safe.httpStatus === "number") fields.push(`HTTP=${safe.httpStatus}`);
  const message = code === "AGENT_PREVIEW_OWNER_INTENT_MISSING" ? "原请求的归属尚无法确认，原请求和草稿已保留。"
    : state.start ? state.start.result ? "原启动已确认，本机草稿签收仍待完成。"
    : state.start.knownRejected ? "原启动被拒绝，完整请求及草稿已保留。" : "原启动结果尚待核对，完整请求及草稿已保留。"
    : state.knownCommitted ? "原准备已提交，工作副本或本机收尾仍待完成。" : "原准备结果尚待核对，完整请求已保留。";
  const source = writingPreparationSourceMessage(state.cause);
  return `${message} [${fields.join("；")}]${source === undefined ? "" : `\n${source}`}`;
}
