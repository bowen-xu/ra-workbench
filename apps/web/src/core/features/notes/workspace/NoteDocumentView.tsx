import { readingNoteDisplayTitle } from "@research-assistant/core/notes";
import "./NoteRecoveryDialogs.css";
import { useEffect, useRef, useState } from "react";

import { ReadingNoteDocumentHeader } from "../reading-notes/ReadingNoteDocumentHeader";
import { NoteEditor } from "../editor/NoteEditor";
import { NoteRelations } from "../links/NoteRelations";
import { NoteHistoryDialog } from "./NoteHistoryDialog";
import { NotePageHeader } from "./NotePageHeader";
import { NoteLifecycleActions } from "./NoteLifecycleActions";

import type {NoteDocumentViewProps} from "@research-assistant/workbench-sdk/note-presentation";

export function NoteDocumentView(props: NoteDocumentViewProps): React.JSX.Element {
  const [diffControlsTarget, setDiffControlsTarget] = useState<HTMLDivElement | null>(null);
  const [showRelations, setShowRelations] = useState(false);
  const moreButton = useRef<HTMLButtonElement>(null);
  const { page } = props.snapshot;
  const lifecycleActions = <NoteLifecycleActions page={page} disabled={!!props.lifecycleBusy} onRestore={props.onRestore} onPermanentlyDelete={props.onPermanentlyDelete} />;
  useEffect(() => { setShowRelations(false); }, [page.id]);
  return <>
    {props.readingSource || props.compactHeader ? <ReadingNoteDocumentHeader
      lifecycleActions={lifecycleActions}
      pageId={page.id}
      exportTitle={page.title}
      onFavoriteChange={favorite => props.onUpdatePage({ isFavorite: favorite })}
      favorite={page.isFavorite}
      onDiffControlsTarget={props.onDiffControlsTarget ?? setDiffControlsTarget}
      {...(props.readingSource ? {note:props.readingSource} : {contextTitle:page.title})}
      readOnly={page.status !== "active" || props.saveStatus === "conflict"}
      {...(props.onReadingColorChange === undefined || page.status !== "active" || props.saveStatus === "conflict" ? {} : { onColorChange: props.onReadingColorChange })}
      saveStatus={props.saveStatus}
      saveMessage={props.saveMessage}
      onOpenHistory={props.onOpenHistory}
      onTrash={props.onTrash}
      onRetrySave={props.onRetrySave}
    /> : <NotePageHeader
      lifecycleActions={lifecycleActions}
      moreButtonRef={moreButton}
      onOpenRelations={() => { props.onCloseHistory(); setShowRelations(true); }}
      onDiffControlsTarget={props.onDiffControlsTarget ?? setDiffControlsTarget}
      onArchive={props.onArchive}
      onDuplicate={props.onDuplicate}
      onFavoriteChange={(isFavorite) => props.onUpdatePage({ isFavorite })}
      onOpenHistory={() => { setShowRelations(false); props.onOpenHistory(); }}
      onOpenPage={props.onOpenPage}
      onRetrySave={props.onRetrySave}
      onTrash={props.onTrash}
      page={page}
      pages={props.pages}
      saveMessage={props.saveMessage}
      saveStatus={props.saveStatus}
    />}
    {!props.retainEditor && <NoteEditor
      pageTitle={props.readingSource || props.compactHeader ? undefined : {value: page.title}}
      diffControlsTarget={diffControlsTarget}
      sourceNavigation={props.sourceNavigation}
      isReadOnly={page.status !== "active"}
      conflictLocked={props.saveStatus === "conflict"}
      document={props.snapshot.document}
      key={`${page.id}:${props.editorSessionId}`}
      onAssetStatus={props.onAssetStatus}
      onChange={props.onDocumentChange}
      onInstallPort={props.onInstallPort}
      queryNoteTargets={props.queryNoteTargets}
      pageTargets={props.pages.filter(candidate => candidate.status !== "trashed").map(candidate => {
        const reading = props.readingNotes.find(note => note.pageId === candidate.id);
        return { id: candidate.id, title: reading ? readingNoteDisplayTitle(reading, candidate.title) : candidate.title };
      })}
      readingTargets={props.readingNotes.map((reading) => ({ id: reading.id, label: reading.source.paperTitle, quote: reading.source.quoteExact }))}
      researchTargets={props.researchTargets}
    />}
    {showRelations && props.readingSource === undefined && !props.compactHeader && <NoteRelations
      finalFocus={moreButton}
      onClose={() => setShowRelations(false)}
      backlinks={props.backlinks}
      onOpenPage={(id) => { setShowRelations(false); props.onOpenPage(id); }}
      onOpenReading={(id) => { setShowRelations(false); props.onOpenReading(id); }}
      outgoing={props.outgoingLinks}
      pages={props.pages}
      readPageMetadata={props.onReadPageMetadata}
    />}
    {props.showHistory && <NoteHistoryDialog revisions={props.revisions} onClose={props.onCloseHistory} onLoad={props.onLoadRevision}
      onRestore={props.onRestoreRevision} onPin={props.onToggleRevisionPinned} readOnly={page.status !== "active" || props.saveStatus === "conflict"} />}
  </>;
}
