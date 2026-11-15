import {useSyncExternalStore} from "react";

export interface NoteOperationLogActions {
  copy(): Promise<void>;
  download(): Promise<void>;
  clear(): void;
  readonly copying: boolean;
  readonly downloading: boolean;
}

interface Registration {readonly actions: NoteOperationLogActions}
const registrations = new Map<string, Set<Registration>>();
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => {listeners.delete(listener);};
};
const notify = () => {listeners.forEach(listener => listener());};

/** The retained editor owns the buffer. Headers receive only its explicit
 * diagnostic commands; ambiguous owners must never expose another editor's log. */
export function registerNoteOperationLogActions(pageId: string, actions: NoteOperationLogActions): () => void {
  const registration = {actions};
  const group = registrations.get(pageId) ?? new Set<Registration>();
  group.add(registration); registrations.set(pageId, group); notify();
  return () => {
    if (!group.delete(registration)) return;
    if (!group.size && registrations.get(pageId) === group) registrations.delete(pageId);
    notify();
  };
}

export function useNoteOperationLogActions(pageId: string | undefined): NoteOperationLogActions | undefined {
  return useSyncExternalStore(subscribe, () => {
    const group = pageId === undefined ? undefined : registrations.get(pageId);
    return group?.size === 1 ? group.values().next().value?.actions : undefined;
  }, () => undefined);
}
