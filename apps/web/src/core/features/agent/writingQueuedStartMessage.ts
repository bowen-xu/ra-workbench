import {decodeAgentSqlImmediateFailure} from "@research-assistant/core/agent";
import {diagnosticError} from "../../services/diagnosticError";
import type {WritingQueuedStartSnapshot} from "./writingQueuedStart";
import {writingDispatchAdmissionMessage} from "./writingDispatchAdmission";
export function writingQueuedStartMessage(state: WritingQueuedStartSnapshot): string {
  const admission = writingDispatchAdmissionMessage(state.cause); if (admission) return admission;
  const cause = state.cause, input = cause && typeof cause === "object" ? cause as Record<string, unknown> : {};
  const message = cause instanceof Error ? cause.message : typeof cause === "string" ? cause : null;
  const sql = decodeAgentSqlImmediateFailure(cause), safe = diagnosticError(cause);
  const code = typeof message === "string" && (/^QUEUE_START_[A-Z_]+$/u.test(message)
    || ["AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST", "AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT", "AGENT_PREVIEW_SCENARIO_CANCELED", "AGENT_PREVIEW_RAW_PLAN_REQUIRED"].includes(message)) ? message
    : sql?.code ?? (typeof safe.code === "string" ? safe.code : "QUEUE_START_UNCONFIRMED");
  const intent = state.intent, fields = [`code=${code}`, ...(intent ? [`C=${intent.start.conversationId}`, `R=${intent.start.requestId}`, `I=${intent.inputId}`,
    `Q=${intent.expectedRevision}`, `contextSnapshot=${intent.start.contextSnapshotId}`, `mode=${intent.start.mode}`] : []), ...(state.turnId ? [`T=${state.turnId}`] : [])];
  if (state.resumeCommand) fields.push(`resumeOperation=${state.resumeCommand.requestId}`, `resumeQ=${state.resumeCommand.expectedRevision}`);
  const detail = state.status?.type === "unconfirmed" ? state.status.detail : sql?.diagnostic;
  if (detail) for (const field of ["operationId", "phase", "epoch", "sqlFence", "sqliteCode", "sqliteExtendedCode", "safeCause"] as const)
    if (detail[field] != null) fields.push(`${field}=${detail[field]}`);
  if (typeof input.cancelOperation === "string" && /^[a-f\d-]{36}$/iu.test(input.cancelOperation)) fields.push(`cancelOperation=${input.cancelOperation}`);
  if (input.code === "QUEUE_START_CANCEL_UNCONFIRMED" && typeof input.cancelOperation === "string") {
    const cancellation = decodeAgentSqlImmediateFailure({code: "DATA_SQL_RECOVERY_REQUIRED", reason: input.detail})?.diagnostic;
    if (cancellation?.operationId?.toLowerCase() === input.cancelOperation.toLowerCase())
      for (const field of ["operationId", "phase", "epoch", "sqlFence", "sqliteCode", "sqliteExtendedCode", "safeCause"] as const)
        if (Object.hasOwn(cancellation, field)) fields.push(`cancel.${field}=${cancellation[field]}`);
  }
  if (input.code === "preparation" && ["conflict", "invalidInput", "unbound", "modelUnavailable", "unavailable"].includes(String(input.reason))) fields.push(`reason=${input.reason}`);
  if (typeof safe.httpStatus === "number") fields.push(`HTTP=${safe.httpStatus}`);
  return `${state.result ? "原任务已启动，本机恢复与观察仍待完成。" : state.resumeCommand ? "原队列恢复尚待核对，启动意图保持不变。"
    : state.queuePaused ? "原提交已接纳，队列当前暂停；明确继续才恢复队列。" : "原启动结果尚待核对，完整请求已保留。"} [${fields.join("；")}]`;
}
