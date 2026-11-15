import { useEffect, useRef, useState } from "react";
import { Star, X } from "lucide-react";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteRevisionSummary } from "@research-assistant/data/notes";
import { Dialog } from "../../../components/overlay/OverlayDialog";
import { notify } from "../../../services/notifications";
import { NoteSnapshotView } from "../editor/NoteSnapshotView";
import { formatNoteDate } from "./notesPresentation";

export function NoteHistoryDialog({ revisions, onClose, onLoad, onRestore, onPin, readOnly }: {
  readonly revisions: readonly NoteRevisionSummary[];
  readonly onClose: () => void;
  readonly onLoad: (id: NoteRevisionSummary["id"]) => Promise<NoteDocumentV1>;
  readonly onRestore: (id: NoteRevisionSummary["id"]) => Promise<void>;
  readonly onPin: (revision: NoteRevisionSummary) => void;
  readonly readOnly: boolean;
}): React.JSX.Element {
  const [selection, setSelection] = useState<NoteRevisionSummary>();
  const [document, setDocument] = useState<NoteDocumentV1>();
  const [loading, setLoading] = useState(false), [restoring, setRestoring] = useState(false);
  const generation = useRef(0);
  useEffect(() => () => { generation.current++; }, []);
  async function select(revision: NoteRevisionSummary) {
    const version = ++generation.current;
    setSelection(revision); setDocument(undefined); setLoading(true);
    try { const result = await onLoad(revision.id); if (version === generation.current) setDocument(result); }
    catch (reason) { if (version === generation.current) notify.error(reason instanceof Error ? reason.message : String(reason)); }
    finally { if (version === generation.current) setLoading(false); }
  }
  async function restore() {
    if (!selection || !document || restoring) return;
    setRestoring(true);
    try { await onRestore(selection.id); onClose(); }
    catch (reason) { notify.error(reason instanceof Error ? reason.message : String(reason)); }
    finally { setRestoring(false); }
  }
  return <Dialog open onClose={() => { if (!restoring) onClose(); }} hideOnEscape={!restoring} hideOnInteractOutside={!restoring} aria-label="版本历史" className="note-history-dialog">
    <header><h2>版本历史</h2><button aria-label="关闭版本历史" disabled={restoring} onClick={onClose} type="button"><X size={18} /></button></header>
    <div className="note-history-layout"><nav aria-label="历史版本列表">
      <p className="note-history-policy">自动保存保留 14 天，重要历史保留 30 天。固定版本长期保留。</p>
      {revisions.length === 0 && <p>还没有历史版本。</p>}
      {revisions.map(revision => <div className="note-history-item" key={revision.id}>
        <button aria-pressed={selection?.id === revision.id} disabled={restoring} className="note-history-restore" onClick={() => void select(revision)} type="button"><span>版本 {revision.pageRevision}</span><small>{formatNoteDate(revision.createdAt)}</small></button>
        <button aria-label={`${revision.isPinned ? "取消固定" : "固定"}版本 ${revision.pageRevision}`} disabled={restoring} aria-pressed={revision.isPinned} className="note-history-pin" onClick={() => onPin(revision)} type="button"><Star aria-hidden="true" fill={revision.isPinned ? "currentColor" : "none"} size={15} /></button>
      </div>)}
    </nav><section aria-label="历史版本预览" aria-busy={loading} className="note-history-preview">
      {!selection && <p>选择一个版本预览。当前正文不会改变。</p>}
      {loading && <p role="status">正在读取版本…</p>}
      {document && <NoteSnapshotView key={selection!.id} document={document} />}
    </section></div>
    <footer><span>恢复前的正文会保留在版本历史中。</span><button className="primary-button" disabled={readOnly || !document || loading || restoring} onClick={() => void restore()} type="button">{restoring ? "正在恢复…" : "恢复此版本"}</button></footer>
  </Dialog>;
}
