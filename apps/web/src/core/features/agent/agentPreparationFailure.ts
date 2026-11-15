import { agentDraftFromInput, type AgentInputDraft, type AgentUserInput } from "@research-assistant/core/agent";
import { notify } from "../../services/notifications";

export const AGENT_PREPARATION_FAILURE = "请求核对失败，消息已保留在输入框中，请检查后重新发送。";
export const AGENT_START_FAILURE = "发送失败，消息已保留在输入框中，请检查后重新发送。";

/** Native validation errors are returned before admission. A rejection after
 * an earlier uncertain attempt cannot disprove that earlier admission. */
export function isAgentStartRejection(reason: unknown): boolean {
  if (typeof reason === "string") return ["modelUnavailable", "confirmationMismatch", "conversationUnavailable", "invalidInput"].includes(reason);
  if (!reason || typeof reason !== "object" || !("code" in reason)) return false;
  // writing_start::StartError is a tagged object. A queued admission CAS
  // conflict reaches Preparation(PreviewError::Conflict) before run_prepared.
  return reason.code === "confirmationMismatch" || reason.code === "unsupportedModel"
    || (reason.code === "preparation" && "reason" in reason && reason.reason === "conflict");
}

/** A failed preparation or definitively rejected start returns to the composer
 * without replaying the request or replacing a later draft with old contents. */
export function recoverAgentPreparationFailure({ text, userInput = [], originalDraft, read, restore, detail, failureMessage = AGENT_PREPARATION_FAILURE, notifyFailure = true }: {
  readonly text: string;
  readonly userInput?: readonly AgentUserInput[];
  readonly originalDraft: AgentInputDraft;
  readonly read: () => AgentInputDraft;
  readonly restore: (draft: AgentInputDraft) => void;
  readonly detail?: string;
  readonly failureMessage?: string;
  readonly notifyFailure?: boolean;
}): void {
  const current = read();
  if (current === originalDraft && !current.text && !current.evidence?.length && !current.unboundReferences?.length && text)
    restore(agentDraftFromInput(text, userInput));
  if (!notifyFailure) return;
  const message = current !== originalDraft || (current.text && current.text !== text)
    ? "请求核对失败，当前草稿已保留，请检查后重新发送。"
    : failureMessage;
  notify.error(detail ? `${message}\n${detail}` : message);
}
