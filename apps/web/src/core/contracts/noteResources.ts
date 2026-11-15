import type {NoteDocumentSnapshot, NotesGateway} from "@research-assistant/data/notes";
import type {NoteResourceObservation} from "@research-assistant/workbench-sdk/note-resources";
export type {NoteResourceChange, NotePageSource, NoteHistoryCommand, NoteResourceGateway, NoteResourceObservation, NoteResourceAccess} from "@research-assistant/workbench-sdk/note-resources";

/** PDF annotation metadata shares the same note owner but does not receive
 * library mutation, assets, document recovery internals or the client class. */
export type NoteAnnotationGateway = Pick<NotesGateway,
  "listReadingNotes" | "getReadingNote" | "createReadingNote" |
  "updateReadingNote" | "setReadingNoteStatus"
>;
export interface NoteAnnotationAccess extends NoteResourceObservation {
  readonly gateway: NoteAnnotationGateway;
}
export interface LiteratureNoteAccess extends NoteResourceObservation {
  readonly list: (paperId: string) => ReturnType<NotesGateway["listLiteratureNotes"]>;
  readonly create: (paperId: string) => Promise<NoteDocumentSnapshot>;
}
