import { useEffect, useState } from "react";
import { noteBlockId, readingNoteDisplayTitle, type NoteReferenceContentV1 } from "@research-assistant/core/notes";
import { useNotesClient, useNotesVersion } from "../NotesClient";
import { PageLinkBlock } from "../links/PageLinkBlock";

/** Read historical standalone references through the current shared link UI.
 * Identity and quote snapshots stay in the document codec, not in a private card. */
export function ReferenceLinkPreview({ targetKind, targetId, labelSnapshot }: Pick<NoteReferenceContentV1,
  "targetKind" | "targetId" | "labelSnapshot">): React.JSX.Element {
  const client = useNotesClient(), version = useNotesVersion();
  const [target, setTarget] = useState<{ id: string; title: string; unavailable: boolean }>();
  useEffect(() => {
    if (targetKind !== "reading_note" || !targetId) return;
    let active = true;
    void client.gateway.getReadingNote(noteBlockId(targetId)).then(note => {
      if (active) setTarget({ id: targetId, title: readingNoteDisplayTitle(note, labelSnapshot), unavailable: note.status === "trashed" });
    }).catch(() => {
      if (active) setTarget({ id: targetId, title: labelSnapshot, unavailable: true });
    });
    return () => { active = false; };
  }, [client, version, targetKind, targetId, labelSnapshot]);
  const current = targetKind === "reading_note" && target?.id === targetId ? target : undefined;
  const title = current?.title || labelSnapshot;
  return <PageLinkBlock targetId={targetId} title={title} navigationLabel={`打开引用：${title}`}
    disabled={!targetId || !!current?.unavailable} unavailable={current?.unavailable ?? false}
    onOpen={() => {
      const event = targetKind === "reading_note" ? "research-assistant:open-reading-note"
        : targetKind === "note_page" ? "research-assistant:open-note-page" : "research-assistant:open-research-target";
      window.dispatchEvent(new CustomEvent(event, { detail: targetKind === "reading_note" || targetKind === "note_page"
        ? targetId : { kind: targetKind, id: targetId, labelSnapshot } }));
    }} />;
}
