import { useEffect, useRef, useState } from "react";
import { agentDeletionErrors, decodeAgentDeletionRequest, decodeAgentSqlImmediateFailure, type AgentCatalogCursor, type AgentConversationBinding, type AgentDeletionRequest } from "@research-assistant/core/agent";
import type { AgentCatalogGateway, AgentDeletionBatchGateway } from "@research-assistant/data/agent";
import { notify } from "../../services/notifications";
import { diagnosticError } from "../../services/diagnosticError";
import { aiAssistanceErrorMessage } from "../../services/aiAssistanceErrorMessage";
import { ConversationDeleteConfirmation } from "./ConversationDeleteConfirmation";
import { useAgentMountedGuard } from "./useAgentMountedGuard";
import { deletionBatchCatalogMessage } from "./conversationDeletionBatchMessage";
import { deletionReasonMessage } from "./conversationDeletionRecoveryMessage";
import "./ConversationDeleteDialog.css";

interface Props {
  readonly internal?: boolean;
  readonly binding: AgentConversationBinding | null;
  readonly catalog: AgentCatalogGateway;
  readonly deletions: AgentDeletionBatchGateway;
  readonly viewIdentity?: string;
  readonly onClose: () => void;
  readonly onConfirmed: (id: string) => void | Promise<void>;
  readonly flushCleanup?: () => Promise<void>;
  readonly onDeleted: (ids: readonly string[]) => void;
}

