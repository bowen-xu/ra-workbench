import { decodeAgentSqlImmediateFailure } from "@research-assistant/core/agent";
import { diagnosticError } from "../../services/diagnosticError";
import {writingQueueCancellationDomain, type WritingQueueCancellationSnapshot} from "./writingQueueCancellation";

export function writingQueueCancellationMessage(state: WritingQueueCancellationSnapshot | undefined): string {
  const intent = state?.intent;
  const local = state?.cause instanceof Error && ["QUEUE_CANCEL_RECOVERY_INVALID", "QUEUE_CANCEL_RECOVERY_CONFLICT"].includes(state.cause.message)
    ? state.cause.message : state?.cause instanceof Error && state.cause.message === "QUEUE_CANCEL_PREVIEW_CHANGED" ? state.cause.message
      : state?.phase === "localError" ? "QUEUE_CANCEL_PERSISTENCE_FAILED" : null;
  const failure = decodeAgentSqlImmediateFailure(state?.cause), safe = diagnosticError(state?.cause);
  const domain = writingQueueCancellationDomain(state?.cause);
  const detail = state?.status?.type === "unconfirmed" ? state.status.detail : failure?.diagnostic;
  const synthetic = typeof state?.cause === "string" && ["AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST", "AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT", "AGENT_PREVIEW_SCENARIO_CANCELED"].includes(state.cause) ? state.cause : null;
  const code = local ?? synthetic ?? (domain || state?.definitiveConflict ? "DATA_SQL_DOMAIN" : failure?.code) ?? safe?.code ?? "QUEUE_CANCEL_UNCONFIRMED";
  const fields = [`code=${code}`, ...(intent ? [`C=${intent.conversationId}`, `R=${intent.previewRequestId}`, `I=${intent.inputId}`, `operation=${intent.requestId}`] : [])];
  if (detail) for (const field of ["phase", "epoch", "sqlFence", "sqliteCode", "sqliteExtendedCode", "safeCause"] as const)
    if (detail[field] != null) fields.push(`${field}=${detail[field]}`);
  if (typeof safe.httpStatus === "number") fields.push(`HTTP=${safe.httpStatus}`);
  if (domain || state?.definitiveConflict) fields.push(`reason=${domain?.safeReason ?? "conflict"}`);
  if (typeof safe.cause === "string" && state?.cause != null) fields.push(`cause=${safe.cause}`);
  return `${state?.knownCommitted ? "取消已提交，本机恢复记录尚待清理。" : domain || state?.definitiveConflict ? "原取消请求被拒绝，原消息与操作身份保留。" : state?.phase === "localError" ? "取消意图尚未可靠保存，未发送新的取消请求。" : "原取消结果尚待核对，队列消息保留。"} [${fields.join("；")}]`;
}
