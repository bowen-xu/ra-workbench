import { invoke } from "../../../runtime/platformCore";
import type {
  NoteDocumentV1,
  NoteLink,
  NotePage,
  NotePageId,
  NoteRevisionId,
  ReadingNoteBlock,
} from "@research-assistant/core/notes";
import {
  decodeNotePageCatalogPage,
  type NotePageCatalogGateway,
  NotesGatewayError,
  type NoteDocumentSaveRequest,
  type NoteDocumentSnapshot,
  type NoteLinkDraft,
  type NoteListQuery,
  type NotePageDraft,
  type NotePageMetadataUpdate,
  type NotePageMoveRequest,
  type NotePermanentDeleteReport,
  type NotePermanentDeleteRequest,
  type NoteRevisionSummary,
  type NoteSearchPage,
  type NoteSearchQuery,
  type NoteStatusUpdate,
  type NotesErrorCode,
  type NotesGateway,
  type NoteAsset,
  type NoteAssetImportRequest,
  type NoteExportReport,
  type NoteExportRequest,
  type NoteTag,
  type ReadingNoteDraft,
  type ReadingNoteListQuery,
  type ReadingNoteUpdate,
} from "@research-assistant/data/notes";

interface CommandErrorShape {
  readonly code?: string;
  readonly message?: string;
  readonly currentRevision?: number;
}

function commandError(reason: unknown): NotesGatewayError {
  if (typeof reason === "object" && reason !== null) {
    const value = reason as CommandErrorShape;
    return new NotesGatewayError(
      (value.code ?? "operation_unknown") as NotesErrorCode,
      value.message ?? "笔记操作失败",
      value.currentRevision,
    );
  }
  return new NotesGatewayError("operation_unknown", reason instanceof Error ? reason.message : String(reason));
}

async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(name, args);
  } catch (reason: unknown) {
    throw commandError(reason);
  }
}

export const tauriNotePageCatalogGateway: NotePageCatalogGateway = {
  queryNotePageCatalog: async (query, options) => {
    // Each native dispatch prepares at most 100 old metadata rows. Cancellation
    // stops before the next dispatch; committed progress remains resumable.
    while (true) {
      options?.signal?.throwIfAborted();
      try {
        const result = await command("query_note_page_catalog", {query});
        options?.signal?.throwIfAborted();
        return decodeNotePageCatalogPage(result, query.limit ?? 20);
      } catch (reason: unknown) {
        options?.signal?.throwIfAborted();
        if (!(reason instanceof NotesGatewayError) || reason.code !== "catalog_preparing") throw reason;
      }
    }
  },
};