/** Freeze the complete scope for confirmation, then submit a single native batch. */
export function ConversationArchiveClearDialog({binding, internal = false, catalog, deletions, viewIdentity, onClose, onConfirmed, onDeleted, flushCleanup}: Props): React.JSX.Element | null {
  const [original] = useState(() => ({binding: structuredClone(binding), internal, catalog, deletions, viewIdentity, onClose, onConfirmed, onDeleted, flushCleanup}));
  const scope = JSON.stringify([binding, internal, viewIdentity]);
  const view = useRef({scope, deletions, generation: 0});
  if (view.current.scope !== scope || view.current.deletions !== deletions) {
    view.current = {scope, deletions, generation: view.current.generation + 1};
  }
  const [requests, setRequests] = useState<readonly AgentDeletionRequest[] | null>(null);
  const [loadFailure, setLoadFailure] = useState<{cause: unknown} | null>(null);
  const [pending, setPending] = useState(false);
  const mounted = useAgentMountedGuard(), loadEpoch = useRef(0), running = useRef(false);
  async function load(): Promise<void> {
    const epoch = ++loadEpoch.current, generation = view.current.generation; setRequests(null); setLoadFailure(null);
    try {
      const plan: AgentDeletionRequest[] = [], seen = new Set<string>(), cursors = new Set<string>();
      let before: AgentCatalogCursor | null = null;
      do {
        const page = await original.catalog.list({archived: true, limit: 100, before, binding: original.binding, ...(original.internal ? {internal: true} : {})});
        if (!mounted.current || generation !== view.current.generation || epoch !== loadEpoch.current) return;
        for (const conversation of page.conversations) {
          if (!conversation.archived || Boolean(conversation.internal) !== original.internal) throw Error("DELETION_BATCH_CATALOG_CHANGED");
          if (seen.has(conversation.id)) continue; seen.add(conversation.id);
          plan.push(decodeAgentDeletionRequest({schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: conversation.id,
            binding: original.binding, expectedTitle: conversation.title, expectedDurableCount: conversation.durableCount, force: true}));
        }
        before = page.next;
        if (before) {const cursor = JSON.stringify(before); if (cursors.has(cursor)) throw Error("DELETION_BATCH_CATALOG_CHANGED"); cursors.add(cursor);}
      } while (before);
      setRequests(plan);
    } catch (cause: unknown) {if (mounted.current && generation === view.current.generation && epoch === loadEpoch.current) setLoadFailure({cause});}
  }
  useEffect(() => {void load(); return () => {loadEpoch.current++;};}, []);
  const loading = requests === null && !loadFailure;
  async function perform(): Promise<void> {
    if (running.current || loading) return;
    if (loadFailure) {await load(); return;}
    if (!requests?.length) return;
    running.current = true; setPending(true);
    const generation = view.current.generation;
    const accept = () => mounted.current && generation === view.current.generation;
    try {
      const result = await original.deletions.deleteBatch(requests);
      const deleted = result.results.filter(item => item.status === "deleted").map(item => item.conversationId);
      const failed = result.results.filter(item => item.status === "failed");
      const pendingCleanup = result.results.filter(item => item.status === "deleted" && item.cleanupPending).length;
      let localCleanupFailed = false;
      for (const id of deleted) {
        try {await original.onConfirmed(id);} catch {localCleanupFailed = true;}
      }
      if (deleted.length && original.flushCleanup) {
        try {await original.flushCleanup();} catch {localCleanupFailed = true;}
      }
      if (!accept()) return;
      if (failed.length) {
        const details = failed.slice(0, 3).map(item => {
          const safe = diagnosticError(item.error);
          const sql = decodeAgentSqlImmediateFailure(item.error);
          const code = typeof item.error === "string" && agentDeletionErrors.some(error => error === item.error) ? item.error
            : typeof safe.code === "string" ? safe.code : "DELETE_FAILED";
          const sqlMatches = sql && (!sql.diagnostic?.operationId || sql.diagnostic.operationId.toLowerCase() === item.requestId.toLowerCase());
          const reason = sqlMatches ? aiAssistanceErrorMessage(item.error) : deletionReasonMessage(code) ?? "删除失败，请刷新列表后重新确认。";
          return `${reason}（code=${code}；operation=${item.requestId}）`;
        }).join("；");
        notify.error(`已删除 ${deleted.length} 个归档对话，${failed.length} 个未完成。${localCleanupFailed ? "对话已删除，本地清理未完成，请检查本机存储。" : ""}${pendingCleanup ? `${pendingCleanup} 个对话的执行副本仍待清理，将在下次启动或后续清理时重试。` : ""}${details}`);
      } else if (localCleanupFailed) notify.error(`已删除 ${deleted.length} 个归档对话，本地清理未完成，请检查本机存储。${pendingCleanup ? `${pendingCleanup} 个对话的执行副本仍待清理。` : ""}`);
      else if (pendingCleanup) notify.warning(`已删除 ${deleted.length} 个归档对话；其中 ${pendingCleanup} 个对话的执行副本仍待清理，将在下次启动或后续清理时重试。`);
      else notify.success(`已清空 ${deleted.length} 个归档对话`);
      if (deleted.length) original.onDeleted(deleted); original.onClose();
    } catch (cause: unknown) {
      if (!accept()) return;
      const safe = diagnosticError(cause);
      const known = typeof cause === "string" && agentDeletionErrors.some(error => error === cause) ? cause : null;
      const code = known ?? (typeof safe.code === "string" ? safe.code : "DELETION_BATCH_UNCONFIRMED");
      const details = decodeAgentSqlImmediateFailure(cause) ? aiAssistanceErrorMessage(cause) : "";
      const identity = requests.slice(0, 3).map(request => request.requestId).join(", ");
      const reason = known && known !== "unavailable" ? deletionReasonMessage(known) ?? "本次清空被拒绝。" : "无法确认清空结果，请刷新列表后重新确认。";
      notify.error(`${reason}（code=${code}；operation=${identity}）${details}`);
      original.onClose();
    } finally {running.current = false; if (accept()) setPending(false);}
  }
  if (view.current.generation !== 0) return null;
  const scopeLabel = original.binding ? `${{manuscript: "当前稿件", paper: "当前文献", note: "当前笔记", readingNote: "当前阅读笔记"}[original.binding.kind]}关联的` : "全部";
  const description = loading ? "正在核对已归档对话…" : loadFailure ? "尚未核对归档对话，未发送本批次删除。" : requests?.length === 0 ? "没有可清理的归档对话。"
    : `将永久删除${scopeLabel} ${requests?.length ?? ""} 个已归档对话及其历史、分支来源和执行副本，无法撤销。论文、稿件、笔记、待确认修改及文件恢复记录会保留。搜索条件不限制清理范围，运行中的对话需先停止。`;
  return <ConversationDeleteConfirmation title="清空归档对话" description={description} tone="warning" confirmLabel="清空归档" autoConfirm={false}
    pending={pending} blocked={loading || requests?.length === 0} retry={loadFailure !== null}
    {...(loadFailure ? {errorMessage: deletionBatchCatalogMessage(loadFailure.cause)} : {})}
    onClose={() => {if (!running.current) onClose();}} onConfirm={() => void perform()} />;
}
