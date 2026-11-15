import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { WorkspacePresentation } from "./components/workspace-tabs/WorkspacePresentation";
import {readWorkspaceState, writeWorkspaceState, workspaceString} from "./runtime/workspaceState";
import {loadNoteTabs} from "./features/notes/workspace/noteTabSession";
import {isAbandonedEmptyNoteDraft} from "./features/notes/workspace/noteDraftIdentity";

import type { WorkspaceLocation, WorkspaceView } from "./contracts/workbench";
export type { WorkspaceLocation, WorkspaceView } from "./contracts/workbench";

const sameLocation = (left: WorkspaceLocation | undefined, right: WorkspaceLocation): boolean =>
  left?.presentation === right.presentation && left?.workspaceView === right.workspaceView && left?.view === right.view && left.tabId === right.tabId && left.notePageId === right.notePageId;

export interface WorkspaceNavigationRestore { readonly homeTabId: string; readonly readActiveTabId: () => string; }
export interface WorkspaceNavigationSelection {
  readonly availableViews: readonly WorkspaceView[];
  /** A selected workspace set retains the full product's recovery preference. */
  readonly persistenceKey?: string;
  /** Runtime discovery and activation have not settled. Preserve the saved
   * location until the selected views are known or the user navigates. */
  readonly deferredRestore?: boolean;
}
const DEFAULT_RESTORE: WorkspaceNavigationRestore = {homeTabId:"library",readActiveTabId:()=>"library"};

interface RestoredWorkspaceLocation {
  readonly location: WorkspaceLocation;
  readonly unavailable: boolean;
}

function readWorkspaceLocation(restore: WorkspaceNavigationRestore, selection?: WorkspaceNavigationSelection): RestoredWorkspaceLocation {
  const first = selection?.availableViews[0] ?? "library";
  const fallback: WorkspaceLocation = first === "library" ? {view:first,tabId:restore.readActiveTabId()} : {view:first};
  const fallbackResult = {location: fallback, unavailable: false};
  const decode = (value: unknown): RestoredWorkspaceLocation => {
    if (!value || typeof value !== "object") return fallbackResult;
    const row = value as Record<string,unknown>, views = ["library","notes","writing","settings", ...(selection?.availableViews ?? [])];
    if (typeof row.view !== "string") return fallbackResult;
    if (!views.includes(row.view)) return {location: fallback, unavailable: selection !== undefined};
    if(row.view === "notes" && typeof row.notePageId === "string" && isAbandonedEmptyNoteDraft(row.notePageId,loadNoteTabs().map(tab=>tab.id))) return {location: {view:"notes"}, unavailable: false};
    if (selection && !selection.availableViews.includes(row.view as WorkspaceView)
      && !(row.view === "notes" && typeof row.notePageId === "string")) return {location: fallback, unavailable: true};
    return {location: {view:row.view as WorkspaceView,
      ...(typeof row.tabId === "string" ? {tabId:row.tabId} : {}),
      ...(typeof row.notePageId === "string" ? {notePageId:row.notePageId} : {}),
      ...(row.presentation === "document" ? {presentation: "document", ...(views.includes(String(row.workspaceView)) ? {workspaceView:row.workspaceView as WorkspaceView} : {})} : {})},
      unavailable: row.presentation === "document" && typeof row.workspaceView === "string"
        && selection !== undefined && !selection.availableViews.includes(row.workspaceView)};
  };
  const previous = readWorkspaceState("location",fallbackResult,decode);
  return selection?.persistenceKey ? readWorkspaceState(selection.persistenceKey,previous,decode) : previous;
}

export function loadWorkspaceLocation(restore: WorkspaceNavigationRestore = DEFAULT_RESTORE, selection?: WorkspaceNavigationSelection): WorkspaceLocation {
  return readWorkspaceLocation(restore, selection).location;
}

