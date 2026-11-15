import type { NotesGateway, ReadingNoteGateway } from "@research-assistant/data/notes";

/** Bounded metadata queries and receipt-based library mutations on Core-owned storage. */
export type NoteCatalogStorage = Pick<NotesGateway,
  "pageCatalog" | "listPageMetadata" | "listFavoritePageIds" | "reorderFavorites" |
  "applyMetadataBatch" | "replayMetadataBatch"
>;

/** Existing whole-library transfer compatibility; uses the same Core store and file adapters. */
export type NoteLibraryTransferStorage = Pick<NotesGateway,
  "exportNotes" | "cancelNoteExport" | "listPages" | "listReadingNotes" |
  "getReadingNote" | "loadDocument" | "listOutgoing"
>;

export type NoteManagementStorage = NoteCatalogStorage & NoteLibraryTransferStorage;

/** A source-linked note creation command; no library enumeration or raw storage access. */
export type ReadingNoteCreationStorage = Pick<ReadingNoteGateway, "createReadingNote">;
