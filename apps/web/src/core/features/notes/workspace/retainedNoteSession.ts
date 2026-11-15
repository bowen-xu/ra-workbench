import type { NoteLink } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import type { NoteSaveStatus } from "../editor/noteSaveController";
import type { AppNoteHbsSession } from "./appNoteHbsSession";
import type { HbsSession } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";

/** One open tab owns its save and editor lifetime, including while hidden. */
export interface RetainedNoteSession {
  snapshot: NoteDocumentSnapshot;
  editorSessionId: number;
  saveStatus: NoteSaveStatus;
  saveMessage: string | undefined;
  outgoing: readonly NoteLink[];
  incoming: readonly NoteLink[];
  controller: AppNoteHbsSession;
  hbs: HbsSession<NoteDocumentV1>;
  unregisterText: (() => void) | undefined;
  recoveryTimer: ReturnType<typeof setTimeout> | undefined;
  invalidated: boolean;
  observedRevision: number;
}
