import {NotesGatewayError} from "@research-assistant/data/notes";

export function catalogErrorMessage(reason: unknown): string {
  return reason instanceof NotesGatewayError ? `${reason.code}: ${reason.message}${reason.currentRevision===undefined?"":`（当前代次 ${reason.currentRevision}）`}`
    : reason instanceof Error ? reason.message : String(reason);
}
