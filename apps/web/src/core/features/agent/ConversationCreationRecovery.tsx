import { decodeAgentSqlImmediateFailure, type AgentConversationSummary } from "@research-assistant/core/agent";
import { diagnosticError } from "../../services/diagnosticError";
import { aiAssistanceErrorMessage } from "../../services/aiAssistanceErrorMessage";
import type { useBoundConversationCreation } from "./useBoundConversationCreation";
import { creationCanFinish } from "./conversationCreationController";
import { useAgentRequestGuard } from "./useAgentMountedGuard";
import { AgentRecoveryMessage } from "./AgentRecoveryMessage";

export function conversationCreationMessage(cause: unknown): string {
  if (decodeAgentSqlImmediateFailure(cause)) return aiAssistanceErrorMessage(cause);
  const safe = diagnosticError(cause), fields: string[] = [];
  if (typeof safe.code === "string") fields.push(safe.code);
  if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599) fields.push(`HTTP ${safe.httpStatus}`);
  const message = cause === "notFound" ? "当前对话或关联已不可读取，请核对原操作。" : cause === "conflict" ? "原创建意图与当前记录冲突。"
    : cause === "invalidInput" ? "原创建请求无效。" : cause instanceof Error && cause.message === "CREATION_RECOVERY_CLEANUP_REQUIRED"
      ? "当前会话已接纳，请先清理本机恢复记录再新建对话。" : cause instanceof Error && cause.message === "CREATION_RECOVERY_CONFLICT"
      ? "本机已保留另一个创建意图，请核对其他窗口。" : cause instanceof Error && /CREATION_RECOVERY_/.test(cause.message)
        ? "无法核对本机创建记录，已保留原记录。" : "创建结果尚未确认，请核对原操作。";
  return `${message}${fields.length ? `（${fields.join("；")}）` : ""}`;
}
type Props = Pick<ReturnType<typeof useBoundConversationCreation>, "state" | "check" | "retryOriginal" | "readCurrent" | "finishKnown" | "clearAdopted"> & {
  readonly captureAdoption: () => (conversation: AgentConversationSummary | null) => void;
  readonly onFinished?: () => void;
};
export function ConversationCreationRecovery({ state, check, retryOriginal, readCurrent, finishKnown, clearAdopted, captureAdoption, onFinished }: Props): React.JSX.Element | null {
  const requestGuard = useAgentRequestGuard();
  function read(action: () => Promise<AgentConversationSummary | null>): void {
    const current = requestGuard(), adopt = captureAdoption();
    void action().then(conversation => { if (current()) adopt(conversation); });
  }
  if (!state || (state.phase === "idle" && !state.localIssue)) return null;
  // These phases belong to the ordinary create-and-adopt flow. A failure is
  // still displayed immediately, including a failed local acknowledgement.
  if (!state.localIssue && !state.cause && ["saving", "committing", "reading"].includes(state.phase)) return null;
  const busy = ["saving", "checking", "reading"].includes(state.phase);
  return <AgentRecoveryMessage id="conversation-creation-recovery" label="对话创建恢复" content="对话创建状态需要处理，请检查后继续。"
    diagnosticText={state.intent ? `C=${state.intent.id}；operationId=${state.intent.operationId}` : undefined}>
    {state.proof && <p>原创建操作已保存{state.current ? "。" : "；当前会话须另行读取。"}</p>}
    {state.adopted && <p>当前会话已接纳，本机恢复记录尚待清理。</p>}
    {state.localIssue != null && <p role="alert">{conversationCreationMessage(state.localIssue)}</p>}
    {state.cause != null && <p role="alert">{state.phase === "readError" ? "原创建操作已保存，但当前会话读取失败。" : ""}{conversationCreationMessage(state.cause)}</p>}
    {state.current?.archived && <p role="status">此对话已归档，不能继续发送。恢复后可重新读取当前对话。</p>}
    {state.status?.type === "noSqlReceipt" && <p>当前 SQL 未查到原创建回执；这不表示原操作从未执行。{state.status.closed ? "请重开后核对。" : ""}</p>}
    {state.status?.type === "unconfirmed" && <p>结果仍待核对：{state.status.reason}</p>}
    <div className="agent-failure-actions">
    {(state.phase === "unknown" || state.phase === "notCommitted") && state.intent && <button type="button" disabled={busy} onClick={() => { void check(); }}>核对原创建操作</button>}
    {!state.adopted && ["committed", "readError", "ready"].includes(state.phase) && <button type="button" disabled={busy} onClick={() => read(readCurrent)}>读取当前对话</button>}
    {state.adopted && <button type="button" onClick={() => { if (clearAdopted()) onFinished?.(); }}>清理本机记录并结束恢复</button>}
    {state.phase === "notCommitted" && <button type="button" disabled={state.status?.type !== "noSqlReceipt" || state.status.closed} onClick={() => read(retryOriginal)}>重试原创建操作</button>}
    {creationCanFinish(state) && <button type="button" onClick={() => { if (finishKnown()) onFinished?.(); }}>保留草稿并结束本次创建核对</button>}
    </div>
    {busy && <p role="status">{state.phase === "saving" ? "正在创建对话…" : state.phase === "checking" ? "正在核对原创建操作…" : "正在读取当前对话…"}</p>}
  </AgentRecoveryMessage>;
}
