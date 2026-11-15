import { decodeAgentSqlImmediateFailure } from "@research-assistant/core/agent";
import { aiAssistanceErrorMessage } from "../../services/aiAssistanceErrorMessage";
import { diagnosticError } from "../../services/diagnosticError";
import type { ConversationArchiveSnapshot } from "./conversationArchiveController";

export function conversationArchiveMessage(cause: unknown): string {
  if (decodeAgentSqlImmediateFailure(cause)) return aiAssistanceErrorMessage(cause);
  if (cause === "busy") return "对话仍有未结束任务，请等待完成或显式停止后再归档。";
  if (cause === "conflict") return "对话状态已变化，原归档／恢复操作未保存，请核对当前状态后重试。";
  if (cause === "tooLarge") return "对话历史超过当前归档校验上限，尚未归档。";
  if (cause === "corrupt") return "对话历史损坏或版本不受支持，无法归档。";
  if (cause === "notFound") return "原对话已不存在，未确认新的归档状态。";
  if (cause === "invalidInput") return "归档／恢复请求无效。";
  if (cause === "unavailable") return "归档服务暂时不可用，原操作结果尚未确认。（unavailable）";
  if (cause === "AGENT_PREVIEW_SCENARIO_CANCELED") return "预览场景正在结束，本次归档重试未被接纳；原操作仍待核对。（AGENT_PREVIEW_SCENARIO_CANCELED）";
  if (cause instanceof Error && cause.message === "ARCHIVE_RECOVERY_CONFLICT") return "本机归档恢复记录已变化，已保留原记录，请核对其他窗口。（ARCHIVE_RECOVERY_CONFLICT）";
  if (cause instanceof Error && ["ARCHIVE_RECOVERY_INVALID", "ARCHIVE_RECOVERY_UNAVAILABLE"].includes(cause.message))
    return `无法可靠保存或核对本机归档恢复记录；已保留原操作，请检查本机存储。（${cause.message}）`;
  const safe = diagnosticError(cause), details: string[] = [];
  if (typeof safe.code === "string" && /^[A-Za-z][A-Za-z0-9_.-]{0,100}$/.test(safe.code)) details.push(safe.code);
  if (cause instanceof Error && ["ARCHIVE_PROOF_MISMATCH", "ARCHIVE_METADATA_MISMATCH"].includes(cause.message)) details.push(cause.message);
  if (typeof safe.requestId === "string") details.push(`requestId=${safe.requestId}`);
  if (typeof safe.cause === "string" && safe.cause !== "unclassified") details.push(safe.cause);
  if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599) details.push(`HTTP ${safe.httpStatus}`);
  return `归档／恢复结果尚未确认，请核对原操作。${details.length ? `（${details.join("；")}）` : ""}`;
}
export function archiveRecoveryMessage(state: ConversationArchiveSnapshot): string {
  const identity = state.intent ? ` C=${state.intent.id}；operationId=${state.intent.operationId}。` : "";
  const title = state.context?.title.replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/gu, " ").trim();
  const characters = Array.from(title || "未命名对话");
  const label = characters.length > 40 ? `${characters.slice(0, 40).join("")}…` : characters.join("");
  const subject = state.intent ? `“${label}”的${state.intent.archived ? "归档" : "恢复"}操作：` : "";
  if (state.localIssue) return `${subject}${conversationArchiveMessage(state.localIssue)}${identity}`;
  if (state.proof) {
    const detail = state.phase === "readError" ? ` 当前状态读取失败：${conversationArchiveMessage(state.cause)}` : " 当前归档状态须另行读取。";
    return `${subject}原操作已保存。${detail}${identity}`;
  }
  if (state.recordedCommitted && state.status?.type === "noSqlReceipt") return `${subject}本机已记录原操作提交，但当前数据环境未找到回执；请核对原环境。${identity}`;
  if (state.status?.type === "unconfirmed") return `${subject}原操作结果仍待核对：${state.status.reason}${state.status.closed ? "；存储已关闭" : ""}。${identity}`;
  if (state.status?.type === "noSqlReceipt") return `${subject}未找到原操作回执，无法据此确认操作是否执行。${state.status.closed ? "请重开后核对。" : ""}${identity}`;
  return `${subject}${conversationArchiveMessage(state.cause)}${identity}`;
}
