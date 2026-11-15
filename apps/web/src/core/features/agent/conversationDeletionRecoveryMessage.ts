import type { ConversationDeletionSnapshot } from "./conversationDeletionController";
import { conversationDeletionDiagnostic } from "./conversationDeletionDiagnostic";
import { agentDeletionErrors } from "@research-assistant/core/agent";
import { diagnosticError } from "../../services/diagnosticError";

const reasons: Record<string, string> = {
  busy: "此对话仍有运行或恢复任务，请等待完成或显式停止后重试。",
  conflict: "对话已发生变化。关闭此窗口后会自动核对列表，请重新确认。",
  notArchived: "对话已恢复，只能删除已归档对话。",
  notFound: "找不到此对话，无法确认本次删除成功。",
  tooLarge: "历史超过当前删除校验上限，尚未删除。",
  corrupt: "历史损坏或版本不受支持，无法删除。",
  invalidInput: "删除确认内容无效，请重新核对。",
  branchHistory: "此对话仍有关联分支历史，无法删除。",
  proposalApplications: "此对话仍有关联待处理修改，无法删除。",
  executionWorkspace: "此对话仍有关联执行副本，无法删除。",
};
export function deletionReasonMessage(reason: string): string | undefined { return reasons[reason]; }
const local: Record<string, string> = {
  DELETION_RECOVERY_BATCH_RESERVED: "此对话的旧清空操作尚未结束，请稍后重试。",
  DELETION_RECOVERY_LOCK_BUSY: "其他窗口正在处理此对话的删除，请稍后核对。",
  DELETION_RECOVERY_LOCK_UNAVAILABLE: "当前环境不支持跨窗口保护，尚未发送删除。",
  DELETION_RECOVERY_STORAGE_STALE: "预览数据已被其他窗口更新，请保全输入并重开预览后核对。",
  DELETION_RECOVERY_STORAGE_CHECK_UNAVAILABLE: "当前存储无法核对其他窗口的更新，尚未发送删除。",
  DELETION_RECOVERY_INVALID: "本机删除恢复记录损坏，原记录已保留，无法继续删除。",
  DELETION_RECOVERY_CONFLICT: "本机删除恢复记录已变化，原记录已保留，请重新打开核对。",
  DELETION_RECOVERY_UNAVAILABLE: "本机删除恢复记录尚未可靠保存，请重试本机恢复记录。",
  DELETION_RECOVERY_EPOCH_CHANGED: "原删除请求属于另一套数据，请返回原数据环境核对。",
  DELETION_RECOVERY_PROOF_MISSING: "此前已确认删除，但当前缺少原提交证明；原请求已保留，不会再次删除。",
  DELETION_STATUS_MISMATCH: "原删除查询返回了其他操作的结果，未采用该结果，请核对原请求。",
  DELETION_RECEIPT_MISMATCH: "删除返回的身份与原请求不符，原请求已保留，请核对。",
};
export function deletionRecoveryMessage(state: ConversationDeletionSnapshot): string {
  const {record} = state, cause = state.localIssue ?? state.cause;
  const safe = diagnosticError(cause);
  const http = typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599
    ? `；HTTP ${safe.httpStatus}` : "";
  const code = cause instanceof Error ? cause.message : null;
  if (code && local[code]) return `${local[code]}（${code}${record ? `；operation=${record.intent.requestId}` : ""}${http}）`;
  if (record) {
    const diagnostic = conversationDeletionDiagnostic(cause, record.intent.requestId, {force: record.intent.force});
    if (diagnostic) return diagnostic;
  }
  if (state.phase === "committed") return "原删除已确认；请完成原会话的本机草稿与队列收尾，不会再次删除。";
  if (state.phase === "retryable") return record?.stage === "pending" ? "原删除请求已可靠保存，尚未发送。可继续原请求。"
    : "尚无原 SQL 提交回执；文件清理可能已发生。可沿原请求显式重试。";
  const identity = record ? `；operation=${record.intent.requestId}` : "";
  if (typeof cause === "string" && reasons[cause]) return `${reasons[cause]}（code=${cause}${identity}）`;
  const previewCode = typeof safe.code === "string" && /^(?:AGENT_PREVIEW_[A-Z_]+|agent_preview_[a-z_]+)$/u.test(safe.code) ? safe.code : null;
  const reason = typeof cause === "string" && agentDeletionErrors.some(error => error === cause) ? cause : previewCode ?? "DELETION_RECOVERY_UNCONFIRMED";
  return `删除失败或结果不明，请先核对原请求。核对不会重复删除或启动模型。（code=${reason}${identity}${http}）`;
}
export function deletionRecoveryAction(state: ConversationDeletionSnapshot): string | undefined {
  if (!state.record) return state.localIssue ? "重试" : undefined;
  if (state.localIssue) return "重试本机恢复记录";
  if (state.phase === "committed") return "完成本机收尾";
  if (state.phase === "retryable") return "重试原删除";
  if (state.phase === "unknown") return "核对原删除";
  return undefined;
}
