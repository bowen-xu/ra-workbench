import type { AgentModelIdentity } from "@research-assistant/core/agent";
import type { EventCallback } from "@tauri-apps/api/event";
import type { PlatformEvents } from "./platformEvents";
import {NativePreviewResumeHold} from "./nativePreviewResumeHold";

export interface NativeAgentPreviewBootstrap {
  readonly schemaVersion: 1;
  readonly generation: string;
  readonly model: AgentModelIdentity;
  readonly manuscriptId: string;
  readonly mechanism: "native-agent-raft-sqlite";
  readonly supported: readonly string[];
  readonly pending: readonly string[];
}
export interface NativeAgentPreviewSample {
  readonly sse: string;
  readonly chunkBytes?: number;
  readonly delayMs?: number;
  readonly failAfterChunks?: number;
  readonly httpStatus?: number;
  readonly actualModel?: string;
}
export interface NativeAgentPreviewStatus { readonly error: unknown | null }
export interface NativeAgentPreviewInvokeOptions {
  readonly signal?: AbortSignal;
  /** Opt in only where the caller knows the command's conversation identity. */
  readonly previewOwnerConversationId?: string;
  /** Owner recovered from the original request's durable preview intent. */
  readonly previewOwnerIntent?: {readonly conversationId: string; readonly fixtureId: string | null};
}
export interface NativeAgentPreview {
  readonly bootstrap: NativeAgentPreviewBootstrap;
  readonly invoke: (command: string, args?: Record<string, unknown>, options?: NativeAgentPreviewInvokeOptions) => Promise<unknown>;
  /** Bind or release the fixture for future opt-in dispatches of this conversation. */
  setConversationOwner?(conversationId: string, fixtureId: string | null, generation: string): void;
  /** Snapshot the owner for a newly-created request before any asynchronous work. */
  snapshotConversationOwner?(conversationId: string): {generation: string; fixtureId: string | null};
  setOwnerIntentClearer?(clear: () => Promise<void>): void;
  readonly events: PlatformEvents;
  configure(samples: readonly NativeAgentPreviewSample[]): Promise<void>;
  holdNextContextResume?(onCommitted: () => void): () => void;
  reconnect(): void;
  reset(onOwnerDrained?: () => void): Promise<void>;
  subscribe(changed: () => void): () => void;
  getSnapshot(): NativeAgentPreviewStatus;
  dispose(): void;
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("agent_preview_invalid_response");
  return value as Record<string, unknown>;
}
function integer(value: unknown): value is number { return Number.isSafeInteger(value) && Number(value) >= 0; }
const historyReads = new Set(["agent_open_conversation", "agent_read_history", "agent_wait_for_history"]);
function uuid(value: unknown): value is string { return typeof value === "string" && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu.test(value); }
function decodeBootstrap(value: unknown): NativeAgentPreviewBootstrap {
  const data = object(value), model = object(data.model);
  if (data.schemaVersion !== 1 || data.mechanism !== "native-agent-raft-sqlite" || !uuid(data.generation) || !uuid(data.manuscriptId)
    || typeof model.providerId !== "string" || typeof model.modelId !== "string" || !model.providerId || !model.modelId
    || (model.revision !== undefined && model.revision !== null && typeof model.revision !== "string")
    || !Array.isArray(data.supported) || data.supported.some(item => typeof item !== "string")
    || !Array.isArray(data.pending) || data.pending.some(item => typeof item !== "string")) throw new Error("agent_preview_invalid_bootstrap");
  return data as unknown as NativeAgentPreviewBootstrap;
}

