import { useCallback, useEffect, useRef, useState } from "react";
import { NoteFloatingWindow } from "../../core/components/note-window/NoteFloatingWindow";
import { noteBlockId, notePageId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import { useNotesClient, useNotesVersion } from "../../core/features/notes/NotesClient";
import {CoreNoteWorkspace as NotesWorkspace} from "../../core/public/noteDocuments";
import type {WorkspaceClosePort as NoteWorkspacePort} from "../../core/contracts/workbench";
import { hasLegacyReadingRecoveryDraft, migrateReadingNoteRecoveryDraft } from "../../core/features/notes/workspace/noteRecovery";
import "../../core/features/notes/reading-notes/ReadingNoteFloatingPanel.css";
import { openNoteDocumentTab } from "../../core/features/notes/workspace/openNoteDocumentTab";
import { LITERATURE_NOTE_TRASH_EVENT, type LiteratureNoteTrashRequest } from "../../core/features/notes/reading-notes/literatureNotePanelActions";
import { notify } from "../../core/services/notifications";
import { READING_NOTE_PANEL_TRANSFER, type ReadingNotePanelTransferRequest } from "../../core/features/notes/reading-notes/readingNotePanelTransfer";

/** App-owned non-modal host; route changes do not own the document session. */
export function ReadingNoteFloatingPanel(): React.JSX.Element | null {
  const client = useNotesClient(), version = useNotesVersion();
  const [note, setNote] = useState<ReadingNoteBlock>();
  const [literatureDocument, setLiteratureDocument] = useState<NoteDocumentSnapshot>();
  const literaturePage = literatureDocument?.page;
  const [annotationKind, setAnnotationKind] = useState<"concept" | "statement">();
  const literaturePageRef = useRef(literaturePage); literaturePageRef.current = literaturePage;
  const noteRef = useRef(note); noteRef.current = note;
  const setError = useCallback((message: string | undefined) => { if (message) notify.error(message); }, []);
  const [legacyCollision, setLegacyCollision] = useState(false);
  const [recoverySession, setRecoverySession] = useState(0);
  const workspace = useRef<NoteWorkspacePort>(null), panel = useRef<HTMLElement>(null);
  const pending = useRef(false), alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const focusEditor = (): void => {
    const editor = panel.current?.querySelector<HTMLElement>(".bn-editor[contenteditable='true'], .bn-editor [contenteditable='true']");
    (editor ?? panel.current)?.focus({ preventScroll: true });
  };
  useEffect(() => {
    const transfer = (event: Event): void => {
      const request = (event as CustomEvent<ReadingNotePanelTransferRequest>).detail;
      if (!request || (request.pageId !== noteRef.current?.pageId && request.pageId !== literaturePageRef.current?.id)) return;
      event.preventDefault();
      if (pending.current) { request.resolve(false); return; }
      pending.current = true;
      void (async () => {
        try {
          const ready = !workspace.current || await workspace.current.prepareClose();
          if (ready && alive.current) { setNote(undefined); setLiteratureDocument(undefined); setLegacyCollision(false); }
          request.resolve(ready && alive.current);
        } catch (reason) { setError(reason instanceof Error ? reason.message : "笔记编辑状态尚未保存"); request.resolve(false); }
        finally { pending.current = false; }
      })();
    };
    window.addEventListener(READING_NOTE_PANEL_TRANSFER, transfer);
    return () => window.removeEventListener(READING_NOTE_PANEL_TRANSFER, transfer);
  }, [setError]);
  const open = useCallback(async (id: string, kind: "reading" | "literature" | "concept" | "statement" = "reading") => {
    if (pending.current) return;
    if ((kind === "reading" && (noteRef.current?.id === id || noteRef.current?.pageId === id)) || (kind !== "reading" && literaturePageRef.current?.id === id)) { focusEditor(); return; }
    pending.current = true;
    try {
      if (workspace.current && !await workspace.current.prepareClose()) return;
      if (kind !== "reading") {
        const target = await client.gateway.loadDocument(notePageId(id));
        if (alive.current) { setNote(undefined); setLiteratureDocument(target); setAnnotationKind(kind === "literature" ? undefined : kind); setLegacyCollision(false); setError(undefined); }
        return;
      }
      const target = await client.gateway.getReadingNote(noteBlockId(id));
      if (!target.pageId) throw new Error("阅读笔记尚未迁移为文档。");
      const migration = migrateReadingNoteRecoveryDraft(target.id, target.pageId);
      if (alive.current) { setLiteratureDocument(undefined); setNote(target); setLegacyCollision(migration === "collision" || hasLegacyReadingRecoveryDraft(target.id, target.pageId)); setError(undefined); }
    } catch (reason: unknown) { if (alive.current) setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { pending.current = false; }
  }, [client]);
  useEffect(() => {
    const listener = (event: Event) => { const id = (event as CustomEvent<unknown>).detail; if (typeof id === "string") void open(id); };
    const literatureListener = (event: Event) => { const id = (event as CustomEvent<unknown>).detail; if (typeof id === "string") void open(id, "literature"); };
    const annotationListener = (event: Event) => {
      const value=(event as CustomEvent<{pageId?:unknown;kind?:unknown}>).detail;
      if(value && typeof value.pageId === "string" && (value.kind === "concept" || value.kind === "statement"))void open(value.pageId,value.kind);
    };
    const literatureDeleted = (event: Event) => { if ((event as CustomEvent<unknown>).detail === literaturePageRef.current?.id) { setLiteratureDocument(undefined); setError(undefined); } };
    const deleted = (event: Event) => {
      if ((event as CustomEvent<unknown>).detail === noteRef.current?.id) { setNote(undefined); setError(undefined); setLegacyCollision(false); }
    };
    window.addEventListener("research-assistant:open-reading-note", listener);
    window.addEventListener("research-assistant:open-literature-note", literatureListener);
    window.addEventListener("research-assistant:open-annotation-note", annotationListener);
    window.addEventListener("research-assistant:literature-note-deleted", literatureDeleted);
    window.addEventListener("research-assistant:reading-note-deleted", deleted);
    return () => { window.removeEventListener("research-assistant:open-annotation-note", annotationListener); window.removeEventListener("research-assistant:open-literature-note", literatureListener); window.removeEventListener("research-assistant:literature-note-deleted", literatureDeleted); window.removeEventListener("research-assistant:open-reading-note", listener); window.removeEventListener("research-assistant:reading-note-deleted", deleted); };
  }, [open]);
  useEffect(() => {
    const listener = (event: Event): void => {
      const request = (event as CustomEvent<LiteratureNoteTrashRequest>).detail;
      if (!request || typeof request.pageId !== "string" || typeof request.resolve !== "function" || typeof request.reject !== "function") return;
      event.preventDefault();
      if (pending.current) { request.reject(new Error("笔记操作尚未完成，请稍后重试。")); return; }
      pending.current = true;
      void (async () => {
        try {
          const isOpen = literaturePageRef.current?.id === request.pageId;
          if (isOpen && workspace.current && !await workspace.current.prepareClose(true)) throw new Error("请先保存或解决当前笔记草稿的冲突，再删除。草稿已保留。");
          const current = await client.gateway.loadDocument(request.pageId);
          await client.mutate(gateway => gateway.setPageStatus({ id: request.pageId, status: "trashed", expectedRevision: current.page.revision }));
          if (isOpen && alive.current) { setLiteratureDocument(undefined); setError(undefined); }
          request.resolve();
        } catch (reason) { request.reject(reason); }
        finally { pending.current = false; }
      })();
    };
    window.addEventListener(LITERATURE_NOTE_TRASH_EVENT, listener);
    return () => window.removeEventListener(LITERATURE_NOTE_TRASH_EVENT, listener);
  }, [client]);
  useEffect(() => {
    const id = noteRef.current?.id;
    if (!id) return;
    let current = true;
    void client.gateway.getReadingNote(id).then(next => {
      if (current && noteRef.current?.id === id) {
        setNote(next);
        if (next.pageId && hasLegacyReadingRecoveryDraft(next.id, next.pageId)) setLegacyCollision(true);
      }
    })
      .catch(reason => { if (current) setError(reason instanceof Error ? reason.message : String(reason)); });
    return () => { current = false; };
  }, [client, version]);
  async function restoreLegacyDraft() {
    const target = noteRef.current;
    if (!target?.pageId || pending.current) return;
    pending.current = true;
    try {
      if (!await workspace.current?.prepareClose(true)) { setError("请先保存或解决当前文档草稿的版本冲突，再恢复旧阅读草稿。两份草稿均已保留。"); return; }
      if (migrateReadingNoteRecoveryDraft(target.id, target.pageId) === "collision") throw new Error("当前文档仍有恢复草稿，请先核对保存结果。");
      if (alive.current) { setLegacyCollision(hasLegacyReadingRecoveryDraft(target.id, target.pageId)); setRecoverySession(value => value + 1); setError(undefined); }
    } catch (reason: unknown) { if (alive.current) setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { pending.current = false; }
  }
  const pageId = note?.pageId ?? literaturePage?.id;
  const title = literaturePage ? annotationKind === "concept" ? "概念笔记" : annotationKind === "statement" ? "语句笔记" : "文献笔记" : "阅读笔记";
  return <NoteFloatingWindow open={!!pageId} identity={pageId ?? "closed"} title={title} label={`${title}编辑面板`}
    closeLabel="关闭阅读笔记" resizeLabel="阅读笔记" panelRef={panel} session={{
      prepareClose: async () => !workspace.current || await workspace.current.prepareClose(),
      openTab: accepted => pageId ? openNoteDocumentTab({kind: "page", pageId}, accepted) : Promise.resolve(false),
      dismiss: () => { setNote(undefined); setLiteratureDocument(undefined); setError(undefined); setLegacyCollision(false); },
      reportError: reason => setError(reason instanceof Error ? reason.message : String(reason)),
      beginOperation: () => { if (pending.current) return false; pending.current = true; return true; },
      endOperation: () => { pending.current = false; },
    }}>
    {legacyCollision && <div className="reading-note-recovery-notice" role="status">当前文档与旧阅读草稿均已保留。保存当前文档后可恢复旧草稿进行版本比较。<button type="button" onClick={() => void restoreLegacyDraft()}>恢复旧阅读草稿</button></div>}
    {pageId && <NotesWorkspace key={`${pageId}:${recoverySession}`} ref={workspace} embedded compactHeader={!!literaturePage && !!annotationKind} initialPageId={pageId} initialSnapshot={literatureDocument} {...(note ? { readingSource: note } : {})} />}
  </NoteFloatingWindow>;
}
