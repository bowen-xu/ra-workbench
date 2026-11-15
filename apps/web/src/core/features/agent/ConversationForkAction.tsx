import { ExternalLink, GitBranch, LoaderCircle } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { decodeAgentSqlImmediateFailure, type AgentConversationSummary } from "@research-assistant/core/agent";
import { ControlHint } from "../../components/ControlHint";
import { getAppStorage } from "../../runtime/appStorage";
import { notify } from "../../services/notifications";
import { aiAssistanceErrorMessage } from "../../services/aiAssistanceErrorMessage";
import { diagnosticError } from "../../services/diagnosticError";
import { acquireConversationForkOwner, type ConversationForkController, type ConversationForkPorts, type ConversationForkSnapshot } from "./conversationForkController";
import { forkScope, type ForkScope } from "./conversationForkPending";

function failureMessage(error: unknown, childId: string | undefined): string {
  const storage = decodeAgentSqlImmediateFailure(error);
  if (storage && (!storage.diagnostic?.operationId || storage.diagnostic.operationId === childId)) {
    const message = storage.diagnostic?.safeCause === "data_epoch_changed"
      ? "数据环境已变化；保留原分支请求，请回到原数据环境核对。" : "分支原操作需核对；保留同一子会话身份。";
    return `${message}${aiAssistanceErrorMessage(error)}`;
  }
  if (error === "conflict") return "对话状态已变化，请核对原分支操作。";
  if (error === "busy") return "所选轮次尚未结束，请刷新历史后核对。";
  if (error === "notFound") return "无法找到该轮次或稿件关联，请刷新历史后核对。";
  if (error === "tooLarge") return "历史超过当前分支处理上限，未创建分支。";
  if (error === "corrupt") return "历史损坏或版本不受支持，无法创建分支。";
  if (error instanceof Error && error.message === "FORK_RECOVERY_EPOCH_CHANGED") return "数据环境已变化；保留原分支请求，请回到原数据环境核对。";
  if (error instanceof Error && error.message === "FORK_RECOVERY_LOCK_BUSY") return "其他窗口正在处理此分支；稍后核对原请求。（FORK_RECOVERY_LOCK_BUSY）";
  if (error instanceof Error && error.message === "FORK_RECOVERY_LOCK_UNAVAILABLE") return "当前环境不支持跨窗口互斥，未发送分支请求。（FORK_RECOVERY_LOCK_UNAVAILABLE）";
  if (error instanceof Error && error.message === "FORK_RECOVERY_STORAGE_STALE") return "预览数据已被其他窗口更新；现有草稿和请求保留。请先保全输入，再重开预览核对。（FORK_RECOVERY_STORAGE_STALE）";
  if (error instanceof Error && /^FORK_RECOVERY_/u.test(error.message)) return `本机分支恢复记录需核对，已保留原记录。（${error.message}）`;
  const safe = diagnosticError(error), fields: string[] = [];
  if (typeof safe.code === "string") fields.push(safe.code);
  if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599) fields.push(`HTTP ${safe.httpStatus}`);
  return `无法确认分支创建成功。请核对原分支操作；核对不会重新创建或启动模型。${fields.length ? `（${fields.join("；")}）` : ""}`;
}
interface Props {
  readonly scope: ForkScope; readonly turnLabel: string; readonly disabled: boolean;
  readonly ports: ConversationForkPorts; readonly onOpen: (conversation: AgentConversationSummary) => void;
  readonly recoveryOnly?: boolean;
}
/** Same control and retained owner in every host, including mock-data previews. */
export function ConversationForkAction({ scope, turnLabel, disabled, ports, onOpen, recoveryOnly = false }: Props): React.JSX.Element | null {
  const storage = getAppStorage(), identity = forkScope(scope), owner = useRef<ConversationForkController | null>(null);
  const live = useRef(0), [owned, setOwned] = useState<{storage: Storage; identity: string; snapshot: ConversationForkSnapshot} | null>(null);
  const state = owned?.storage === storage && owned.identity === identity ? owned.snapshot : null;
  useEffect(() => {
    const lease = acquireConversationForkOwner(scope, ports, storage), controller = lease.controller;
    owner.current = controller;
    const sync = () => setOwned({storage, identity, snapshot: controller.getSnapshot()});
    const detach = controller.subscribe(sync); sync();
    return () => { live.current++; detach(); lease.release(); if (owner.current === controller) owner.current = null; };
  }, [identity, ports.fork, ports.metadata, storage]);
  async function run(action: "submit" | "check" | "retryOriginal" | "readCurrent" | "retryLocal"): Promise<void> {
    if (disabled || recoveryOnly && action === "submit") return;
    const view = live.current, lease = acquireConversationForkOwner(scope, ports, storage);
    try {
      const controller = lease.controller;
      if (action === "check" || action === "retryLocal") {
        if (action === "check") await controller.check(); else await controller.retryLocal();
        if (controller.getSnapshot().proof) await controller.readCurrent();
      }
      else await controller[action]();
      if (view !== live.current) return;
      const result = controller.getSnapshot();
      if (result.localIssue || result.cause) notify.error(failureMessage(result.localIssue ?? result.cause, result.record?.intent.childId));
      else if (result.current) notify.success(`${action === "check" ? "已核对" : "已创建"}“${result.current.title}”${result.current.archived ? "（已归档）" : ""}；当前对话和草稿保留。`);
    } finally { lease.release(); }
  }
  const busy = !state || owner.current?.hasPendingOperation() || ["locking", "saving", "checking", "reading", "finishing"].includes(state.phase);
  const ready = state?.phase === "ready", idle = state?.phase === "idle";
  const label = ready ? "打开分支" : busy ? "正在核对分支…" : idle ? "从此轮创建分支" : "核对原分支操作";
  const Icon = ready ? ExternalLink : busy ? LoaderCircle : GitBranch;
  async function finish(open: boolean): Promise<void> {
    const controller = owner.current;
    if (!controller || disabled) return;
    const view = live.current, finished = await controller.finish(open ? onOpen : undefined);
    if (!finished && view === live.current) notify.error(failureMessage(controller.getSnapshot().localIssue, state?.record?.intent.childId));
  }
  if (recoveryOnly && idle) return null;
  return <>
    <ControlHint content={disabled ? "等待历史及对话状态核对完成" : `${label}；不会发送消息或执行工具`}>
      <button type="button" disabled={disabled || busy} aria-label={disabled ? "等待历史及对话状态核对完成" : `${label}（${turnLabel}）`}
        onClick={() => ready ? void finish(true) : void run(idle ? "submit" : "check")}>
        <Icon className={busy ? "conversation-fork-spinner" : undefined} size={14} aria-hidden="true" />
        <span className="conversation-fork-sr-only">{label}</span>
      </button>
    </ControlHint>
    {state && !idle && !ready && !busy && <div className="conversation-fork-recovery" aria-label={`${turnLabel}分支恢复`}>
      {state.record && <details><summary>原分支请求</summary><p>parent={state.record.intent.parentId}<br />child / operationId={state.record.intent.childId}<br />turn={state.record.intent.throughTurnId}</p></details>}
      {state.proof && <p>原分支已提交；当前子会话另行读取。</p>}
      {state.status?.type === "noSqlReceipt" && <p>未查到原回执，不表示从未创建。{state.status.closed ? "存储已关闭，请重开后核对。" : ""}</p>}
      {state.status?.type === "unconfirmed" && <p>原结果仍不确定；保留请求，待存储恢复后核对。</p>}
      {state.localIssue != null && <button type="button" disabled={disabled} onClick={() => { void run("retryLocal"); }}>重试本机分支记录保存</button>}
      {state.phase === "notCommitted" && <button type="button" disabled={disabled || state.status?.type !== "noSqlReceipt" || state.status.closed || !!state.localIssue}
        onClick={() => { void run("retryOriginal"); }}>重试原分支请求</button>}
      {(state.phase === "committed" || state.phase === "readError") && <button type="button" disabled={disabled} onClick={() => { void run("readCurrent"); }}>读取当前分支</button>}
      {state.proof && state.phase === "readError" && state.cause === "notFound" && <button type="button" disabled={disabled} onClick={() => { void finish(false); }}>结束已提交分支核对</button>}
    </div>}
    {ready && state.localIssue != null && <div className="conversation-fork-recovery"><button type="button" disabled={disabled} onClick={() => { void run("retryLocal"); }}>重试本机分支记录保存</button><button type="button" disabled={disabled} onClick={() => { void finish(false); }}>清理本机分支记录</button></div>}
  </>;
}
