import { deletionBatchMemberSettled, type ConversationDeletionBatchSnapshot } from "./conversationDeletionBatchController";
import { deletionRecoveryMessage } from "./conversationDeletionRecoveryMessage";
import { conversationDeletionDiagnostic } from "./conversationDeletionDiagnostic";
import { agentDeletionErrors } from "@research-assistant/core/agent";
import { diagnosticError } from "../../services/diagnosticError";

export function deletionBatchCatalogMessage(cause: unknown): string {
  const safe = diagnosticError(cause);
  const code = typeof cause === "string" && agentDeletionErrors.some(value => value === cause) ? cause
    : typeof safe.code === "string" && /^(?:AGENT_PREVIEW_[A-Z_]+|agent_preview_[a-z_]+)$/u.test(safe.code) ? safe.code : null;
  const http = typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599
    ? `；HTTP ${safe.httpStatus}` : "";
  return `无法核对归档列表，未发送本批次删除。（DELETION_BATCH_CATALOG_UNAVAILABLE${code ? `；cause=${code}` : ""}${http}）请重试，核对完成后仍需确认清空。`;
}

export function deletionBatchAction(state: ConversationDeletionBatchSnapshot): "check" | "continue" | "finish" | "repair" {
  if (state.issue || Object.values(state.items).some(item => item.localIssue)) return "repair";
  const batch = state.batch;
  if (batch?.phase === "finishing") return "finish";
  if (!batch || batch.requests.some(request => !state.items[request.conversationId])) return "check";
  const items = batch.requests.map(request => state.items[request.conversationId]!);
  if (items.every(deletionBatchMemberSettled)) return "finish";
  return items.some(item => item.phase === "retryable" || item.phase === "rejected" && item.record?.reason === "busy") ? "continue" : "check";
}
const local: Record<string, string> = {
  DELETION_BATCH_INVALID: "本机清空批次记录损坏，原记录已保留。",
  DELETION_RECOVERY_INVALID: "本机原删除恢复记录损坏，原记录已保留。",
  DELETION_BATCH_CONFLICT: "原清空批次或其成员已被另一操作占用，请返回原入口核对。",
  DELETION_BATCH_LOCK_BUSY: "其他窗口正在处理原清空批次，请稍后核对。",
  DELETION_BATCH_LOCK_UNAVAILABLE: "当前环境不支持跨窗口保护，尚未发送清空。",
  DELETION_BATCH_STORAGE_STALE: "预览数据已被其他窗口更新，请保全输入并重开预览后核对。",
  DELETION_BATCH_STORAGE_CHECK_UNAVAILABLE: "当前存储无法核对其他窗口的更新，尚未发送清空。",
  DELETION_STATUS_MISMATCH: "查询结果与原清空成员不匹配，未采用该结果。",
};
export function deletionBatchMessage(state: ConversationDeletionBatchSnapshot, diagnosticLimit = Infinity): string {
  const batch = state.batch, identity = batch ? `operation=${batch.batchId}` : "";
  const done = Object.values(state.items).filter(item => item.phase === "committed" || item.phase === "ready").length;
  const messages = Object.values(state.items).flatMap(item => item.localIssue || item.cause ? [deletionRecoveryMessage(item)] : []);
  if (state.issue) {
    const code = state.issue instanceof Error && local[state.issue.message] ? state.issue.message : "DELETION_BATCH_RECOVERY_UNAVAILABLE";
    messages.unshift(`${local[code] ?? "本机清空恢复记录未能可靠读取或保存，请重试本机恢复记录。"}（${code}；${identity}）`);
    if (batch) {
      const diagnostic = conversationDeletionDiagnostic(state.issue, batch.requests[0]!.requestId, {force: true});
      if (diagnostic) messages.push(diagnostic);
    }
  }
  const action = deletionBatchAction(state);
  const instruction = action === "check" ? "请核对原清空，核对不会继续删除。"
    : action === "continue" ? "可显式继续原清空，仅处理原批次，不纳入后来归档的对话。"
      : action === "finish" ? (batch?.retained.length || Object.values(state.items).some(item => item.phase === "rejected")
        ? "已知拒绝的对话会保留；请完成本机收尾并结束本批次。" : "原删除已确认；请完成本机收尾，不会再次删除。") : "请先修复本机恢复记录。";
  return `${batch ? `原批次 ${batch.requests.length} 个，已确认删除 ${done} 个。` : ""}${instruction}${messages.slice(0, diagnosticLimit).join("；")}${messages.length > diagnosticLimit ? "其余诊断保留在弹窗中。" : ""}`;
}
