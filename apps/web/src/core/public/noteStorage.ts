import { getNotesGateway } from "../features/notes/gateways";
import type { NoteManagementStorage, ReadingNoteCreationStorage } from "../contracts/noteStorage";

export type { NoteCatalogStorage, NoteLibraryTransferStorage, NoteManagementStorage, ReadingNoteCreationStorage } from "../contracts/noteStorage";
export { loadBrowserNoteCatalogRuntime } from "../features/notes/gateways/browserNoteCatalogRuntime";

/** Reuse the Core-owned adapter. This grants neither a new database nor a raw storage handle. */
export function getNoteManagementGateway(): NoteManagementStorage { return getNotesGateway(); }
export function getReadingNoteCreationGateway(): ReadingNoteCreationStorage { return getNotesGateway(); }
