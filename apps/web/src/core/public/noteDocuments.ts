import {createElement, useMemo, type ComponentProps, type RefObject} from "react";
import type {NoteDocumentSession, NoteSessionOptions, NoteAcceptanceOptions, NoteAcceptance, RetainedNoteEditorsProps} from "@research-assistant/workbench-sdk/note-session";
import {useNoteHistoryScope as useHistoryScope} from "../features/notes/editor/useNoteHistoryScope";
import {useNotesClient} from "../features/notes/NotesClient";
import {useHostedNoteDocumentSession as useDocumentSession} from "../features/notes/workspace/noteWorkspaceHostContext";
import {useNoteAcceptance as useAcceptance} from "../features/notes/agent/useNoteAcceptance";
import {useNoteCatalogTargets as useCatalogTargets} from "../features/notes/workspace/useNoteCatalogTargets";
import {RetainedNoteEditors as OwnedRetainedNoteEditors} from "../features/notes/workspace/RetainedNoteEditors";
import {publicNoteSession, noteSessionOwner} from "../features/notes/workspace/publicNoteSessionHandles";

/** Single-document UI and sessions. Library organization and bulk commands belong to note_manager. */
export function useNoteDocumentSession(options: NoteSessionOptions): NoteDocumentSession {
  const session=useDocumentSession(options);
  const saveControllerRef=useMemo(() => Object.freeze({get current() {return session.saveControllerRef.current;}}), [session.saveControllerRef]);
  return {
    snapshot:session.snapshot, setSnapshot:session.setSnapshot, loadState:session.loadState,
    saveStatus:session.saveStatus, saveMessage:session.saveMessage, editorSessionId:session.editorSessionId,
    saveControllerRef, retainedSessions:session.retainedSessions.map(publicNoteSession),
    loadPage:session.loadPage, disposeSession:session.disposeSession, ensureDurable:session.ensureDurable,
    ensureAllDurable:session.ensureAllDurable, saveExplicitly:session.saveExplicitly,
    onDocumentChange:session.onDocumentChange, onRetainedDocumentChange:session.onRetainedDocumentChange,
    getSnapshot:session.getSnapshot, getSelectedPageId:session.getSelectedPageId,
    refreshAfterStructure:session.refreshAfterStructure, deactivate:session.deactivate, retain:session.retain,
    updateMetadata:session.updateMetadata, observePages:session.observePages, rememberLinks:session.rememberLinks,
  };
}
export function useNoteAcceptance(options: NoteAcceptanceOptions): NoteAcceptance {
  const client=useNotesClient();
  return useAcceptance({...options,notes:client.gateway});
}
export function useNoteCatalogTargets(version: number) {
  return useCatalogTargets(useNotesClient(),version);
}
/** The Core provider remains the history owner; plugins do not pass its private class. */
export function useNoteHistoryScope(root: RefObject<HTMLElement|null>, active=true): void {
  useHistoryScope(useNotesClient().history,root,active);
}
export { NoteDocumentView } from "../features/notes/workspace/NoteDocumentView";
export {CoreNoteWorkspace} from "../features/notes/workspace/CoreNoteWorkspace";
export {CoreNoteWorkspaceHost} from "../features/notes/workspace/CoreNoteWorkspaceHost";
export {useCoreNoteWorkspaceHost} from "../features/notes/workspace/noteWorkspaceHostContext";
export type {CoreNoteWorkspaceProps} from "../features/notes/workspace/CoreNoteWorkspace";
export {useNoteDocumentMetadata} from "../features/notes/workspace/useNoteDocumentMetadata";
export { NoteDocumentPending } from "../features/notes/workspace/NoteDocumentPending";
/** Public metadata handles return to the original Core editor and save owner. */
export function RetainedNoteEditors(props: RetainedNoteEditorsProps & {
  // Existing App-owned diagnostics can observe the editor. This composition
  // callback is deliberately absent from the independently consumed SDK.
  readonly onEditorReady?: NonNullable<ComponentProps<typeof OwnedRetainedNoteEditors>["onEditorReady"]>;
}): React.JSX.Element {
  return createElement(OwnedRetainedNoteEditors, {...props, sessions: props.sessions.map(noteSessionOwner)});
}
export { useNotesShortcuts } from "../features/notes/workspace/useNotesShortcuts";
export {useHostedNoteWorkspaceTabs as useNoteWorkspaceTabs} from "../features/notes/workspace/noteWorkspaceHostContext";
export { useNoteLifecycleNotice } from "../features/notes/workspace/useNoteLifecycleNotice";
export { clearNoteRecoveryDraft } from "../features/notes/workspace/noteRecovery";
export { synchronizeNoteReview, holdTextEditor, holdNoteEditorInput, getNoteReviewSaveState, isTextEditorHeld } from "../features/notes/editor/noteTextEditorPorts";
export { requestReadingNotePanelTransfer } from "../features/notes/reading-notes/readingNotePanelTransfer";
export { catalogErrorMessage } from "../features/notes/noteCatalogError";
export { reportNoteCreationError } from "../features/notes/workspace/NotePages";
export type { NotePageSource } from "../features/notes/workspace/NotePages";
export { noteContextCandidate } from "../features/notes/workspace/noteContextCandidate";
export { PageLinkBlock } from "../features/notes/links/PageLinkBlock";
export { NotePageToolbar } from "../features/notes/workspace/NotePageToolbar";
export { NOTE_METADATA_CHANGED, NOTE_PAGES_DELETED } from "../features/notes/noteResourceEvents";
export type { NoteMetadataChanged } from "../features/notes/noteResourceEvents";
export { prepareNavigationSessions } from "../features/notes/workspace/noteNavigationSessions";
export { useResearchObjectTargets } from "../features/notes/links/useResearchObjectTargets";
export { openNoteDocumentTab } from "../features/notes/workspace/openNoteDocumentTab";
