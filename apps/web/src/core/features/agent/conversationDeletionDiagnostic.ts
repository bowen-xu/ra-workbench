import { decodeAgentSqlImmediateFailure } from "@research-assistant/core/agent";
import { aiAssistanceErrorMessage } from "../../services/aiAssistanceErrorMessage";
import { diagnosticError } from "../../services/diagnosticError";

/** Only the original operation's allowlisted storage evidence may be displayed. */
export function conversationDeletionDiagnostic(error: unknown, requestId: string, options: {readonly force?: boolean} = {}): string | null {
  const failure = decodeAgentSqlImmediateFailure(error);
  if (!failure || failure.diagnostic?.operationId && failure.diagnostic.operationId.toLowerCase() !== requestId.toLowerCase()) return null;
  const safe = diagnosticError(error);
  const http = typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599
    ? `（HTTP ${safe.httpStatus}）` : "";
  const cleanup = failure.code === "DATA_OPERATION_PARTIAL" && failure.diagnostic?.safeCause?.endsWith("_after_workspace_cleanup")
    ? "工作区文件清理已完成，对话元数据尚未删除。" : options.force && failure.code !== "DATA_OPERATION_PARTIAL" ? "文件清理可能已经执行。" : "";
  return `保留原删除请求，请核对原操作。${cleanup}${aiAssistanceErrorMessage(error)}${http}`;
}
export function conversationDeletionStorageBlocked(error: unknown, requestId: string): boolean {
  const failure = decodeAgentSqlImmediateFailure(error);
  if (!failure || failure.diagnostic?.operationId && failure.diagnostic.operationId.toLowerCase() !== requestId.toLowerCase()) return false;
  return ["DATA_SQL_INVALID", "DATA_SQL_UNMANAGED", "DATA_SQL_OPERATION_CONFLICT", "DATA_SQL_CORRUPT"].includes(failure.code);
}
