import { createEmptyNoteDocument, noteBlockId, notePageId, type NotePage, type NotePageId } from "@research-assistant/core/notes";
import { NotesGatewayError, type NoteDocumentSaveRequest, type NoteDocumentSnapshot, type NoteConflictResolutionRequest, type NoteConflictResolutionReceipt } from "@research-assistant/data/notes";
import { prepareConflictBlocks } from "../editor/noteConflictMerge";
import { now, readState, writeState, type BrowserNotesState, type BrowserRevision } from "./browserNotesStorage";

type Save = (state: BrowserNotesState, request: NoteDocumentSaveRequest) => BrowserNotesState;
type Snapshot = (state: BrowserNotesState, pageId: NotePageId) => NoteDocumentSnapshot;
function assertRevision(current: number, expected: number) {
  if (current !== expected) throw new NotesGatewayError("revision_conflict", "笔记版本已变化", current);
}

export function pruneBrowserRevisions(
  revisions: readonly BrowserRevision[],
  timestamp: number = Date.now(),
): readonly BrowserRevision[] {
  const day = 24 * 60 * 60 * 1000;
  return revisions.filter((revision) => {
    if (revision.isPinned) return true;
    const createdAt = Date.parse(revision.createdAt);
    if (!Number.isFinite(createdAt)) return true;
    const retentionDays = revision.reason === "autosave" ? 14 : 30;
    return timestamp - createdAt <= retentionDays * day;
  });
}


export async function resolveBrowserConflict(request: NoteConflictResolutionRequest, saveBrowserDocument: Save, snapshot: Snapshot): Promise<NoteConflictResolutionReceipt> {
    let state = await readState();
    const prior = state.conflictResolutions?.[request.requestId];
    if (prior) {
      if (JSON.stringify(prior.request) !== JSON.stringify(request)) throw new NotesGatewayError("invalid_input", "冲突操作 ID 已用于不同内容");
      return prior.receipt;
    }
    const current = snapshot(state, request.document.pageId);
    assertRevision(current.page.revision, request.expectedRevision);
    let copy: NoteDocumentSnapshot | undefined;
    if (request.copy) {
      const id = notePageId(crypto.randomUUID());
      const page: NotePage = { ...current.page, id, title: `${Array.from(current.page.title).slice(0, 230).join("")}（冲突副本）`, status: "active", revision: 0, position: state.pages.length, createdAt: now(), updatedAt: now() };
      const { parentPageId: _parent, ...topLevel } = page;
      state = { ...state, pages: [...state.pages, topLevel], documents: { ...state.documents, [id]: createEmptyNoteDocument(id, noteBlockId(crypto.randomUUID())) } };
      state = saveBrowserDocument(state, { document: { ...request.document, pageId: id, blocks: prepareConflictBlocks(request.document.blocks, current.document, true) }, expectedRevision: 0, saveReason: "structure" });
      copy = snapshot(state, id);
    }
    state = saveBrowserDocument(state, { document: request.copy ? current.document : request.document, expectedRevision: request.expectedRevision, saveReason: "manual" });
    const receipt = { original: snapshot(state, current.page.id), ...(copy ? { copy } : {}) };
    state = { ...state, conflictResolutions: { ...state.conflictResolutions, [request.requestId]: { request, receipt } } };
    await writeState(state);
    return receipt;
  }
