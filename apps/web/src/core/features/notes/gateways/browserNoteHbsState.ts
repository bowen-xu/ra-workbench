import type { DocumentCheckpoint, DocumentCommit, DocumentSnapshot } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";

/** Shares the existing notes transaction; no second document store or writer. */
export interface BrowserNoteHbsRecord {
  readonly baseline?: NoteDocumentV1;
  readonly checkpoint?: DocumentCheckpoint<NoteDocumentV1>;
  readonly preservationRevision?: number;
  readonly receipts?: Readonly<Record<string, {
    readonly request: DocumentCommit<NoteDocumentV1>;
    readonly reply: { readonly type: "receipt"; readonly requestId: string; readonly snapshot: DocumentSnapshot<NoteDocumentV1> };
    readonly creation?: {readonly favorite: boolean; readonly source?: import("@research-assistant/data/notes").NoteDocumentSaveRequest};
  }>>;
}
export const browserHbsRecordKey = (id: string): string => `$hbs:${id}`;
