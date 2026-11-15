import { NotesGatewayError } from "@research-assistant/data/notes";

import type {NoteOpenFailure} from "@research-assistant/workbench-sdk/note-documents";
export type {NoteOpenFailure} from "@research-assistant/workbench-sdk/note-documents";

/** Only the document lookup can establish that a page is missing. */
export function noteOpenFailure(reason: unknown, missingDocument = false): NoteOpenFailure {
  const code = reason instanceof NotesGatewayError ? reason.code : "note_open_failed";
  const message = reason instanceof Error ? reason.message : typeof reason === "string" ? reason : "读取笔记时发生未知错误。";
  const revision = reason instanceof NotesGatewayError && reason.currentRevision !== undefined
    ? `（当前版本 ${reason.currentRevision}）` : "";
  return { kind: missingDocument && code === "not_found" ? "missing" : "failed", code, message: message + revision };
}
