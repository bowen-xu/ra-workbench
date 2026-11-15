import {diagnosticError} from "../../services/diagnosticError";
import type {WritingSubmissionSnapshot} from "./writingSubmissionController";
export function writingWorkspacePreparationMessage(state: WritingSubmissionSnapshot, unsupported = false): string {
  const failure = state.workspaceFailure, preview = state.preview;
  const code = typeof failure === "string" && ["invalidInput", "busy", "conflict", "denied", "notFound", "unavailable", "recoveryRequired", "unsupportedPlatform", "WORKSPACE_RECOVERY_METADATA_MISSING"].includes(failure)
    ? failure : diagnosticError(failure).code;
  const message = unsupported ? "原请求冻结的模型不支持当前执行模式，请重新选择模型或返回编辑。"
    : state.queue ? "无法准备队列消息，原消息已保留，请检查后重试。" : "无法准备可编辑副本，原请求已保留，请检查后重试。";
  return `${message}${code && preview ? ` [code=${code}；C=${preview.conversationId}；R=${preview.requestId}；contextSnapshot=${preview.contextSnapshotId}]` : ""}`;
}
