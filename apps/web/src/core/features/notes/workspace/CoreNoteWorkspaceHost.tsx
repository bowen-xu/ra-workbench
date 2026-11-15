import {
  useContext, useEffect, useId, useImperativeHandle, useLayoutEffect, useMemo,
  useReducer, useRef, useState, type MutableRefObject, type ReactNode,
} from "react";
import {createPortal} from "react-dom";
import type {NoteSessionOptions} from "@research-assistant/workbench-sdk/note-session";
import type {CoreNoteWorkspaceHostProps} from "@research-assistant/workbench-sdk/note-presentation";
import {useNotesClient} from "../NotesClient";
import {notify} from "../../../services/notifications";
import {getReaderCloseCoordinator} from "../../../workspace/DocumentCloseCoordinator";
import {useNoteDocumentSession} from "./useNoteDocumentSession";
import {useNoteWorkspaceTabs} from "./useNoteWorkspaceTabs";
import {OwnedRetainedNoteEditors, type OwnedRetainedNoteEditorsProps} from "./RetainedNoteEditors";
import {
  NoteWorkspaceHostContext, type NoteWorkspaceHostContextValue, type OwnedNoteWorkspaceTabsOptions,
} from "./noteWorkspaceHostContext";
import {
  createNoteWorkspaceProjection, liveNoteWorkspaceProjection, sameNoteWorkspacePresentation,
  type NoteWorkspaceProjection,
} from "./noteWorkspaceProjection";
import "./CoreNoteWorkspace.css";

interface EditorProjection extends NoteWorkspaceProjection<OwnedRetainedNoteEditorsProps> {
  readonly target: HTMLElement;
}

function lastActive<T extends {readonly ref: MutableRefObject<{readonly active?: boolean}>}>(values: Iterable<T>): T | undefined {
  const entries = [...values];
  return entries.filter(entry => entry.ref.current.active).at(-1) ?? entries.at(-1);
}

/** Reuses the enclosing main-document scope. Standalone callers get the same
 * original Core owner; embedded reading windows explicitly create their own. */
export function CoreNoteWorkspaceHost(props: CoreNoteWorkspaceHostProps): React.JSX.Element {
  const parent = useContext(NoteWorkspaceHostContext);
  return parent ? <>{props.children}</> : <OwnedCoreNoteWorkspaceHost {...props}/>;
}

