import type { WorkspaceTab } from "./WorkspaceTabs";
import { reorderedRowIds } from "../reorderLayout";

function samePresentation(left: WorkspaceTab, right: WorkspaceTab): boolean {
  const keys = new Set([...Object.keys(left), ...Object.keys(right)] as (keyof WorkspaceTab)[]);
  for (const key of keys) {
    const a = left[key], b = right[key];
    if (typeof a === "function" && typeof b === "function") continue;
    if (a !== b) return false;
  }
  return true;
}

/** Presentation snapshots are stable; actions always dispatch to the latest owner. */
export class WorkspaceTabsRegistry {
  private groups = new Map<string, readonly WorkspaceTab[]>();
  private current = new Map<string, WorkspaceTab>();
  private snapshot: readonly WorkspaceTab[] = [];
  private listeners = new Set<() => void>();
  private order: readonly string[] = [];
  constructor(order: readonly string[] = [], private readonly saveOrder?: (order:readonly string[])=>void) { this.order = order; }
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {this.listeners.add(listener); return () => {this.listeners.delete(listener);};};
  set(group: string, tabs: readonly WorkspaceTab[]): void {
    const removed = new Set((this.groups.get(group) ?? []).filter(previous=>!tabs.some(tab=>tab.id===previous.id)).map(tab=>tab.id));
    this.order = this.order.filter(id=>!removed.has(id));
    this.groups.set(group, tabs); this.publish();
  }
  remove(group: string): void {this.groups.delete(group); this.publish();}
  move(source: string, target: string, after: boolean): void {
    if (this.current.get(source)?.home || this.current.get(target)?.home) return;
    this.order = reorderedRowIds(this.order, [source], target, after ? "after" : "before"); this.publish();
  }
  private publish(): void {
    const all = [...this.groups.values()].flat(), byId = new Map(all.map(tab => [tab.id, tab]));
    const ordered = new Set(this.order);
    // Lazy workspace owners publish at different times. Keep restored IDs
    // until their owner explicitly closes them; an absent host is not a close.
    this.order = [...this.order, ...all.filter(tab => !ordered.has(tab.id)).map(tab => tab.id)];
    this.saveOrder?.(this.order);
    this.current = byId;
    const next = this.order.flatMap(id => byId.has(id) ? [byId.get(id)!] : []);
    if (next.length === this.snapshot.length && next.every((tab, index) => samePresentation(tab, this.snapshot[index]!))) return;
    this.snapshot = next.map(tab => ({...tab,
      select: () => this.current.get(tab.id)?.select(),
      ...(tab.close ? {close: () => this.current.get(tab.id)?.close?.()} : {}),
      ...(tab.pin ? {pin: () => this.current.get(tab.id)?.pin?.()} : {}),
    }));
    this.listeners.forEach(listener => listener());
  }
}