export const tauriNotesGateway: NotesGateway = {
  pageCatalog: tauriNotePageCatalogGateway,
  loadIndexDocuments: pageIds => command("load_note_index_documents", {pageIds}),
  listPageMetadata: ids => command("list_note_page_metadata", {ids}),
  applyMetadataBatch: request => command("apply_note_metadata_batch", {request}),
  replayMetadataBatch: request => command("replay_note_metadata_batch", {request}),
  commitNavigation: request => command("commit_note_navigation", { request }),
  listFavoritePageIds: () => command("list_note_favorites"),
  reorderFavorites: request => command("reorder_note_favorites", { request }),
  resolveConflict: request => command("resolve_note_conflict", { request }),
  loadRevision: (pageId, revisionId) => command("load_note_revision", { pageId, revisionId }),
  listLiteratureNotes: paperId => command("list_literature_notes", { paperId }),
  createLiteratureNote: paperId => command("create_literature_note", { paperId }),
  listPages(query?: NoteListQuery): Promise<readonly NotePage[]> {
    return command("list_note_pages", { query });
  },
  createPage(draft: NotePageDraft): Promise<NoteDocumentSnapshot> {
    return command("create_note_page", { draft });
  },
  updatePage(request: NotePageMetadataUpdate): Promise<NotePage> {
    return command("update_note_page", { request });
  },
  movePage(request: NotePageMoveRequest): Promise<NotePage> {
    return command("move_note_page", { request });
  },
  duplicatePage(pageId: NotePageId): Promise<NoteDocumentSnapshot> {
    return command("duplicate_note_page", { pageId });
  },
  async setPageStatus(request: NoteStatusUpdate<NotePageId>): Promise<NotePage> {
    if (!["active", "archived", "trashed"].includes(request.status)) throw new NotesGatewayError("invalid_input", "笔记状态无效");
    const receipt = await tauriNotesGateway.applyMetadataBatch({operationId: crypto.randomUUID(),
      action: request.status === "active" ? "restore" : request.status === "trashed" ? "trash" : "archive",
      items: [{id: request.id, expectedRevision: request.expectedRevision}]});
    const failure = receipt.failures[0];
    if (failure) throw new NotesGatewayError(failure.code as NotesErrorCode, failure.message, failure.currentRevision);
    const page = receipt.pages.find(page => page.id === request.id) ?? (await tauriNotesGateway.listPageMetadata([request.id]))[0];
    if (!page) throw new NotesGatewayError("not_found", "笔记不存在");
    return page;
  },
  permanentlyDeletePage(request: NotePermanentDeleteRequest<NotePageId>): Promise<NotePermanentDeleteReport> {
    return command("permanently_delete_note_page", { request });
  },
  loadDocument(pageId: NotePageId): Promise<NoteDocumentSnapshot> {
    return command("load_note_document", { pageId });
  },
  saveDocument(request: NoteDocumentSaveRequest): Promise<NoteDocumentSnapshot> {
    return command("save_note_document", { request });
  },
  listRevisions(pageId: NotePageId): Promise<readonly NoteRevisionSummary[]> {
    return command("list_note_revisions", { pageId });
  },
  setRevisionPinned(pageId: NotePageId, revisionId: NoteRevisionId, isPinned: boolean): Promise<NoteRevisionSummary> {
    return command("set_note_revision_pinned", { pageId, revisionId, isPinned });
  },
  restoreRevision(pageId: NotePageId, revisionId: NoteRevisionId, expectedRevision: number): Promise<NoteDocumentSnapshot> {
    return command("restore_note_revision", { pageId, revisionId, expectedRevision });
  },
  listReadingNotes(query?: ReadingNoteListQuery): Promise<readonly ReadingNoteBlock[]> {
    return command("list_reading_notes", { query });
  },
  getReadingNote(id): Promise<ReadingNoteBlock> {
    return command("get_reading_note", { id });
  },
  createReadingNote(draft: ReadingNoteDraft): Promise<ReadingNoteBlock> {
    return command("create_reading_note", { draft });
  },
  updateReadingNote(request: ReadingNoteUpdate): Promise<ReadingNoteBlock> {
    return command("update_reading_note", { request });
  },
  setReadingNoteStatus(request): Promise<ReadingNoteBlock> {
    return command("set_reading_note_status", { request });
  },
  permanentlyDeleteReadingNote(request): Promise<NotePermanentDeleteReport> {
    return command("permanently_delete_reading_note", { request });
  },
  upsertLink(draft: NoteLinkDraft): Promise<NoteLink> {
    return command("upsert_note_link", { draft });
  },
  removeLink(id: string): Promise<void> {
    return command("remove_note_link", { id });
  },
  listSourceLabels: pageIds => command("list_note_source_labels", {pageIds}),
  listOutgoing(pageId: NotePageId): Promise<readonly NoteLink[]> {
    return command("list_note_links", { pageId });
  },
  listBacklinks(targetKind, targetId): Promise<readonly NoteLink[]> {
    return command("list_note_backlinks", { targetKind, targetId });
  },
  searchNotes(query: NoteSearchQuery): Promise<NoteSearchPage> {
    return command("search_notes", { query });
  },
  rebuildNoteIndex(): Promise<void> {
    return command("rebuild_note_index");
  },
  cancelNoteIndexRebuild(): Promise<boolean> {
    return command("cancel_note_index_rebuild");
  },
  getNoteIndexState(): Promise<NoteSearchPage["indexState"]> {
    return command("get_note_index_state");
  },
  importNoteAsset(request: NoteAssetImportRequest): Promise<NoteAsset> {
    return command("import_note_asset", { request });
  },
  cancelNoteAssetImport(): Promise<boolean> {
    return command("cancel_note_asset_import");
  },
  async readNoteAsset(id: string): Promise<Uint8Array> {
    return Uint8Array.from(await command<readonly number[]>("read_note_asset", { id }));
  },
  exportNotes(request: NoteExportRequest): Promise<NoteExportReport> {
    return command("export_notes", { request });
  },
  cancelNoteExport(): Promise<boolean> {
    return command("cancel_note_export");
  },
  listNoteTags(): Promise<readonly NoteTag[]> {
    return command("list_note_tags");
  },
  ensureNoteTag(name: string): Promise<NoteTag> {
    return command("ensure_note_tag", { name });
  },
};

export type { NoteDocumentV1 };