export function OwnedCoreNoteWorkspaceHost({children, ...props}: CoreNoteWorkspaceHostProps): React.JSX.Element {
  const client = useNotesClient(), hostId = useId();
  const [, publish] = useReducer(value => value + 1, 0);
  const [bridge] = useState(() => ({
    alive: true,
    subscribers: new Set<MutableRefObject<NoteSessionOptions>>(),
    tabProjections: new Map<symbol, NoteWorkspaceProjection<OwnedNoteWorkspaceTabsOptions>>(),
    editorProjections: new Map<symbol, EditorProjection>(),
    lastEditor: undefined as EditorProjection | undefined,
    container: Object.assign(document.createElement("div"), {className: "core-note-editor-host"}),
  }));
  const session = useNoteDocumentSession({client,
    onError: message => {
      const subscriber = [...bridge.subscribers].at(-1);
      if (subscriber) subscriber.current.onError(message); else notify.error(message);
    },
    onNotice: message => {
      const subscriber = [...bridge.subscribers].at(-1);
      if (subscriber) subscriber.current.onNotice(message); else notify.info(message);
    },
    onLoaded: (id, outgoing, incoming) => {
      for (const subscriber of bridge.subscribers) subscriber.current.onLoaded(id, outgoing, incoming);
    },
  });
  const liveSession = useRef(session); liveSession.current = session;
  const liveProps = useRef(props); liveProps.current = props;
  const [defaultTabs] = useState<OwnedNoteWorkspaceTabsOptions>(() => ({
    get page() { return liveSession.current.snapshot?.page; },
    get active() { return liveProps.current.active ?? true; },
    get enabled() { return liveProps.current.embedded !== true; },
    get dirty() { return liveSession.current.saveStatus !== "saved"; },
    open: async id => { await liveSession.current.loadPage(id); liveProps.current.onNavigatePage?.(id); },
    getPageId: () => liveSession.current.getSelectedPageId(),
    prepare: id => liveSession.current.ensureDurable(false, id),
    clear: () => liveSession.current.deactivate(),
    dispose: id => liveSession.current.disposeSession(id),
    onHome: () => liveProps.current.onHome?.(),
    onClose: (ids, active, presentation) => liveProps.current.onClosePage?.(ids, active, presentation),
  }));
  const [tabOptions] = useState(() => liveNoteWorkspaceProjection(() =>
    lastActive(bridge.tabProjections.values())?.ref.current ?? defaultTabs));
  const tabs = useNoteWorkspaceTabs(tabOptions);
  const methods = useMemo(() => {
    const changed = () => { if (bridge.alive) publish(); };
    const editorProjection = () => {
      const entries = [...bridge.editorProjections.values()];
      return entries.filter(entry => entry.ref.current.activeId !== undefined).at(-1) ?? entries.at(-1);
    };
    const moveEditor = () => {
      const projection = editorProjection();
      if (projection) {
        bridge.lastEditor = projection;
        if (bridge.container.parentElement !== projection.target) projection.target.append(bridge.container);
      } else bridge.container.remove();
    };
    return {
      subscribeSession(options: MutableRefObject<NoteSessionOptions>) {
        bridge.subscribers.add(options);
        const current = liveSession.current;
        const retained = current.retainedSessions.find(value => value.snapshot.page.id === current.snapshot?.page.id);
        if (current.snapshot) options.current.onLoaded(current.snapshot.page.id, retained?.outgoing ?? [], retained?.incoming ?? []);
        return () => { bridge.subscribers.delete(options); };
      },
      mountTabs(owner: symbol, options: MutableRefObject<OwnedNoteWorkspaceTabsOptions>) {
        bridge.tabProjections.set(owner, createNoteWorkspaceProjection(options)); changed();
        return () => { bridge.tabProjections.delete(owner); changed(); };
      },
      updateTabs(owner: symbol) {
        const entry = bridge.tabProjections.get(owner);
        if (!entry || sameNoteWorkspacePresentation(entry.snapshot, entry.ref.current)) return;
        entry.snapshot = {...entry.ref.current}; changed();
      },
      mountEditors(owner: symbol, options: MutableRefObject<OwnedRetainedNoteEditorsProps>, target: HTMLElement) {
        bridge.editorProjections.set(owner, {...createNoteWorkspaceProjection(options), target});
        moveEditor(); changed();
        return () => { bridge.editorProjections.delete(owner); moveEditor(); changed(); };
      },
      updateEditors(owner: symbol) {
        const entry = bridge.editorProjections.get(owner);
        if (!entry || sameNoteWorkspacePresentation(entry.snapshot, entry.ref.current)) return;
        entry.snapshot = {...entry.ref.current}; moveEditor(); changed();
      },
    };
  }, [bridge]);
  useLayoutEffect(() => {
    bridge.alive = true; publish();
    return () => { bridge.alive = false; bridge.container.remove(); };
  }, [bridge]);
  useImperativeHandle(props.ref, () => ({prepareClose: (saved = false) => liveSession.current.ensureAllDurable(saved)}), []);
  useEffect(() => getReaderCloseCoordinator().register(`notes-workspace:${hostId}`, {
    isActive: () => false, prepare: () => liveSession.current.ensureAllDurable(), commit: () => undefined,
  }), [hostId]);
  const context: NoteWorkspaceHostContextValue = {props, session, tabs, ...methods};
  const mountedEditor = [...bridge.editorProjections.values()].filter(value => value.ref.current.activeId !== undefined).at(-1)
    ?? [...bridge.editorProjections.values()].at(-1);
  const projection = mountedEditor ?? bridge.lastEditor;
  let editors: ReactNode;
  if (projection) {
    const {onInstallPort: _port, sourceNavigation: _source, diffControlsTarget: _diff, ...inactive} = projection.live;
    const editorProps = mountedEditor ? projection.live : {...inactive, diffControlsTarget: null};
    editors = createPortal(<OwnedRetainedNoteEditors {...editorProps}
      sessions={session.retainedSessions}
      activeId={mountedEditor?.ref.current.activeId}
      onChange={(document, editorSessionId) => {tabs.pin(document.pageId); session.onRetainedDocumentChange(document, editorSessionId);}}
    />, bridge.container);
  }
  return <NoteWorkspaceHostContext.Provider value={context}>{children}{editors}</NoteWorkspaceHostContext.Provider>;
}
