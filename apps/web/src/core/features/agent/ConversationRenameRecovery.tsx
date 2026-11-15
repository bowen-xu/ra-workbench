import { decodeAgentSqlImmediateFailure, type AgentConversationSummary } from "@research-assistant/core/agent";
import { aiAssistanceErrorMessage } from "../../services/aiAssistanceErrorMessage";
import { diagnosticError } from "../../services/diagnosticError";
import type { useConversationRename } from "./useConversationRename";

export function conversationRenameMessage(cause: unknown): string {
  if (decodeAgentSqlImmediateFailure(cause)) return aiAssistanceErrorMessage(cause);
  if (cause === "conflict") return "对话名称已在其他位置修改，请关闭后重新核对当前名称。";
  if (cause === "notFound") return "原对话已不存在，未确认新的名称。";
  if (cause === "invalidInput") return "名称或改名请求无效。";
  if (cause instanceof Error && ["RENAME_RECOVERY_INVALID", "RENAME_RECOVERY_UNAVAILABLE"].includes(cause.message))
    return "无法核对本机改名恢复记录，请保留原操作并检查本机存储。";
  if (cause instanceof Error && cause.message === "RENAME_RECOVERY_CONFLICT") return "本机恢复记录已对应另一个改名意图，已保留该记录。请核对其他窗口。";
  return `改名结果尚未确认，请核对原操作。${transportDiagnostic(cause)}`;
}
function transportDiagnostic(cause: unknown): string {
  const safe = diagnosticError(cause), details: string[] = [];
  if (typeof safe.code === "string") details.push(safe.code);
  if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599) details.push(`HTTP ${safe.httpStatus}`);
  return details.length ? `（${details.join("；")}）` : "";
}
type Props = Pick<ReturnType<typeof useConversationRename>, "state" | "check" | "retryOriginal" | "readCurrent"> & {
  readonly onCurrent: (current: AgentConversationSummary | null) => void;
};
/** Only the original typed intent can be inspected/retried. Proof is never a title. */
export function ConversationRenameRecovery({ state, check, retryOriginal, readCurrent, onCurrent }: Props): React.JSX.Element | null {
  if (!state || (["idle", "saving", "ready"].includes(state.phase) && !state.localIssue)) return null;
  const busy = state.phase === "checking" || state.phase === "reading";
  return <div className="conversation-rename-recovery" aria-live="polite">
    {state.proof && <><p>原改名操作已保存{state.current ? "。" : "；当前名称须另行读取。"}</p>
      <details><summary>查看保存证明</summary><p>epoch={state.proof.epoch}；sqlFence={state.proof.sqlFence}</p></details></>}
    {state.localIssue != null && <p role="alert">{conversationRenameMessage(state.localIssue)}</p>}
    {state.cause != null && <p role="alert">{state.phase === "readError"
      ? `原改名操作已保存，但当前名称读取失败。${decodeAgentSqlImmediateFailure(state.cause) ? conversationRenameMessage(state.cause) : `请重试读取。${transportDiagnostic(state.cause)}`}`
      : conversationRenameMessage(state.cause)}</p>}
    {state.status?.type === "unconfirmed" && <p>结果仍待核对：{state.status.reason}{state.status.closed ? "；存储已关闭" : ""}</p>}
    {state.status?.type === "noSqlReceipt" && <p>当前 SQL 未查到原操作回执；此状态不表示原操作从未执行。{state.status.closed ? "请重开后核对。" : ""}</p>}
    {state.intent && <p>C={state.intent.id}<br />operationId={state.intent.operationId}</p>}
    {(state.phase === "unknown" || state.phase === "notCommitted") && state.intent && <button type="button" disabled={busy} onClick={() => { void check(); }}>核对原改名操作</button>}
    {(state.phase === "committed" || state.phase === "readError") && <button type="button" disabled={busy} onClick={() => { void readCurrent().then(onCurrent); }}>读取当前名称</button>}
    {state.phase === "notCommitted" && <button type="button" disabled={state.status?.type !== "noSqlReceipt" || state.status.closed}
      onClick={() => { void retryOriginal().then(onCurrent); }}>重试原改名操作</button>}
    {busy && <p role="status">{state.phase === "checking" ? "正在核对原改名操作…" : "正在读取当前名称…"}</p>}
  </div>;
}
