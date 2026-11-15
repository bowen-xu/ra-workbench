import { useEffect, useRef, useState } from "react";
import type { NotePageId, NoteLifecycleStatus } from "@research-assistant/core/notes";
import { openPreviewTab, type PreviewTab } from "../../../components/workspace-tabs/previewTabs";
import { usePublishWorkspaceTabs } from "../../../components/workspace-tabs/WorkspaceTabs";
import type { WorkspacePresentation } from "../../../components/workspace-tabs/WorkspacePresentation";
import { notify } from "../../../services/notifications";
import { NOTE_METADATA_CHANGED, NOTE_PAGES_DELETED, type NoteMetadataChanged } from "../noteResourceEvents";
import {loadNoteTabs} from "./noteTabSession";
import {writeWorkspaceState} from "../../../runtime/workspaceState";

export function useNoteWorkspaceTabs(options: {
  readonly page: { readonly id: NotePageId; readonly title: string; readonly status?: NoteLifecycleStatus } | undefined;
  readonly active: boolean;
  readonly homeActive?: boolean;
  readonly enabled: boolean;
  readonly dirty: boolean;
  readonly transientIds?: readonly string[];
  readonly pinnedPageId?: NotePageId;
  readonly documentPageId?: NotePageId;
  readonly open: (id: NotePageId, background?: boolean) => Promise<void>;
  readonly getPageId: () => NotePageId | undefined;
  readonly prepare: (id?: NotePageId) => Promise<boolean>;
  readonly clear: (preserveHome?: boolean) => void;
  readonly dispose?: (id: NotePageId) => void;
  readonly onHome?: () => void;
  readonly onClose?: ((ids: readonly string[], active?: string, presentation?: WorkspacePresentation) => void) | undefined;
}) {
  const [restored] = useState(() => options.enabled ? loadNoteTabs() : []);
  const [tabs, setTabs] = useState<readonly PreviewTab[]>(restored);
  const pinned = useRef(new Set(restored.filter(tab=>!tab.preview).map(tab=>tab.id))), documents = useRef(new Set(restored.filter(tab=>tab.document).map(tab=>tab.id)));
  const transient = options.transientIds?.join("\0") ?? "";
  useEffect(() => {
    if (options.enabled) writeWorkspaceState("note-tabs",tabs.filter(tab=>!options.transientIds?.includes(tab.id)).map(tab=>({...tab,...(documents.current.has(tab.id)?{document:true}:{})})));
  }, [tabs,options.enabled,transient]);
  const statuses = useRef(new Map<string, NoteLifecycleStatus>());
  const deletionSequence = useRef(0), deletionDestination = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!options.documentPageId) return;
    documents.current.add(options.documentPageId);
    pin(options.documentPageId);
  }, [options.documentPageId]);
  function pin(id: string): void {
    pinned.current.add(id);
    setTabs(current => current.map(tab => tab.id === id ? { ...tab, preview: false } : tab));
  }
  useEffect(() => { if (options.pinnedPageId) pin(options.pinnedPageId); }, [options.pinnedPageId]);
  useEffect(() => {
    if (!options.page || !options.enabled) return;
    const { id, title } = options.page;
    setTabs(current => openPreviewTab(current, id, title || "未命名笔记", pinned.current.has(id)));
  }, [options.page?.id, options.page?.title, options.enabled]);
  useEffect(() => { if (options.page && options.dirty) pin(options.page.id); }, [options.page?.id, options.dirty]);
  useEffect(() => {
    if (options.page?.status) statuses.current.set(options.page.id, options.page.status);
  }, [options.page?.id, options.page?.status]);
  const live = useRef({ tabs, options }); live.current = { tabs, options };
  useEffect(() => {
    if (!options.enabled) return;
    const committed = (event: Event) => {
      const receipt = (event as CustomEvent<NoteMetadataChanged>).detail;
      const ids = receipt.action === "trash" ? receipt.pages.filter(page => page.status === "trashed" && statuses.current.get(page.id) !== "trashed").map(page => page.id) : [];
      for (const page of receipt.pages) statuses.current.set(page.id, page.status);
      if (ids.length) void closeDeleted(ids).catch(reason => notify.error(String(reason)));
    };
    window.addEventListener(NOTE_METADATA_CHANGED, committed);
    const deleted = (event: Event) => { void closeDeleted((event as CustomEvent<readonly string[]>).detail).catch(reason => notify.error(String(reason))); };
    window.addEventListener(NOTE_PAGES_DELETED, deleted);
    return () => { window.removeEventListener(NOTE_METADATA_CHANGED, committed); window.removeEventListener(NOTE_PAGES_DELETED, deleted); };
  }, [options.enabled]);
  async function closeDeleted(ids: readonly string[]): Promise<void> {
    const current = live.current, deleted = new Set(ids);
    const affected = current.tabs.filter(tab => deleted.has(tab.id));
    if (!affected.length) return;
    const active = current.options.getPageId() ?? deletionDestination.current, activeDeleted = !!active && deleted.has(active);
    const remaining = current.tabs.filter(tab => !deleted.has(tab.id));
    const index = current.tabs.findIndex(tab => tab.id === active);
    const next = activeDeleted ? remaining[Math.min(index, remaining.length - 1)]?.id : active;
    const sequence = activeDeleted ? ++deletionSequence.current : deletionSequence.current;
    if (activeDeleted) deletionDestination.current = next;
    // The mutation owner already completed the durable guard before deleting.
    // Do not save a now-trashed owner or reopen another deleted cascade member.
    live.current = {...current, tabs: remaining};
    setTabs(remaining);
    for (const tab of affected) {
      pinned.current.delete(tab.id); documents.current.delete(tab.id); statuses.current.delete(tab.id);
      current.options.dispose?.(tab.id as NotePageId);
    }
    if (activeDeleted) current.options.clear(current.options.homeActive);
    try {
      if (activeDeleted && next) await current.options.open(next as NotePageId, !current.options.active || current.options.homeActive === true);
    } finally {
      if (sequence === deletionSequence.current && (activeDeleted || deletionDestination.current === undefined)) {
        if (activeDeleted) deletionDestination.current = undefined;
        const latest = live.current, selected = latest.options.getPageId();
        latest.options.onClose?.(latest.tabs.map(tab => tab.id), selected,
          latest.options.active ? documents.current.has(selected ?? "") ? "document" : "workspace" : undefined);
      }
    }
  }
  async function close(id: string): Promise<void> {
    if (!await live.current.options.prepare(id as NotePageId)) return;
    const current = live.current;
    const index = current.tabs.findIndex(tab => tab.id === id);
    const remaining = current.tabs.filter(tab => tab.id !== id);
    const active = current.options.page?.id;
    const next = active === id ? remaining[Math.min(index, remaining.length - 1)]?.id : active;
    if (active === id && next) {
      await current.options.open(next as NotePageId, !current.options.active || current.options.homeActive === true);
      if (current.options.getPageId() !== next) return;
    }
    else if (active === id) current.options.clear(current.options.homeActive);
    setTabs(remaining);
    pinned.current.delete(id); documents.current.delete(id); statuses.current.delete(id);
    current.options.onClose?.(remaining.map(tab => tab.id), next, current.options.active ? documents.current.has(next ?? "") ? "document" : "workspace" : undefined);
  }
  async function home(): Promise<boolean> {
    if (!await live.current.options.prepare()) return false;
    live.current.options.clear();
    live.current.options.onHome?.();
    return true;
  }
  usePublishWorkspaceTabs("notes", [{id:"notes:home",owner:"notes",title:"返回笔记首页",home:true,active:options.active && (!options.page || options.homeActive === true),select:()=>{void home().catch(reason=>notify.error(String(reason)));}}, ...tabs.map(tab => ({
    id: `notes:${tab.id}`, owner: "notes", destination: tab.id, presentation: documents.current.has(tab.id) ? "document" : "workspace", title: tab.title, preview: tab.preview,
    active: options.active && !options.homeActive && options.page?.id === tab.id, dirty: options.page?.id === tab.id && options.dirty,
    select: () => { void live.current.options.open(tab.id as NotePageId).catch(reason => notify.error(String(reason))); },
    close: () => { void close(tab.id).catch(reason => notify.error(String(reason))); }, pin: () => pin(tab.id),
  } as const))], options.enabled);
  return { pin, closeDeleted, home, retainedIds: tabs.map(tab => tab.id) };
}
