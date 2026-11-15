import { getNotesGateway } from "../features/notes/gateways";
import type { NoteDocumentAccess } from "../contracts/documents";

export type { NoteDocumentAccess } from "../contracts/documents";
export type { NoteDocumentSnapshot, NoteRevisionSummary } from "@research-assistant/data/notes";

/** Reuse the original adapter and its recovery/authorization path. The public
 * type exposes document operations, not the entire note-library management API.
 */
export function getNoteDocumentGateway(): NoteDocumentAccess {
  return getNotesGateway();
}
