import {createContext, useContext, useLayoutEffect, useRef, type MutableRefObject} from "react";
import type {NoteSessionOptions} from "@research-assistant/workbench-sdk/note-session";
import type {CoreNoteWorkspaceProps} from "@research-assistant/workbench-sdk/note-presentation";
import type {useNoteDocumentSession} from "./useNoteDocumentSession";
import type {useNoteWorkspaceTabs} from "./useNoteWorkspaceTabs";
import type {OwnedRetainedNoteEditorsProps} from "./RetainedNoteEditors";

export type OwnedNoteWorkspaceSession = ReturnType<typeof useNoteDocumentSession>;
export type OwnedNoteWorkspaceTabs = ReturnType<typeof useNoteWorkspaceTabs>;
export type OwnedNoteWorkspaceTabsOptions = Parameters<typeof useNoteWorkspaceTabs>[0];

export interface NoteWorkspaceHostContextValue {
  readonly props: CoreNoteWorkspaceProps;
  readonly session: OwnedNoteWorkspaceSession;
  readonly tabs: OwnedNoteWorkspaceTabs;
  readonly subscribeSession: (options: MutableRefObject<NoteSessionOptions>) => () => void;
  readonly mountTabs: (owner: symbol, options: MutableRefObject<OwnedNoteWorkspaceTabsOptions>) => () => void;
  readonly updateTabs: (owner: symbol) => void;
  readonly mountEditors: (owner: symbol, props: MutableRefObject<OwnedRetainedNoteEditorsProps>, target: HTMLElement) => () => void;
  readonly updateEditors: (owner: symbol) => void;
}

export const NoteWorkspaceHostContext = createContext<NoteWorkspaceHostContextValue | undefined>(undefined);

export function useCoreNoteWorkspaceHost(): CoreNoteWorkspaceProps | undefined {
  return useContext(NoteWorkspaceHostContext)?.props;
}

function useRequiredNoteWorkspaceHost(): NoteWorkspaceHostContextValue {
  const host = useContext(NoteWorkspaceHostContext);
  if (!host) throw new Error("NOTE_WORKSPACE_HOST_UNAVAILABLE：笔记文档呈现需要原Core笔记宿主。");
  return host;
}

/** Subscribers own presentation callbacks only, never the document session. */
export function useHostedNoteDocumentSession(options: NoteSessionOptions): OwnedNoteWorkspaceSession {
  const {session, subscribeSession} = useRequiredNoteWorkspaceHost();
  const live = useRef(options); live.current = options;
  useLayoutEffect(() => subscribeSession(live), [subscribeSession]);
  return session;
}

export function useHostedNoteWorkspaceTabs(options: OwnedNoteWorkspaceTabsOptions): OwnedNoteWorkspaceTabs {
  const {tabs, mountTabs, updateTabs} = useRequiredNoteWorkspaceHost();
  const owner = useRef(Symbol("note-tabs-projection"));
  const live = useRef(options); live.current = options;
  useLayoutEffect(() => mountTabs(owner.current, live), [mountTabs]);
  useLayoutEffect(() => updateTabs(owner.current));
  return tabs;
}
