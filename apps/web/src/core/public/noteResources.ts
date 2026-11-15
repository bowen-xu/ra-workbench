import { useNotesClient, useNotesVersion } from "../features/notes/NotesClient";
import { noteResourceAccess, noteAnnotationAccess, literatureNoteAccess } from "../features/notes/noteResourceAccess";
import type { NoteResourceAccess, NoteAnnotationAccess, LiteratureNoteAccess } from "../contracts/noteResources";

export type { NoteResourceAccess, NoteResourceObservation, NoteResourceChange, NoteResourceGateway,
  NoteAnnotationAccess, NoteAnnotationGateway, LiteratureNoteAccess, NoteHistoryCommand, NotePageSource } from "../contracts/noteResources";

export function useNoteResources(): NoteResourceAccess { return noteResourceAccess(useNotesClient()); }
export function useNoteAnnotations(): NoteAnnotationAccess { return noteAnnotationAccess(useNotesClient()); }
export function useLiteratureNotes(): LiteratureNoteAccess { return literatureNoteAccess(useNotesClient()); }
export function useNoteResourceVersion(): number {
  return useNotesVersion();
}

/** Shared document/list lifecycle transaction, receipts, and undo ownership. */
export {applyNotePageStatus, commitNoteMetadata, publishNoteMetadataReceipt} from "../features/notes/noteMetadataActions";
