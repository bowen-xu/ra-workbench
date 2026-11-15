import type { NoteDocumentGateway, ReadingNoteGateway } from "@research-assistant/data/notes";

/** A single-document port. Library creation, trees, favorites and bulk changes
 * stay outside this public workbench contract. Native authorization is unchanged.
 */
export type NoteDocumentAccess = Pick<NoteDocumentGateway,
  "loadDocument" | "saveDocument" | "listRevisions" | "loadRevision" |
  "restoreRevision" | "setRevisionPinned" | "resolveConflict"
> & Pick<ReadingNoteGateway, "getReadingNote">;
