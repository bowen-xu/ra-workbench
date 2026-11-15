import { useEffect, useState } from "react";
import { noteBlockId, notePageId, readingNoteDisplayTitle } from "@research-assistant/core/notes";
import { useNotesClient, useNotesVersion } from "../NotesClient";
import { PageLinkBlock } from "../links/PageLinkBlock";

export function PageLinkPreview({ targetId, labelSnapshot }: { readonly targetId: string; readonly labelSnapshot: string }): React.JSX.Element {
  const client = useNotesClient(), version = useNotesVersion();
  const [target, setTarget] = useState<{ id: string; title: string; unavailable: boolean }>();
  useEffect(() => {
    if (!targetId) return;
    let active = true;
    void client.gateway.loadDocument(notePageId(targetId)).then(async snapshot => {
      let title = snapshot.page.title;
      try {
        const source = await client.gateway.getReadingNote(noteBlockId(targetId));
        title = readingNoteDisplayTitle(source, title);
      } catch { /* Ordinary pages have no reading source. */ }
      if (active) setTarget({ id: targetId, title, unavailable: snapshot.page.status === "trashed" });
    }).catch(() => { if (active) setTarget({ id: targetId, title: labelSnapshot, unavailable: true }); });
    return () => { active = false; };
  }, [client, version, targetId, labelSnapshot]);
  const current = target?.id === targetId ? target : undefined;
  return <PageLinkBlock targetId={targetId} title={current?.title || labelSnapshot} disabled={!targetId || !!current?.unavailable} unavailable={current?.unavailable ?? false}
    onOpen={() => window.dispatchEvent(new CustomEvent("research-assistant:open-note-page", { detail: targetId }))} />;
}
