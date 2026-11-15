import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentCatalogGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { agentMetadataGateway } from "../../services/agentGateway";
import { notify } from "../../services/notifications";
import { useAgentMountedGuard } from "./useAgentMountedGuard";
import { useConversationRename } from "./useConversationRename";
import { renameUnresolved } from "./conversationRenameController";
import { conversationRenameMessage } from "./ConversationRenameRecovery";
import { ConversationRenameDialog } from "./ConversationRenameDialog";
import "./ConversationHeaderTitle.css";

export const CONVERSATION_TITLE_CHANGED_EVENT = "research-assistant:conversation-title-changed";

export interface ConversationTitleRenameAccess {
  readonly conversationId: string;
  readonly catalog: Pick<AgentCatalogGateway, "rename" | "renameStatus">;
  readonly metadata?: Pick<AgentMetadataGateway, "read">;
  readonly onRenamed?: (conversation: AgentConversationSummary) => void;
  readonly onConflict?: () => void;
}

/** Inline title editing uses the same catalog contract as list renaming. */
export function ConversationHeaderTitle({title, rename}: {
  readonly title: string;
  readonly rename?: ConversationTitleRenameAccess;
}): React.JSX.Element {
  return rename ? <EditableConversationTitle key={rename.conversationId} title={title} rename={rename}/> : <span className="writing-agent-header-subtitle">{title}</span>;
}
function EditableConversationTitle({ title, rename }: { readonly title: string; readonly rename: ConversationTitleRenameAccess }): React.JSX.Element {
  const [editing, setEditing] = useState<{expectedTitle: string; draft: string} | null>(null);
  const [saving, setSaving] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [invalid, setInvalid] = useState(false);
  const trigger = useRef<HTMLSpanElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const active = useRef(false);
  const pending = useRef(false);
  const composing = useRef(false);
  const restoreFocus = useRef(false);
  const mounted = useAgentMountedGuard();
  const operation = useConversationRename(rename.conversationId, { catalog: rename.catalog, metadata: rename.metadata ?? agentMetadataGateway });
  const [recoveryOpen, setRecoveryOpen] = useState(false);
  const wasRecoveryOpen = useRef(false);
  const scope = useRef(rename.conversationId); scope.current = rename.conversationId;
  const latestTitle = useRef(title); latestTitle.current = title;
  const isEditing = editing !== null;
  useLayoutEffect(() => {
    if (isEditing) { input.current?.focus(); input.current?.select(); }
    else if (restoreFocus.current) { restoreFocus.current = false; trigger.current?.focus(); }
  }, [isEditing]);
  useEffect(() => {
    // Unknown removes the inline input before opening its native recovery
    // dialog, so the browser's remembered focus target may no longer exist.
    if (wasRecoveryOpen.current && !recoveryOpen) trigger.current?.focus();
    wasRecoveryOpen.current = recoveryOpen;
  }, [recoveryOpen]);
  function begin(): void {
    if (!rename || !title) return;
    if (operation.state && renameUnresolved(operation.state)) {
      if (!["saving", "checking", "reading"].includes(operation.state.phase)) setRecoveryOpen(true);
      return;
    }
    active.current = true;
    setConflict(false); setInvalid(false);
    setEditing({expectedTitle: title, draft: title});
  }
  function close(): void {
    active.current = false;
    setEditing(null);
  }
  async function save(returnFocus = false): Promise<void> {
    if (!active.current || !editing || !rename || pending.current || composing.current || conflict) return;
    const name = editing.draft.trim();
    if (!name || [...name].length > 180 || editing.draft.includes("\0")) {
      setInvalid(true);
      notify.error("名称不能为空，且不能超过 180 个字符或包含空字符。");
      return;
    }
    if (name === editing.expectedTitle) { restoreFocus.current = returnFocus; close(); return; }
    pending.current = true;
    setSaving(true);
    try {
      const result = await operation.submit(editing.expectedTitle, name);
      if (result) {
        notify.success(`改名操作已保存，当前名称为“${result.title}”`);
        window.dispatchEvent(new Event(CONVERSATION_TITLE_CHANGED_EVENT));
        if (mounted.current && scope.current === result.id) {
          if (latestTitle.current === editing.expectedTitle || latestTitle.current === result.title) rename.onRenamed?.(result);
          else rename.onConflict?.();
          restoreFocus.current = returnFocus; close();
        }
      } else if (mounted.current) {
        const failure = operation.getSnapshot();
        if (failure?.phase === "rejected") {
          notify.error(conversationRenameMessage(failure.cause));
          setConflict(failure.cause === "conflict");
        } else {
          // Recovery shares the production native dialog, not a private popup.
          close(); setRecoveryOpen(true);
        }
      }
    } finally {
      pending.current = false;
      if (mounted.current) setSaving(false);
    }
  }
  if (editing) return <input data-tab-action="both" ref={input} className="conversation-header-title-input" aria-label="对话名称"
    value={editing.draft} size={Math.min(32, Math.max(8, [...editing.draft].length))} readOnly={saving}
    aria-busy={saving} aria-invalid={invalid || conflict}
    onChange={event => { setEditing({...editing, draft: event.target.value}); setInvalid(false); }}
    onCompositionStart={() => { composing.current = true; }} onCompositionEnd={() => { composing.current = false; }}
    onBlur={() => { void save(); }}
    onKeyDown={event => {
      if (event.key === "Tab") {
        event.preventDefault(); event.stopPropagation();
      }
      if (event.nativeEvent.isComposing || composing.current || event.keyCode === 229) return;
      if (event.key === "Enter") {
        event.preventDefault(); event.stopPropagation(); return;
      }
      if (event.key === "Escape") {
        event.preventDefault(); event.stopPropagation();
        if (pending.current) return;
        restoreFocus.current = true; close();
        if (conflict) rename?.onConflict?.();
      } else if (event.key === "Tab") {
        void save(true);
      }
    }}/>;
  return <>
    <span ref={trigger} className="writing-agent-header-subtitle" {...(rename ? {
      role: "button", tabIndex: 0, "aria-label": `重命名对话 ${title}`, "aria-disabled": !title,
      onDoubleClick: begin,
      onKeyDown: (event: KeyboardEvent<HTMLSpanElement>) => {
        if (!title || event.nativeEvent.isComposing || !["Enter", "F2"].includes(event.key)) return;
        event.preventDefault(); begin();
      },
    } : {})}>{title}</span>
    {recoveryOpen && <ConversationRenameDialog conversationId={rename.conversationId} initialTitle={title}
      catalog={rename.catalog} metadata={rename.metadata ?? agentMetadataGateway} onClose={() => setRecoveryOpen(false)}
      onRenamed={result => { if (mounted.current && scope.current === result.id) rename.onRenamed?.(result); }} {...(rename.onConflict ? { onConflict: rename.onConflict } : {})}/>}</>;
}