/** Same-origin Vite proxy owns private authentication. No secret reaches JS. */
export async function createNativeAgentPreview(fetcher: typeof fetch = fetch): Promise<NativeAgentPreview> {
  const resumeHold = new NativePreviewResumeHold();
  const controllers = new Set<AbortController>();
  const listeners = new Map<string, Set<EventCallback<unknown>>>();
  const observers = new Set<() => void>();
  const conversationOwners = new Map<string, string>();
  let disposed = false, polling = false, paused = false, generationReady = true, sequence = 0, revision = 0, generationVersion = 0;
  let resyncPosition: number | null = null;
  let resetting: Promise<void> | undefined;
  let ownerIntentClearer: (() => Promise<void>) | undefined;
  let confirmedReset: {generation: string; detach?: () => void; detached: boolean; cleared: boolean} | undefined;
  let status: NativeAgentPreviewStatus = { error: null };
  let pollController: AbortController | undefined;
  const setError = (error: unknown | null): void => { status = { error }; observers.forEach(changed => changed()); };
  async function post(path: "invoke" | "events", payload: unknown, controller = new AbortController()): Promise<unknown> {
    if (disposed) throw new Error("agent_preview_closed");
    controllers.add(controller);
    try {
      const response = await fetcher(`/__agent_runtime__/${path}`, { method: "POST", headers: { "content-type": "application/json" },
        credentials: "same-origin", body: JSON.stringify(payload), signal: controller.signal });
      let data: Record<string, unknown>;
      try { data = object(await response.json()); } catch { throw new Error(`agent_preview_invalid_http_response: HTTP ${response.status}`); }
      if (!response.ok) {
        // Synthetic I/O failures belong to this preview transport, not a
        // production enum/DTO decoder. Keep their stable code and HTTP status
        // together so the actual recovery controls can render both safely.
        if (typeof data.error === "string" && /^AGENT_PREVIEW_SYNTHETIC_IO_[A-Z_]+$/u.test(data.error)) {
          throw {code: data.error, status: response.status};
        }
        // Keep production structured errors intact for the existing decoders.
        if (data.error !== undefined) throw data.error;
        throw { code: typeof data.code === "string" ? data.code : "agent_preview_http_error", message: typeof data.message === "string" ? data.message : `HTTP ${response.status}`, status: response.status };
      }
      if (disposed) throw new Error("agent_preview_closed");
      if (!("result" in data)) throw new Error("agent_preview_missing_result");
      return data.result;
    } finally { controllers.delete(controller); }
  }
  const invoke = async (command: string, args?: Record<string, unknown>, options?: NativeAgentPreviewInvokeOptions) => {
    if (!generationReady) throw new Error("agent_preview_recovery_required");
    const owner = generationVersion;
    const conversationId = options?.previewOwnerIntent?.conversationId ?? options?.previewOwnerConversationId;
    if (conversationId !== undefined && !uuid(conversationId)) throw new Error("agent_preview_owner_invalid");
    if (options?.previewOwnerIntent?.fixtureId != null && !uuid(options.previewOwnerIntent.fixtureId)) throw new Error("agent_preview_owner_invalid");
    // Capture before dispatch: changing the map cannot retarget an in-flight operation.
    const previewOwner = conversationId === undefined ? undefined : {
      generation: bootstrap.generation, conversationId: conversationId.toLowerCase(),
      fixtureId: options?.previewOwnerIntent ? options.previewOwnerIntent.fixtureId : conversationOwners.get(conversationId.toLowerCase()) ?? null,
    };
    // Only read-only history/recommendation requests are cancellable: interrupting a dispatched
    // mutation would leave its commit outcome ambiguous.
    const signal = ["agent_wait_for_history", "agent_recommend_context"].includes(command) ? options?.signal : undefined;
    if (signal?.aborted) throw signal.reason ?? new DOMException("Aborted", "AbortError");
    const controller = new AbortController();
    const abort = (): void => controller.abort();
    signal?.addEventListener("abort", abort, { once: true });
    try {
      const result = await post("invoke", { command, args: args ?? {}, ...(previewOwner ? {previewOwner} : {}) }, controller);
      await resumeHold.after(command, args);
      if (owner !== generationVersion) throw new Error("agent_preview_stale_response: 存储代次已变化，请核对当前状态。");
      if (!generationReady) throw new Error("agent_preview_recovery_required");
      return result;
    } catch (error) {
      // Closing the old native registry can finish its history wait before the
      // reset receipt reaches JS. Hold only these read errors at that boundary;
      // editor flush/delivery ACK commands must continue draining normally.
      if (owner === generationVersion && resetting && historyReads.has(command)) await resetting.catch(() => undefined);
      if (owner !== generationVersion) throw new Error("agent_preview_stale_response: 存储代次已变化，请核对当前状态。");
      throw error;
    } finally { signal?.removeEventListener("abort", abort); }
  };
  let bootstrap = decodeBootstrap(await invoke("agent_preview_bootstrap"));
  const active = (): boolean => !disposed && [...listeners.values()].some(set => set.size > 0);
  async function poll(): Promise<void> {
    if (polling || !active() || paused || !generationReady || resyncPosition !== null) return;
    polling = true;
    const current = revision;
    try {
      while (active() && revision === current) {
        const controller = new AbortController(); pollController = controller;
        const page = object(await post("events", { afterSequence: sequence, waitMs: 30_000 }, controller));
        if (!active() || revision !== current) return;
        if (!integer(page.nextSequence) || typeof page.resyncRequired !== "boolean" || !Array.isArray(page.events)) throw new Error("agent_preview_invalid_event_page");
        if (page.resyncRequired) {
          // Remember evidence for an explicit reset. Ordinary reconnect cannot
          // skip editor ACKs or pretend that the missing requests were handled.
          resyncPosition = page.nextSequence;
          throw new Error("agent_preview_events_resync_required: 事件记录已过期，请重置隔离预览。");
        }
        for (const value of page.events) {
          const event = object(value);
          if (!integer(event.sequence) || event.sequence !== sequence + 1 || typeof event.name !== "string" || !("payload" in event)) throw new Error("agent_preview_event_gap");
          for (const handler of listeners.get(event.name) ?? []) handler({ event: event.name, id: event.sequence, payload: event.payload });
          sequence = event.sequence;
        }
        if (page.nextSequence !== sequence) throw new Error("agent_preview_invalid_event_cursor");
      }
    } catch (error) { if (active() && revision === current && !pollController?.signal.aborted) setError(error); }
    finally { polling = false; pollController = undefined; if (active() && revision !== current) void poll(); }
  }
  async function resetOwner(onOwnerDrained?: () => void): Promise<void> {
    // Keep ACK subscribers alive while the native owner cancels and drains its
    // old tasks. Only the confirmed new generation permits abandoning its edge.
    // A lost reply leaves this confirmed generation unchanged. Explicit retry
    // must target it again instead of resetting a replacement environment.
    if (!confirmedReset) {
      const response = object(await post("invoke", { command: "agent_preview_reset", args: { request: { expectedGeneration: bootstrap.generation } } }));
      if (response.schemaVersion !== 1 || !uuid(response.generation)) throw new Error("agent_preview_invalid_reset_receipt");
      confirmedReset = {generation: response.generation, ...(onOwnerDrained ? {detach: onOwnerDrained} : {}), detached: false, cleared: false};
      generationVersion += 1; generationReady = false; conversationOwners.clear(); paused = true; revision += 1; pollController?.abort();
    }
    const confirmed = confirmedReset;
    try {
      // Native execution no longer needs old ACK hosts. Detach those views
      // synchronously before old invoke completions can publish stale errors.
      // A failed detach still invalidates the deleted generation.
      if (!confirmed.detached) { confirmed.detach?.(); confirmed.detached = true; }
      if (!confirmed.cleared) { await ownerIntentClearer?.(); confirmed.cleared = true; }
      const next = decodeBootstrap(await post("invoke", { command: "agent_preview_bootstrap", args: {} }));
      if (next.generation !== confirmed.generation) throw new Error("agent_preview_reset_identity_mismatch");
      let cursor = resyncPosition ?? sequence;
      for (;;) {
        let page: Record<string, unknown>;
        try { page = object(await post("events", { afterSequence: cursor, waitMs: 0 })); }
        catch (error) {
          // A restarted backend has a new event counter. Only the explicit,
          // confirmed reset boundary permits leaving that old namespace.
          if (cursor > 0 && error === "preview_event_cursor_invalid") { cursor = 0; continue; }
          throw error;
        }
        if (!integer(page.nextSequence) || page.nextSequence < cursor || typeof page.resyncRequired !== "boolean" || !Array.isArray(page.events)
          || (page.events.length > 0 && page.nextSequence === cursor)) throw new Error("agent_preview_invalid_reset_cursor");
        cursor = page.nextSequence;
        if (page.resyncRequired || page.events.length < 64) break;
      }
      sequence = cursor; resyncPosition = null; bootstrap = next; generationReady = true; confirmedReset = undefined;
      setError(null);
    } catch (error) { setError(error); throw error; }
    finally { paused = false; void poll(); }
  }
  return {
    get bootstrap() { return bootstrap; }, invoke,
    setOwnerIntentClearer(clear) { ownerIntentClearer = clear; },
    snapshotConversationOwner(conversationId) {
      if (disposed || !generationReady) throw new Error("agent_preview_recovery_required");
      if (!uuid(conversationId)) throw new Error("agent_preview_owner_invalid");
      return {generation: bootstrap.generation, fixtureId: conversationOwners.get(conversationId.toLowerCase()) ?? null};
    },
    setConversationOwner(conversationId, fixtureId, generation) {
      if (disposed) throw new Error("agent_preview_closed");
      if (!generationReady) throw new Error("agent_preview_recovery_required");
      if (!uuid(conversationId) || !uuid(generation) || (fixtureId !== null && !uuid(fixtureId))) throw new Error("agent_preview_owner_invalid");
      // Late fixture-creation completions cannot install an owner for a new store.
      if (generation.toLowerCase() !== bootstrap.generation.toLowerCase()) throw new Error("agent_preview_stale_owner");
      if (fixtureId === null) conversationOwners.delete(conversationId.toLowerCase());
      else conversationOwners.set(conversationId.toLowerCase(), fixtureId.toLowerCase());
    },
    events: { async listen<T>(name: string, handler: EventCallback<T>) {
      if (disposed) throw new Error("agent_preview_closed");
      const owned = listeners.get(name) ?? new Set<EventCallback<unknown>>(); listeners.set(name, owned);
      const callback = handler as EventCallback<unknown>; owned.add(callback); void poll();
      return () => { owned.delete(callback); if (!active()) { revision += 1; pollController?.abort(); } };
    } },
    async configure(samples) { const response = object(await invoke("agent_preview_configure", { scenario: { samples } })); if (response.schemaVersion !== 1) throw new Error("agent_preview_invalid_configuration_receipt"); },
    holdNextContextResume(onCommitted) {return resumeHold.arm(onCommitted);},
    reconnect() { if (!generationReady || resyncPosition !== null || resetting) return; revision += 1; pollController?.abort(); setError(null); void poll(); },
    reset(onOwnerDrained) { resetting ??= resetOwner(onOwnerDrained).catch(error => { generationReady = false; setError(error); throw error; }).finally(() => { resetting = undefined; }); return resetting; },
    subscribe(changed) { observers.add(changed); return () => { observers.delete(changed); }; },
    getSnapshot: () => status,
    dispose() { disposed = true; resumeHold.cancel(); revision += 1; controllers.forEach(controller => controller.abort()); controllers.clear(); listeners.clear(); observers.clear(); },
  };
}