/** Visits include PDF identities; reading positions and editor undo have their own owners. */
export function useWorkspaceNavigation(initialLocation?: WorkspaceLocation, restore: WorkspaceNavigationRestore = DEFAULT_RESTORE, selection?: WorkspaceNavigationSelection) {
  const [initialRestore] = useState(() => initialLocation === undefined
    ? readWorkspaceLocation(restore, selection) : {location: initialLocation, unavailable: false});
  const deferredRestore = useRef(initialLocation === undefined && selection?.deferredRestore === true);
  const unavailableRestore = useRef(initialRestore.unavailable);
  const [restorationPendingAtRender, setRestorationPending] = useState(deferredRestore.current || unavailableRestore.current);
  const cancelDeferredRestore = useCallback(() => {
    deferredRestore.current = false;
    unavailableRestore.current = false;
    setRestorationPending(false);
  }, []);
  const restoreRef = useRef(restore); restoreRef.current = restore;
  const [history, setHistory] = useState<{ entries: WorkspaceLocation[]; index: number }>(() => ({
    entries: [initialRestore.location], index: 0,
  }));
  const lastLibraryTab = useRef(history.entries[0]?.tabId ?? restore.homeTabId);
  const availableViewKey = selection ? JSON.stringify(selection.availableViews) : undefined;
  useLayoutEffect(() => {
    if ((!deferredRestore.current && !unavailableRestore.current) || selection?.deferredRestore) return;
    const restored = readWorkspaceLocation(restore, selection);
    deferredRestore.current = false;
    unavailableRestore.current = restored.unavailable;
    setRestorationPending(restored.unavailable);
    lastLibraryTab.current = restored.location.tabId ?? restore.readActiveTabId();
    setHistory({entries: [restored.location], index: 0});
  }, [restore, selection?.deferredRestore, selection?.persistenceKey, availableViewKey]);
  const savedLastNote=readWorkspaceState("last-note","",workspaceString);
  const lastNotePage = useRef<string | undefined>(history.entries[0]?.notePageId ?? (savedLastNote && !isAbandonedEmptyNoteDraft(savedLastNote,loadNoteTabs().map(tab=>tab.id)) ? savedLastNote : undefined));
  const locationRef = useRef(history.entries[history.index]!); locationRef.current = history.entries[history.index]!;
  const navigate = useCallback((view: WorkspaceView, tabId?: string, presentation: WorkspacePresentation = "workspace") => {
    cancelDeferredRestore();
    setHistory(current => {
    const destination: WorkspaceLocation = view === "library" ? { view, tabId: tabId ?? lastLibraryTab.current }
      : view === "notes" && (tabId ?? lastNotePage.current) ? { view, notePageId: tabId ?? lastNotePage.current! } : { view };
    const origin = current.entries[current.index]!;
    const location: WorkspaceLocation = presentation === "document"
      ? { ...destination, presentation, workspaceView: origin.workspaceView ?? origin.view } : destination;
    if (sameLocation(current.entries[current.index], location)) return current;
    if (location.view === "notes" && location.notePageId && current.entries[current.index]?.view === "notes"
      && !current.entries[current.index]?.notePageId) {
      const entries = current.entries.slice(0, current.index + 1); entries[current.index] = location;
      return { entries, index: current.index };
    }
    const entries = [...current.entries.slice(0, current.index + 1), location];
    return { entries, index: entries.length - 1 };
    });
  }, []);
  const visitNotePage = useCallback((pageId: string) => { lastNotePage.current = pageId; navigate("notes", pageId, locationRef.current.view === "notes" ? locationRef.current.presentation : undefined); }, [navigate]);
  const visitNotesHome = useCallback(() => {
    cancelDeferredRestore();
    lastNotePage.current = undefined;
    setHistory(current => {
      const location: WorkspaceLocation = {view:"notes"};
      if (sameLocation(current.entries[current.index], location)) return current;
      const entries = [...current.entries.slice(0,current.index+1),location];
      return {entries,index:entries.length-1};
    });
  }, []);
  const visitLibraryTab = useCallback((tabId: string) => navigate("library", tabId), [navigate]);
  const reconcileLibrary = useCallback((tabIds: readonly string[], activeTabId: string) => {
    lastLibraryTab.current = activeTabId;
    setHistory(current => {
      const open = new Set([restoreRef.current.homeTabId, ...tabIds]);
      const entries: WorkspaceLocation[] = [];
      let index = 0;
      current.entries.forEach((entry, entryIndex) => {
        const invalid = entry.view === "library" && !open.has(entry.tabId ?? restoreRef.current.homeTabId);
        if (invalid && entryIndex !== current.index) return;
        const next = invalid ? { view: "library" as const, tabId: activeTabId } : entry;
        if (!sameLocation(entries.at(-1), next)) entries.push(next);
        if (entryIndex <= current.index) index = entries.length - 1;
      });
      return entries.length === current.entries.length && entries.every((entry, i) => sameLocation(entry, current.entries[i]!))
        ? current : { entries, index };
    });
  }, []);
  const back = useCallback(() => {cancelDeferredRestore(); setHistory(current => current.index > 0 ? { ...current, index: current.index - 1 } : current);}, []);
  const forward = useCallback(() => {cancelDeferredRestore(); setHistory(current => current.index < current.entries.length - 1 ? { ...current, index: current.index + 1 } : current);}, []);
  useEffect(() => {
    if (!selection || selection.deferredRestore) return;
    // A released registration must not remain a blank Back/Forward target.
    // Core note documents remain usable when their management view is absent.
    const available = new Set([...selection.availableViews, "notes"]);
    if (!available.has(locationRef.current.view)
      || (locationRef.current.workspaceView !== undefined && !available.has(locationRef.current.workspaceView))) {
      unavailableRestore.current = true;
      setRestorationPending(true);
    }
    setHistory(current => {
      const entries: WorkspaceLocation[] = [];
      let index = -1;
      current.entries.forEach((entry, position) => {
        if (!available.has(entry.view)) return;
        let next = entry;
        if (entry.workspaceView && !available.has(entry.workspaceView)) {
          const {workspaceView: _releasedContext, ...document} = entry;
          next = document;
        }
        if (!sameLocation(entries.at(-1), next)) entries.push(next);
        if (position <= current.index) index = entries.length - 1;
      });
      if (!entries.length) return {entries: [{view: selection.availableViews[0] ?? "settings"}], index: 0};
      index = Math.max(index, 0);
      return index === current.index && entries.length === current.entries.length
        && entries.every((entry, position) => sameLocation(entry, current.entries[position]!)) ? current : {entries, index};
    });
  }, [availableViewKey, selection?.deferredRestore]);
  const reconcileNotes = useCallback((pageIds: readonly string[], activePageId?: string, presentation?: WorkspacePresentation) => {
    lastNotePage.current = activePageId;
    const open = new Set(pageIds);
    setHistory(current => {
      const entries: WorkspaceLocation[] = [];
      let index = 0;
      current.entries.forEach((entry, entryIndex) => {
        const invalid = entry.view === "notes" && entry.notePageId !== undefined && !open.has(entry.notePageId);
        if (invalid && entryIndex !== current.index) return;
        const context = entry.workspaceView ?? "notes";
        const reconciled: WorkspaceLocation = invalid && !activePageId && entry.presentation === "document"
          ? {view: context, ...(context === "library" ? {tabId: lastLibraryTab.current} : {})}
          : invalid ? { view: "notes", ...(activePageId ? { notePageId: activePageId, ...(entry.presentation ? {presentation: entry.presentation, ...(entry.workspaceView ? {workspaceView: entry.workspaceView} : {})} : {}) } : {}) } : entry;
        const next: WorkspaceLocation = entryIndex === current.index && reconciled.view === "notes" && presentation
          ? { view: "notes", ...(activePageId ? {notePageId: activePageId} : {}), ...(presentation === "document" && activePageId ? {presentation, ...(entry.workspaceView ? {workspaceView: entry.workspaceView} : {})} : {}) } : reconciled;
        if (!sameLocation(entries.at(-1), next)) entries.push(next);
        if (entryIndex <= current.index) index = entries.length - 1;
      });
      return { entries, index };
    });
  }, []);
  const location = history.entries[history.index]!;
  useEffect(() => {
    if (restorationPendingAtRender) return;
    if (location.view === "notes" && location.notePageId) lastNotePage.current = location.notePageId;
    writeWorkspaceState(selection?.persistenceKey ?? "location",location);
    writeWorkspaceState("last-note",lastNotePage.current ?? "");
  }, [location, selection?.persistenceKey, restorationPendingAtRender]);
  return { view: location.view, workspaceView: location.workspaceView ?? location.view, location, previous: history.entries[history.index - 1]?.view,
    notePageId: location.view === "notes" ? location.notePageId : lastNotePage.current,
    next: history.entries[history.index + 1]?.view, navigate, visitNotePage, visitNotesHome, visitLibraryTab, reconcileLibrary, reconcileNotes, back, forward };
}
