import { useCreateBlockNote } from "@blocknote/react";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { NoteEditorSurface } from "../../../components/block-editor/NoteEditorSurface";
import { blockEditorDefaults } from "../../../components/block-editor";
import { noteDocumentToBlockNote } from "./blockNoteAdapter";
import { noteEditorSchema, type NoteEditorInstance } from "./noteEditorSchema";
import { noteEditorComponents } from "./noteLinkComponents";

/** Frozen snapshot: production schema/renderers, without a live save/review port. */
export function NoteSnapshotView({ document }: {
  readonly document: NoteDocumentV1;
}): React.JSX.Element {
  const editor = useCreateBlockNote({ ...blockEditorDefaults, schema: noteEditorSchema, ...(document.blocks.length ? { initialContent: noteDocumentToBlockNote(document) } : {}) }) as unknown as NoteEditorInstance;
  return <NoteEditorSurface editor={editor} className="note-editor note-snapshot" onChange={() => {}} readOnly components={noteEditorComponents} ariaLabel="笔记版本正文" />;
}
