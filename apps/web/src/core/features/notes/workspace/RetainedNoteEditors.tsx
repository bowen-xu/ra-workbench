import { memo, useCallback, useContext, useLayoutEffect, useRef, type ComponentProps } from "react";
import {hasNoteDraftContent} from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import type { NoteEditor } from "../editor/NoteEditor";
import { HbsNoteEditor } from "../../../components/hbs/HbsNoteEditor";
import type { NoteEditorInstallPort } from "../editor/noteEditorInstallPort";
import { useNoteNavigationSession } from "./noteNavigationSessions";
import type { RetainedNoteSession } from "./retainedNoteSession";
import {NoteWorkspaceHostContext} from "./noteWorkspaceHostContext";

type EditorProps = ComponentProps<typeof NoteEditor>;
export interface OwnedRetainedNoteEditorsProps extends Omit<EditorProps, "document" | "onChange" | "isReadOnly" | "conflictLocked" | "session"> {
  readonly isPageDraft?: (id:string)=>boolean;
  readonly hasPageTitle?: boolean;
  readonly readingPageIds?: ReadonlySet<string>;
  readonly sessions: readonly RetainedNoteSession[];
  readonly activeId: string | undefined;
  readonly conflictLocked: boolean;
  readonly onChange: (document: EditorProps["document"], editorSessionId: number) => void;
}
type Props = OwnedRetainedNoteEditorsProps;

/** Hidden tabs retain the editor, native history, review baseline and write ports. */
export function RetainedNoteEditors(props: Props): React.JSX.Element {
  const host = useContext(NoteWorkspaceHostContext);
  return host ? <ProjectedRetainedNoteEditors {...props}/> : <OwnedRetainedNoteEditors {...props}/>;
}

/** One stable portal target belongs to Core, even when this presentation moves
 * between the fallback and the independently loaded management component. */
function ProjectedRetainedNoteEditors(props: Props): React.JSX.Element {
  const host = useContext(NoteWorkspaceHostContext);
  const owner = useRef(Symbol("retained-note-editor-projection"));
  const target = useRef<HTMLDivElement>(null);
  const live = useRef(props); live.current = props;
  const mount = host?.mountEditors, update = host?.updateEditors;
  useLayoutEffect(() => {
    if (target.current && mount) return mount(owner.current, live, target.current);
  }, [mount]);
  useLayoutEffect(() => update?.(owner.current));
  return <div ref={target} className="core-note-editor-projection"/>;
}

export function OwnedRetainedNoteEditors(props: Props): React.JSX.Element {
  return <>{props.sessions.map(session => <RetainedEditor key={`${session.snapshot.page.id}:${session.editorSessionId}`}
    {...props} session={session} snapshot={session.snapshot} sessionStatus={session.saveStatus} active={props.activeId === session.snapshot.page.id}/>)}</>;
}

const RetainedEditor = memo(function RetainedEditor({session, active, snapshot, sessionStatus, isPageDraft, ...props}: Props & {
  readonly session: RetainedNoteSession; readonly active: boolean; readonly snapshot: NoteDocumentSnapshot; readonly sessionStatus: RetainedNoteSession["saveStatus"];
}): React.JSX.Element {
  const current = useRef({active, callback: props.onInstallPort}); current.current = {active, callback: props.onInstallPort};
  const port = useRef<NoteEditorInstallPort | undefined>(undefined);
  const retrySave = useCallback(async () => {
    await session.controller.retry();
    if(session.controller.isEditingBlocked())throw new Error(session.saveMessage ?? "MODEL_REQUEST_UNSETTLED：原操作尚未核对。");
  }, [session]);
  const retryRequired = session.controller.requiresRequestRetry?.() ?? false;
  const register = useCallback((value: NoteEditorInstallPort | undefined) => {
    port.current = value;
    if (current.current.active) current.current.callback?.(value);
  }, []);
  useLayoutEffect(() => { if (active) props.onInstallPort?.(port.current); }, [active, props.onInstallPort]);
  useNoteNavigationSession(session.snapshot.page.id, {saves: {current: session.controller}, getSnapshot: () => session.snapshot});
  return <div hidden={!active} inert={!active} className="retained-note-editor">
    <HbsNoteEditor {...props} session={session.hbs} legacyMetadata={!(isPageDraft?.(snapshot.page.id) ?? false)} isReadOnly={snapshot.page.status !== "active"}
      autoFocus={active && !hasNoteDraftContent(snapshot.document)}
      onRetrySave={retrySave} retryRequired={retryRequired}
      pageTitle={props.hasPageTitle && !props.readingPageIds?.has(snapshot.page.id) ? {
        value: snapshot.page.title,
      } : undefined}
      conflictLocked={session.controller.isEditingBlocked?.() ?? false} onInstallPort={register}
      diffControlsTarget={active ? props.diffControlsTarget ?? null : null}
      sourceNavigation={active ? props.sourceNavigation : undefined}/>
  </div>;
}, (previous, next) => !previous.active && !next.active && previous.snapshot === next.snapshot && previous.sessionStatus === next.sessionStatus);
