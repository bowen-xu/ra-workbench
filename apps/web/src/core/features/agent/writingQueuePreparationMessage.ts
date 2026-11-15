import {writingPreparationSourceMessage} from "../../services/aiAssistanceErrorMessage";
import {decodeAgentSqlImmediateFailure} from "@research-assistant/core/agent";
import {diagnosticError} from "../../services/diagnosticError";
import {writingQueueCancellationDomain} from "./writingQueueCancellation";
import type {WritingQueuePreparationSnapshot} from "./writingQueuePreparation";
import {writingDispatchAdmissionMessage} from "./writingDispatchAdmission";

export function writingQueuePreparationMessage(state: WritingQueuePreparationSnapshot | undefined): string {
  const admission = writingDispatchAdmissionMessage(state?.cause); if (admission) return admission;
  const intent = state?.intent, failure = decodeAgentSqlImmediateFailure(state?.cause), safe = diagnosticError(state?.cause);
  const domain = writingQueueCancellationDomain(state?.cause), detail = state?.status?.type === "unconfirmed" ? state.status.detail : failure?.diagnostic;
  const message = state?.cause instanceof Error ? state.cause.message : state?.cause;
  const original = typeof message === "string" && ["QUEUE_PREPARE_RECOVERY_INVALID", "QUEUE_PREPARE_RECOVERY_CONFLICT", "QUEUE_PREPARE_PROOF_MISMATCH",
    "QUEUE_PREPARE_CLAIM_CHANGED", "QUEUE_PREPARE_INPUT_CHANGED", "QUEUE_PREPARE_QUEUE_HOLD_FAILED", "AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST", "AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT", "AGENT_PREVIEW_SCENARIO_CANCELED"].includes(message) ? message : null;
  const code = original ?? (state?.phase === "localError" ? "QUEUE_PREPARE_PERSISTENCE_FAILED" : null)
    ?? (domain ? "DATA_SQL_DOMAIN" : failure?.code) ?? safe.code ?? "QUEUE_PREPARE_UNCONFIRMED";
  const fields = [`code=${code}`, ...(intent ? [`C=${intent.submission.conversationId}`, `R=${intent.submission.requestId}`, `I=${intent.inputId}`, `operation=${intent.submission.requestId}`] : [])];
  if (detail) for (const field of ["phase", "epoch", "sqlFence", "sqliteCode", "sqliteExtendedCode", "safeCause"] as const)
    if (detail[field] != null) fields.push(`${field}=${detail[field]}`);
  if (typeof safe.httpStatus === "number") fields.push(`HTTP=${safe.httpStatus}`);
  if (domain) fields.push(`reason=${domain.safeReason}`);
  if (safe.cause && state?.cause != null) fields.push(`cause=${safe.cause}`);
  const source = writingPreparationSourceMessage(state?.cause);
  return `${state?.knownCommitted ? state.phase === "ready" ? "原准备已提交，当前队首权限尚待核对。" : "原准备已提交，本机恢复记录尚待清理。"
    : state?.definitiveRejection ? "原准备已被明确拒绝，可核对后返回队列。" : "队列准备结果尚待核对，完整原输入和操作身份已保留。"} [${fields.join("；")}]${source === undefined ? "" : `\n${source}`}`;
}
