import type { ReactNode, MutableRefObject } from "react";
import type { Plugin } from "@tiptap/pm/state";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteEditorInstance } from "./noteEditorSchema";
import type { NoteEditorInstallPort } from "./noteEditorInstallPort";
import type { useNoteTextReview } from "./useNoteTextReview";
/** Replace review/persistence ownership, retaining the complete production
 * note editor. A session key is stable for one mounted editor lifetime. */
export interface NoteEditorSession {
  readonly key: string;
  readonly surfaceClassName?: string;
  readonly createHistory: (initial: NoteDocumentV1, canReplayHistory: () => boolean) => Plugin[];
  readonly useReview: typeof useNoteTextReview;
  readonly renderReview: (editor: NoteEditorInstance, port: MutableRefObject<NoteEditorInstallPort | undefined>) => ReactNode;
}
