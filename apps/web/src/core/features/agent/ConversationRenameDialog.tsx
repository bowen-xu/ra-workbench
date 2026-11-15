import { useCallback, useEffect, useRef, useState } from "react";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentCatalogGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { OverlayPortal } from "../../components/overlay/OverlayPortal";
import { agentMetadataGateway } from "../../services/agentGateway";
import { notify } from "../../services/notifications";
import { useAgentMountedGuard } from "./useAgentMountedGuard";
import { useConversationRename } from "./useConversationRename";
import { renameUnresolved } from "./conversationRenameController";
import { ConversationRenameRecovery } from "./ConversationRenameRecovery";
import "./ConversationRenameDialog.css";

interface Props {
  readonly conversationId: string;
  readonly initialTitle: string;
  readonly catalog: Pick<AgentCatalogGateway, "rename" | "renameStatus">;
  readonly metadata?: Pick<AgentMetadataGateway, "read">;
  readonly onClose: () => void;
  readonly onRenamed?: (conversation: AgentConversationSummary) => void;
  readonly onConflict?: () => void;
}

export function ConversationRenameDialog({ conversationId, initialTitle, catalog, metadata = agentMetadataGateway, onClose, onRenamed, onConflict }: Props): React.JSX.Element {
  const [title, setTitle] = useState(initialTitle);
  const [expectedTitle] = useState(initialTitle);
  const [saving, setSaving] = useState(false);
  const [conflict, setConflict] = useState(false);
  const pending = useRef(false);
  const mounted = useAgentMountedGuard();
  const composing = useRef(false);
  const dialog = useRef<HTMLDialogElement>(null);
  const [surface, setSurface] = useState<HTMLDialogElement | null>(null);
  const attach = useCallback((element: HTMLDialogElement | null) => {
    // The shared portal can replace its initial node. Release the actual
    // native top-layer owner on ref detachment, before its DOM is removed.
    if (dialog.current !== element && dialog.current?.open) dialog.current.close();
    dialog.current = element;
    setSurface(element);
  }, []);
  const rename = useConversationRename(conversationId, { catalog, metadata });
  const scope = useRef(conversationId); scope.current = conversationId;
  const currentTitle = useRef(initialTitle); currentTitle.current = initialTitle;
  const unresolved = !!rename.state && renameUnresolved(rename.state);
  const valid = title.trim().length > 0 && [...title.trim()].length <= 180 && !title.includes("\0");
  useEffect(() => {
    if (!surface) return;
    if (!surface.open) surface.showModal();
    surface.querySelector("input")?.select();
  }, [surface]);
  useEffect(() => {
    if (rename.state?.intent && unresolved) setTitle(rename.state.intent.title);
    if (rename.state?.phase === "rejected" && rename.state.cause === "conflict") setConflict(true);
  }, [rename.state, unresolved]);
  function cancel(): void {
    if (pending.current) return;
    onClose();
    if (conflict) onConflict?.();
  }
  async function save(): Promise<void> {
    if (pending.current || composing.current || conflict || !valid || unresolved) return;
    if (title.trim() === expectedTitle) { onClose(); return; }
    pending.current = true;
    setSaving(true);
    try {
      current(await rename.submit(expectedTitle, title.trim()));
    } finally {
      pending.current = false;
      if (mounted.current) setSaving(false);
    }
  }
  function current(result: AgentConversationSummary | null): void {
    if (!result) return;
    notify.success(`改名操作已保存，当前名称为“${result.title}”`);
    if (mounted.current && scope.current === result.id) {
      if (currentTitle.current === expectedTitle || currentTitle.current === result.title) onRenamed?.(result);
      else onConflict?.();
      onClose();
    }
  }
  return <OverlayPortal kind="surface"><dialog ref={attach} className="conversation-rename-dialog" aria-label="重命名对话"
    onCancel={(event) => { event.preventDefault(); cancel(); }}>
    <form onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <h2>重命名对话</h2>
      <label>对话名称<input autoFocus value={title} readOnly={saving || unresolved} aria-invalid={!valid}
        onFocus={event => event.currentTarget.select()}
        onChange={(event) => setTitle(event.target.value)} onCompositionStart={() => { composing.current = true; }}
        onCompositionEnd={() => { composing.current = false; }}
        onKeyDown={(event) => { if (event.key === "Enter" && (event.nativeEvent.isComposing || composing.current || event.keyCode === 229)) event.preventDefault(); }} /></label>
      {!valid && <p>名称不能为空，且不能超过 180 个字符或包含空字符。</p>}
      <ConversationRenameRecovery {...rename} onCurrent={current}/>
      <div className="conversation-rename-actions"><button type="button" disabled={saving} onClick={cancel}>取消</button>
        <button type="submit" disabled={saving || unresolved || conflict || !valid}>{saving ? "正在保存…" : "保存"}</button></div>
    </form>
  </dialog></OverlayPortal>;
}
