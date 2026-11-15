import { useCallback, useEffect, useRef, useState, type Ref } from "react";
import { Archive, ArchiveRestore, Download, MoreHorizontal, Pencil, Trash2 } from "lucide-react";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentCatalogGateway, AgentConversationExportGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { ActionMenu, ActionMenuItem } from "../../components/ActionMenu";
import { agentCatalogGateway, agentConversationExportGateway } from "../../services/agentGateway";
import { notify } from "../../services/notifications";
import "./ConversationExportMenu.css";
import { ConversationRenameDialog } from "./ConversationRenameDialog";
import { useAgentMountedGuard } from "./useAgentMountedGuard";

interface Props {
  readonly conversationId: string;
  readonly title: string;
  readonly gateway?: AgentConversationExportGateway;
  readonly catalog?: AgentCatalogGateway;
  readonly metadata?: Pick<AgentMetadataGateway, "read">;
  readonly onRenamed?: (conversation: AgentConversationSummary) => void;
  readonly onConflict?: () => void;
  readonly archived?: boolean;
  readonly archivePending?: boolean;
  readonly onArchive?: () => Promise<void>;
  readonly onDelete?: () => void;
  readonly deleting?: boolean;
  readonly contextMenuTrigger?: boolean;
  readonly triggerRef?: Ref<HTMLButtonElement>;
}
/** Keyed by conversation: completion always names the requested transcript. */
export function ConversationMenu({ conversationId, title, gateway = agentConversationExportGateway, catalog = agentCatalogGateway, metadata, onRenamed, onConflict, archived = false, archivePending = false, onArchive, onDelete, deleting = false, contextMenuTrigger = false, triggerRef }: Props): React.JSX.Element {
  const [renaming, setRenaming] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const retainTrigger = useCallback((element: HTMLButtonElement | null) => {
    trigger.current = element;
    if (typeof triggerRef === "function") return triggerRef(element);
    if (triggerRef) triggerRef.current = element;
  }, [triggerRef]);
  const mountedRef = useAgentMountedGuard();
  const wasRenaming = useRef(false);
  useEffect(() => {
    // A native modal keeps the background inert until React removes it.
    // Restoring focus inside the close handler is too early in WKWebView.
    if (wasRenaming.current && !renaming) trigger.current?.focus();
    wasRenaming.current = renaming;
  }, [renaming]);
  const [pending, setPending] = useState<"download" | "archive" | null>(null);
  const pendingRef = useRef(false);
  async function download(): Promise<void> {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending("download");
    try {
      const result = await gateway.download(conversationId);
      if (!mountedRef.current) return;
      notify.success(`“${title}”对话已下载到 Downloads`, { description: result.destinationPath });
    } catch (error) {
      if (!mountedRef.current) return;
      notify.error(error === "tooLarge" ? "对话历史超过导出上限，未生成下载文件。"
        : error === "corrupt" ? "对话历史损坏或版本不受支持，无法导出。"
          : `无法确认“${title}”对话下载成功，请检查 Downloads 文件夹后重试。`);
    } finally {
      pendingRef.current = false;
      if (mountedRef.current) setPending(null);
    }
  }
  function closeRename(): void { setRenaming(false); }
  async function archive(): Promise<void> {
    if (pendingRef.current || archivePending || !onArchive) return;
    pendingRef.current = true; setPending("archive");
    try { await onArchive(); }
    finally { pendingRef.current = false; if (mountedRef.current) setPending(null); }
  }
  return <><ActionMenu hint={!contextMenuTrigger} label={pending === "download" ? `正在下载“${title}”` : pending === "archive" ? `正在${archived ? "恢复" : "归档"}“${title}”` : `“${title}”对话操作`} trigger={<MoreHorizontal aria-hidden="true" size={16} />}
    triggerRef={retainTrigger} autoFocusOnHide={!renaming && !deleting}
    className={`conversation-export-menu${contextMenuTrigger ? " conversation-export-menu-context" : ""}`} contentClassName="conversation-export-content" disabled={pending !== null || archivePending}>
    <ActionMenuItem onClick={() => void download()}><Download aria-hidden="true" size={16} />下载对话（JSON）</ActionMenuItem>
    <ActionMenuItem onClick={() => setRenaming(true)}><Pencil aria-hidden="true" size={16} />重命名对话</ActionMenuItem>
    {onArchive && <ActionMenuItem onClick={() => void archive()}>{archived ? <ArchiveRestore aria-hidden="true" size={16} /> : <Archive aria-hidden="true" size={16} />}{archived ? "恢复对话" : "归档对话"}</ActionMenuItem>}
    {archived && onDelete && <ActionMenuItem onClick={onDelete}><Trash2 aria-hidden="true" size={16} />永久删除对话</ActionMenuItem>}
  </ActionMenu>{renaming && <ConversationRenameDialog key={conversationId} conversationId={conversationId} initialTitle={title} catalog={catalog}
    onClose={closeRename} {...(metadata ? { metadata } : {})} {...(onRenamed ? { onRenamed } : {})} {...(onConflict ? { onConflict } : {})} />}</>;
}
