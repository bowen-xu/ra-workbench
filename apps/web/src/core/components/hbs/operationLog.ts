import type { HbsSession } from "@research-assistant/core/hbs";
import type { HbsDeliveryPort } from "./deliveryPort";

export interface HbsOperationEntry {
  readonly sequence: number;
  readonly time: string;
  readonly label: string;
  readonly detail: unknown;
  readonly channel: "operation" | "communication";
}

export interface HbsOperationLogOptions {
  readonly scope?: string;
  readonly maxEntries?: number;
  readonly maxBytes?: number;
  readonly maxDetailBytes?: number;
}

/** Local, passive diagnostics. Exported deltas include their before/after
 * values and stable paths; they never become review or persistence commands. */
export class HbsOperationLog {
  private entries: readonly HbsOperationEntry[] = [];
  private sizes: number[] = [];
  private bytes = 0;
  private sequence = 0;
  private dropped = 0;
  private truncated = 0;
  private readonly listeners = new Set<() => void>();
  private sessions = new WeakMap<object, number>();
  private readonly deliveries = new WeakMap<object, number>();
  private sessionSequence = 0;
  private readonly limits: Required<HbsOperationLogOptions>;
  constructor(options: HbsOperationLogOptions = {}) {
    this.limits = {scope: options.scope ?? "hbs-adapters", maxEntries: options.maxEntries ?? 2_000,
      maxBytes: options.maxBytes ?? 8 * 1024 * 1024, maxDetailBytes: options.maxDetailBytes ?? 256 * 1024};
    if (!Number.isInteger(this.limits.maxEntries) || this.limits.maxEntries < 1
      || !Number.isInteger(this.limits.maxBytes) || this.limits.maxBytes < 256
      || !Number.isInteger(this.limits.maxDetailBytes) || this.limits.maxDetailBytes < 64) throw new Error("MODEL_OPERATION_LOG_INVALID_LIMIT");
  }
  readonly getSnapshot = () => this.entries;
  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  getStats() { return {recorded: this.sequence, retained: this.entries.length, bytes: this.bytes, dropped: this.dropped,
    truncated: this.truncated, complete: this.dropped === 0 && this.truncated === 0, limits: this.limits}; }
  clear({forgetSessions = false}: {readonly forgetSessions?: boolean} = {}): void {
    this.entries = [];
    this.sizes = []; this.bytes = 0; this.sequence = 0; this.dropped = 0; this.truncated = 0;
    if (forgetSessions) { this.sessions = new WeakMap(); this.sessionSequence = 0; }
    // Keep observer watermarks so reattachment cannot import old messages.
    this.listeners.forEach(listener => listener());
  }
  record(label: string, detail: unknown = {}, channel: HbsOperationEntry["channel"] = "operation"): void {
    let value: unknown;
    try {
      const json = JSON.stringify(detail) ?? "null", bytes = new TextEncoder().encode(json).byteLength;
      if (bytes > this.limits.maxDetailBytes) {
        const context = detail && typeof detail === "object" ? Object.fromEntries(Object.entries(detail).filter(([key, part]) =>
          ["kind", "resourceId", "sessionId", "captureId", "batch", "version", "requestId"].includes(key)
          && (typeof part === "number" || typeof part === "string" && part.length <= 200))) : {};
        const marker = {truncated: true, originalBytes: bytes};
        value = {...context, ...marker};
        if (new TextEncoder().encode(JSON.stringify(value)).byteLength > this.limits.maxDetailBytes) value = marker;
        this.truncated++;
      }
      else value = JSON.parse(json);
    } catch { value = {unavailable: true, reason: "serialization-failed"}; this.truncated++; }
    const entry = {sequence: ++this.sequence, time: new Date().toISOString(), label, channel, detail: value};
    const size = new TextEncoder().encode(JSON.stringify(entry)).byteLength;
    this.entries = [...this.entries, entry]; this.sizes.push(size); this.bytes += size;
    while (this.entries.length > this.limits.maxEntries || this.bytes > this.limits.maxBytes) {
      this.entries = this.entries.slice(1); this.bytes -= this.sizes.shift()!; this.dropped++;
    }
    this.listeners.forEach(listener => listener());
  }
  observe<D>(session: HbsSession<D>, resourceId: string): () => void {
    let previous = session.getSnapshot();
    let sessionId = this.sessions.get(session);
    const fresh = sessionId === undefined;
    if (fresh) { sessionId = ++this.sessionSequence; this.sessions.set(session, sessionId); }
    const context = {kind: session.adapter.kind, resourceId, sessionId};
    if (fresh) {
      const {events: _events, message: _message, ...initial} = previous;
      this.record("会话开始", {...context, initial});
    }
    return session.subscribe(() => {
      const next = session.getSnapshot();
      try {
        const contentChanges = previous.content === next.content ? [] : session.adapter.diff(previous.content, next.content);
        const baselineChanges = previous.baselineProjection === next.baselineProjection ? [] : session.adapter.diff(previous.baselineProjection, next.baselineProjection);
        const ancestorChanges = previous.ancestor === next.ancestor ? [] : session.adapter.diff(previous.ancestor, next.ancestor);
        const reviewAncestorChanges = previous.reviewAncestor === next.reviewAncestor ? [] : session.adapter.diff(previous.reviewAncestor, next.reviewAncestor);
        const baselineAncestorChanges = previous.baselineAncestor === next.baselineAncestor ? [] : session.adapter.diff(previous.baselineAncestor, next.baselineAncestor);
        const preservedChanges = previous.preserved === undefined || next.preserved === undefined ? []
          : previous.preserved === next.preserved ? [] : session.adapter.diff(previous.preserved, next.preserved);
        const preservationChanged = (previous.preserved === undefined) !== (next.preserved === undefined) || !!preservedChanges.length;
        const versionChanged = previous.observed.version !== next.observed.version;
        if (contentChanges.length || baselineChanges.length || versionChanged || previous.pending !== next.pending
          || previous.queued !== next.queued || preservationChanged || previous.preservationError !== next.preservationError
          || ancestorChanges.length || reviewAncestorChanges.length || baselineAncestorChanges.length
          || JSON.stringify(previous.changes) !== JSON.stringify(next.changes) || JSON.stringify(previous.conflicts) !== JSON.stringify(next.conflicts)) {
          this.record("模型状态变化", {...context, version: next.observed.version, contentChanges, baselineChanges,
            ...(versionChanged ? {observed: next.observed} : {}), conflicts: next.conflicts, pending: next.pending, queued: next.queued,
            changes: next.changes, preservationError: next.preservationError,
            ...(preservationChanged ? {preservation: {available: next.preserved !== undefined, changes: preservedChanges,
              ...(previous.preserved === undefined && next.preserved !== undefined ? {initial: next.preserved} : {})}} : {}),
            ancestorChanges, reviewAncestorChanges, baselineAncestorChanges});
        }
        for (const message of next.events.slice(previous.events.length)) this.record("模型事件", {...context, version: next.observed.version, message}, "communication");
      } catch (reason) {
        this.record("诊断记录失败", {...context, error: reason instanceof Error ? reason.message : String(reason)});
      }
      previous = next;
    });
  }
  observeDelivery<D>(delivery: HbsDeliveryPort<D>, resourceId: string): () => void {
    const capture = () => {
      const events = delivery.getSnapshot().events;
      const seen = this.deliveries.get(delivery) ?? 0;
      for (const message of events.slice(seen)) this.record("通信消息", {resourceId, message}, "communication");
      this.deliveries.set(delivery, events.length);
    };
    capture(); return delivery.subscribe(capture);
  }
  exportJson(): string { return JSON.stringify({format: 1, scope: this.limits.scope, capture: this.getStats(), entries: this.entries}, null, 2); }
}

export function operationTarget(target: EventTarget | null): unknown {
  if (!(target instanceof Element)) return {};
  const field = target.closest("input, textarea");
  const selection = window.getSelection();
  const anchor = selection?.anchorNode;
  const anchorElement = anchor instanceof Element ? anchor : anchor?.parentElement;
  return {
    blockId: target.closest("[data-id]")?.getAttribute("data-id") ?? anchorElement?.closest("[data-id]")?.getAttribute("data-id"),
    field: target.getAttribute("aria-label") ?? field?.getAttribute("aria-label"),
    ...(field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement ? {from: field.selectionStart, to: field.selectionEnd} : {}),
    ...(selection?.rangeCount ? {anchorOffset: selection.anchorOffset, focusOffset: selection.focusOffset,
      anchorBlockId: anchorElement?.closest("[data-id]")?.getAttribute("data-id"),
      focusBlockId: (selection.focusNode instanceof Element ? selection.focusNode : selection.focusNode?.parentElement)?.closest("[data-id]")?.getAttribute("data-id")} : {}),
  };
}
