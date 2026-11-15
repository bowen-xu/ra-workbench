import {
  createContext,
  useContext,
  useMemo,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import type { NotesGateway } from "@research-assistant/data/notes";

import { getNotesGateway } from "./gateways";
import { NoteResourceChanges, type NoteResourceChange } from "./NoteResourceChanges";
import { NoteLinkReads } from "./NoteLinkReads";
import {NotePages} from "./workspace/NotePages";
import {NoteCommandHistory} from "./editor/NoteCommandHistory";

export class NotesClient {
  readonly gateway: NotesGateway;
  readonly links: NoteLinkReads;
  readonly creation: NotePages;
  readonly history=new NoteCommandHistory();
  private readonly changes = new NoteResourceChanges();
  private readonly listeners = new Set<() => void>();

  constructor(gateway: NotesGateway) {
    this.gateway = gateway;
    this.links = new NoteLinkReads(gateway);
    this.creation = new NotePages(this);
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getVersion = (): number => this.changes.getVersion();
  getTagsVersion = (): number => this.changes.getTagsVersion();
  getChangesSince = (version: number): readonly NoteResourceChange[] => this.changes.since(version);

  notifyChanged(change: NoteResourceChange = {kind: "all"}): void {
    if (!this.changes.record(change)) return;
    this.links.invalidate(change);
    if (typeof window !== "undefined" && change.kind !== "tags") window.dispatchEvent(new CustomEvent("research-assistant:note-resources-changed", {detail: change.kind === "document" ? {pageId: change.pageId, revision: change.revision} : {}}));
    for (const listener of this.listeners) listener();
  }

  async mutate<T>(operation: (gateway: NotesGateway) => Promise<T>): Promise<T> {
    const result = await operation(this.gateway);
    this.notifyChanged();
    return result;
  }
}

const NotesClientContext = createContext<NotesClient | undefined>(undefined);

export function NotesProvider({ children, gateway }: { readonly children: ReactNode; readonly gateway?: NotesGateway }): React.JSX.Element {
  const client = useMemo(() => new NotesClient(gateway ?? getNotesGateway()), [gateway]);
  return <NotesClientContext.Provider value={client}>{children}</NotesClientContext.Provider>;
}

export function useNotesClient(): NotesClient {
  const client = useContext(NotesClientContext);
  if (client === undefined) throw new Error("NotesProvider is missing");
  return client;
}

export function useNotesVersion(): number {
  const client = useNotesClient();
  return useSyncExternalStore(client.subscribe, client.getVersion, client.getVersion);
}

export function useNoteTagsVersion(): number {
  const client = useNotesClient();
  return useSyncExternalStore(client.subscribe, client.getTagsVersion, client.getTagsVersion);
}
