import { useEffect, useMemo, useSyncExternalStore } from "react";
import { compareTableRows, type DocumentReviewProjector, type HbsSession } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { NoteEditor, type NoteEditorProps } from "../../features/notes/editor/NoteEditor";
import type { NoteEditorSession } from "../../features/notes/editor/noteEditorSession";
import { sameNoteProjection } from "../../features/notes/editor/noteReviewProjection";
import type { useNoteTextReview } from "../../features/notes/editor/useNoteTextReview";
import { useTextEditorHeld } from "../../features/notes/editor/noteTextEditorPorts";
import type { EditorReviewConfiguration } from "../editor-diff/EditorReviewActions";
import { noteSessionHistory } from "./noteSessionHistory";
import { HbsNoteReview } from "./HbsNoteReview";
import type { NoteTableComparison } from "./noteTableDecorations";
import { useHbsNoteMetadataReview } from "../../features/notes/editor/useHbsNoteMetadataReview";
import { HbsNoteOperationLog } from "./HbsNoteOperationLog";
import {useNoteStaleReview} from "../../features/notes/editor/useNoteStaleReview";

function useHbsNoteReview(session: HbsSession<NoteDocumentV1>, options: Parameters<typeof useNoteTextReview>[0], metadata: ReturnType<typeof useHbsNoteMetadataReview>, stale:ReturnType<typeof useNoteStaleReview>): ReturnType<typeof useNoteTextReview> {
  const state=useSyncExternalStore(session.subscribe,session.getSnapshot);
  useEffect(()=>stale.bindEditor(options.editor,options.port),[stale.bindEditor,options.editor,options.port]);
  useEffect(()=>{
    const port=options.port.current;if(!port || sameNoteProjection(port.read(),state.content))return;
    try{port.prepareStructure(port.read(),state.content,undefined,{addToHistory:false})();}
    catch(reason){session.record(`MODEL_EDITOR_INSTALL_FAILED：${reason instanceof Error?reason.message:String(reason)}`);}
  },[session,state.content,options.port]);
  return {trustedDocument:state.content,pendingBlockIds:[],chunks:[],locked:metadata.busy||!!metadata.request,disabled:metadata.busy,busy:state.pending||metadata.busy,
    error:metadata.error,request:metadata.request,metadataPending:metadata.metadataPending,metadataKind:metadata.metadataKind,
    canReplayHistory:()=>true,publishInput:()=>true,resolve:action=>metadata.resolve(action),
    restore:metadata.restore};
}
/** Only session ownership changes. Clipboard, links, move policy, selection,
 * IME, commands and the whole view stay in the existing production NoteEditor. */
export function HbsNoteEditor({session,configuration,tableComparison=compareTableRows,reviewProjector,legacyMetadata=false,onRetrySave,retryRequired=false,...editorProps}: Omit<NoteEditorProps,"session"|"document"|"onChange"> & {
  readonly session:HbsSession<NoteDocumentV1>;readonly configuration?:EditorReviewConfiguration;
  /** Shared default for every host; inject an alternative correspondence policy if needed. */
  readonly tableComparison?:NoteTableComparison;
  readonly reviewProjector?:DocumentReviewProjector;
  readonly legacyMetadata?:boolean;
  readonly onRetrySave?:()=>Promise<void>;
  readonly retryRequired?:boolean;
}): React.JSX.Element {
  const state=useSyncExternalStore(session.subscribe,session.getSnapshot);
  const metadata=useHbsNoteMetadataReview(session,legacyMetadata);
  const stale=useNoteStaleReview(session,legacyMetadata,!!editorProps.isReadOnly);
  const held=useTextEditorHeld(state.content.pageId);
  const disabled=!!editorProps.isReadOnly||!!editorProps.conflictLocked||held||metadata.busy||!!metadata.request||stale.busy;
  const integration=useMemo<NoteEditorSession>(()=>({key:"hbs-document-session",surfaceClassName:"hbs-note-editor",createHistory:(_initial,canReplay)=>[noteSessionHistory(session,canReplay)],
    useReview:options=>useHbsNoteReview(session,options,metadata,stale),
    renderReview:(editor,port)=><><HbsNoteReview editor={editor} port={port} session={session} disabled={disabled} onRetrySave={onRetrySave} retryRequired={retryRequired} controlsTarget={editorProps.diffControlsTarget} {...(configuration?{configuration}:{})}
      tableComparison={tableComparison} {...(reviewProjector ? {reviewProjector} : {})}/>
      <HbsNoteOperationLog editor={editor} session={session}/></>}),[session,configuration,tableComparison,reviewProjector,disabled,onRetrySave,retryRequired,editorProps.diffControlsTarget,metadata,stale]);
  return <NoteEditor {...editorProps} document={state.content} session={integration} onChange={value=>session.edit(value)}
    {...(configuration?.highlight ? {diffHighlight:configuration.highlight} : {})}/>;
}
