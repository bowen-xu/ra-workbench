import { decodeAgentWritingQueueCancelRequest, decodeAgentWritingQueueCancelResult, decodeAgentWritingQueueCancelStatusResponse,
  type AgentWritingQueueCancelRequest, type AgentWritingQueueCancelStatusResponse, type AgentWritingQueueScope, type AgentWritingQueuedStartRequest } from "@research-assistant/core/agent";
import type { AgentInputQueuePreviewGateway } from "@research-assistant/data/agent";
import { getAppStorage } from "../../runtime/appStorage";
import { flushStorage, storagePersistence } from "../../runtime/storagePersistence";

export interface WritingQueueCancellationSnapshot {
  readonly phase: "idle" | "persisting" | "sending" | "unknown" | "checking" | "notFound" | "unconfirmed" | "clearing" | "localError" | "ready";
  readonly intent: AgentWritingQueueCancelRequest | null;
  readonly status: AgentWritingQueueCancelStatusResponse["status"] | null;
  readonly knownCommitted: boolean;
  readonly definitiveConflict: boolean;
  readonly cause: unknown;
}
export interface WritingQueueCancellationRecovery { readonly scope?: AgentWritingQueueScope; readonly storage: Storage }
export function writingQueueCancellationRecovery(scope: {manuscriptId: string | undefined; conversationId: string | null}): WritingQueueCancellationRecovery {
  return {storage: getAppStorage(), ...(scope.manuscriptId && scope.conversationId ? {scope: {manuscriptId: scope.manuscriptId, conversationId: scope.conversationId}} : {})};
}
const initial = (): WritingQueueCancellationSnapshot => ({phase: "idle", intent: null, status: null, knownCommitted: false, definitiveConflict: false, cause: null});
const key = (scope: AgentWritingQueueScope) => `research-assistant:writing-queue-cancel:v1:${scope.manuscriptId.toLowerCase()}:${scope.conversationId.toLowerCase()}`;
export const sameWritingQueueCancellationIntent = (a: AgentWritingQueueCancelRequest, b: AgentWritingQueueCancelRequest) => JSON.stringify(a) === JSON.stringify(b);
const domainReasons: Readonly<Record<string, string>> = {
  invalidInput: "invalidInput", notFound: "notFound", conflict: "conflict", busy: "busy", tooLarge: "tooLarge", corrupt: "corrupt", unavailable: "unavailable",
  "invalid conversation catalog input": "invalidInput", "conversation not found": "notFound", "conversation changed since it was read": "conflict",
  "conversation has an unfinished turn": "busy", "conversation exceeds the validated history budget": "tooLarge",
  "conversation catalog unavailable": "unavailable", "conversation metadata is corrupt": "corrupt",
};
export function writingQueueCancellationDomain(cause: unknown): {classification: string; safeReason: string} | null {
  if (!cause || typeof cause !== "object") return null;
  const source = cause as Record<string, unknown>, row = source.code === "storage" && source.reason && typeof source.reason === "object" ? source.reason as Record<string, unknown> : source;
  if (row.code !== "DATA_SQL_DOMAIN" || typeof row.reason !== "string" || !Object.hasOwn(domainReasons, row.reason)) return null;
  return {classification: domainReasons[row.reason]!, safeReason: row.reason};
}
function definitiveConflict(cause: unknown, intent: AgentWritingQueueCancelRequest): boolean {
  if (!cause || typeof cause !== "object" || !("code" in cause) || cause.code !== "storage" || !("reason" in cause)
    || !cause.reason || typeof cause.reason !== "object") return false;
  const row = cause.reason as Record<string, unknown>;
  return writingQueueCancellationDomain(cause)?.classification === "conflict" && row.conversationId === intent.conversationId
    && row.manuscriptId === intent.manuscriptId && row.inputId === intent.inputId && row.previewRequestId === intent.previewRequestId && row.operationId === intent.requestId;
}

/** A substate of the existing submission owner, never a second execution owner.
 * The local envelope is only a locator; native status proves cancellation. */
export class WritingQueueCancellation {
  private state = initial();
  private reserved = false;
  private clearing = false;
  private retiring = false;
  private attempted = false;
  private disposed = false;
  private pending: Promise<void> | null = null;
  private replacementFrom: AgentWritingQueueCancelRequest | null = null;
  constructor(private readonly gateway: AgentInputQueuePreviewGateway, private readonly storage: Storage,
    private readonly changed: (state: WritingQueueCancellationSnapshot) => void, scope?: AgentWritingQueueScope) {
    if (scope) this.restore(scope);
  }
  getSnapshot = () => this.state;
  unresolved = () => this.state.phase !== "idle" && this.state.phase !== "ready";
  private update(patch: Partial<WritingQueueCancellationSnapshot>): void {
    if (this.disposed) return;
    this.state = {...this.state, ...patch}; this.changed(this.state);
  }
  private read(scope: AgentWritingQueueScope): {intent: AgentWritingQueueCancelRequest; definitiveConflict: boolean} | null {
    const raw = this.storage.getItem(key(scope)); if (raw === null) return null;
    try {
      const value: unknown = JSON.parse(raw);
      if (!value || typeof value !== "object" || Object.keys(value).some(name => !["schemaVersion", "intent", "definitiveConflict"].includes(name))
        || !("schemaVersion" in value) || value.schemaVersion !== 1 || !("intent" in value)
        || ("definitiveConflict" in value && typeof value.definitiveConflict !== "boolean")) throw new Error();
      const intent = decodeAgentWritingQueueCancelRequest(value.intent);
      if (intent.conversationId !== scope.conversationId.toLowerCase() || intent.manuscriptId !== scope.manuscriptId.toLowerCase()) throw new Error();
      return {intent, definitiveConflict: "definitiveConflict" in value && value.definitiveConflict === true};
    } catch { throw new Error("QUEUE_CANCEL_RECOVERY_INVALID"); }
  }
  restore(scope: AgentWritingQueueScope): boolean {
    if (this.disposed) return true;
    if (this.unresolved()) return true;
    try {
      const saved = this.read(scope);
      if (saved) { this.reserved = true; this.attempted = true; this.retiring = false; this.update({...initial(), phase: "unknown", ...saved}); }
    } catch (cause: unknown) { this.update({...initial(), phase: "localError", cause}); }
    return this.unresolved();
  }
  private async persist(intent: AgentWritingQueueCancelRequest): Promise<void> {
    const saved = this.read(intent);
    const replacing = this.replacementFrom && saved && saved.definitiveConflict && sameWritingQueueCancellationIntent(saved.intent, this.replacementFrom);
    if ((this.reserved && !saved) || (saved && !sameWritingQueueCancellationIntent(saved.intent, intent) && !replacing)) throw new Error("QUEUE_CANCEL_RECOVERY_CONFLICT");
    this.storage.setItem(key(intent), JSON.stringify({schemaVersion: 1, intent, definitiveConflict: this.state.definitiveConflict})); this.reserved = true;
    await flushStorage(this.storage); this.replacementFrom = null;
  }
  private async clear(retired = false): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return;
    this.update({phase: "clearing", cause: null});
    try {
      const saved = this.read(intent);
      if ((!saved && !this.clearing) || (saved && !sameWritingQueueCancellationIntent(saved.intent, intent))) throw new Error("QUEUE_CANCEL_RECOVERY_CONFLICT");
      this.storage.removeItem(key(intent)); this.clearing = true; await flushStorage(this.storage);
      this.update({...initial(), phase: "ready", knownCommitted: !retired}); this.reserved = false; this.clearing = false;
    } catch (cause: unknown) { this.update({phase: "localError", cause}); }
  }
  private async retained(action: () => Promise<void>): Promise<void> {
    if (this.disposed) return;
    if (this.pending) return this.pending;
    const pending = action(); this.pending = pending;
    try { await pending; } finally { if (this.pending === pending) this.pending = null; }
  }
  begin(value: AgentWritingQueueCancelRequest): Promise<void> {
    return this.retained(async () => {
      if (this.unresolved()) return;
      const intent = decodeAgentWritingQueueCancelRequest(value);
      if (this.restore(intent)) return;
      this.attempted = false; this.retiring = false; this.update({...initial(), phase: "persisting", intent});
      await this.send();
    });
  }
  private async send(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed || this.state.knownCommitted) return;
    this.update({phase: "persisting", cause: null});
    try { await this.persist(intent); }
    catch (cause: unknown) { this.update({phase: "localError", cause}); return; }
    if (this.disposed) return;
    this.attempted = true; this.update({phase: "sending"});
    try {
      decodeAgentWritingQueueCancelResult(await this.gateway.cancel(structuredClone(intent)), intent);
      if (this.disposed) return;
      this.update({knownCommitted: true}); await this.clear();
    } catch (cause: unknown) {
      if (this.disposed) return;
      const rejected = definitiveConflict(cause, intent);
      this.update({phase: "unknown", cause, definitiveConflict: this.state.definitiveConflict || rejected});
      if (rejected) try {await this.persist(intent);} catch (local: unknown) {this.update({phase: "localError", cause: local});}
    }
  }
  private async inspect(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return;
    if (this.state.knownCommitted) { await this.clear(); return; }
    this.update({phase: "checking", cause: null});
    try {
      const response = decodeAgentWritingQueueCancelStatusResponse(await this.gateway.cancelStatus(structuredClone(intent)));
      if (!sameWritingQueueCancellationIntent(response.intent, intent)) throw new Error("QUEUE_CANCEL_PROOF_MISMATCH");
      if (this.disposed) return;
      this.update({status: response.status, phase: response.status.type === "committed" ? "clearing" : response.status.type,
        knownCommitted: response.status.type === "committed"});
      if (this.state.knownCommitted) await this.clear();
    } catch (cause: unknown) { this.update({phase: "unknown", cause}); }
  }
  check = (): Promise<void> => this.retained(() => this.inspect());
  private async refreshRejected(readCurrent?: AgentInputQueuePreviewGateway["recover"]): Promise<void> {
    const original = this.state.intent; if (this.disposed || !original || !this.state.definitiveConflict || this.state.status?.type !== "notFound") return;
    try {
      await this.persist(original);
      if (this.disposed) return;
      const scope = {conversationId: original.conversationId, manuscriptId: original.manuscriptId};
      const current = readCurrent ? await readCurrent(scope) : await this.gateway.recover(scope);
      if (this.disposed) return;
      if (!current || current.inputId !== original.inputId || current.preview.requestId !== original.previewRequestId
        || current.preview.conversationId !== original.conversationId || current.preview.manuscriptId !== original.manuscriptId
        || current.queue.conversationId !== original.conversationId || current.queue.manuscriptId !== original.manuscriptId
        || current.queue.preparingInputId !== original.inputId || current.queue.revision <= original.expectedRevision)
        throw new Error("QUEUE_CANCEL_PREVIEW_CHANGED");
      const intent = decodeAgentWritingQueueCancelRequest({...original, requestId: crypto.randomUUID(), expectedRevision: current.queue.revision});
      this.replacementFrom = original; this.attempted = false; this.update({...initial(), phase: "persisting", intent});
      await this.send();
    } catch (cause: unknown) {this.update({phase: "localError", cause});}
  }
  retry = (readCurrent?: AgentInputQueuePreviewGateway["recover"]): Promise<void> => this.retained(async () => {
    if (!this.state.intent) return;
    try { if (storagePersistence(this.storage)?.error) await storagePersistence(this.storage)!.retry(); }
    catch (cause: unknown) { this.update({phase: "localError", cause}); return; }
    if (this.disposed) return;
    if (this.retiring) {await this.clear(true); return;}
    if (this.state.knownCommitted) { await this.clear(); return; }
    if (!this.attempted) { await this.send(); return; }
    await this.inspect();
    if (this.state.phase === "notFound" && this.state.definitiveConflict) await this.refreshRejected(readCurrent);
    else if (this.state.phase === "notFound" || this.state.phase === "unconfirmed") await this.send();
  });
  /** A proved original start can retire only its matching cancellation locator.
   * Inspecting this status never sends a cancellation write. */
  retireAfterStarted(start: AgentWritingQueuedStartRequest): Promise<void> {
    return this.retained(async () => {
      const target = {conversationId: start.start.conversationId, manuscriptId: start.start.manuscriptId};
      const matches = (intent: AgentWritingQueueCancelRequest) => intent.conversationId === target.conversationId && intent.manuscriptId === target.manuscriptId
        && intent.inputId === start.inputId && intent.previewRequestId === start.start.requestId;
      const saved = this.read(target);
      if (!saved) {
        if (this.clearing && this.state.intent && matches(this.state.intent)) {
          await this.clear(true); if (this.state.phase !== "ready") throw this.state.cause;
        }
        return;
      }
      if (!matches(saved.intent) || (this.unresolved() && this.state.intent && !sameWritingQueueCancellationIntent(this.state.intent, saved.intent))) return;
      const response = decodeAgentWritingQueueCancelStatusResponse(await this.gateway.cancelStatus(structuredClone(saved.intent)));
      if (!sameWritingQueueCancellationIntent(response.intent, saved.intent)) throw new Error("QUEUE_START_CANCEL_MISMATCH");
      if (this.disposed) return;
      if (response.status.type !== "notFound") throw {code: "QUEUE_START_CANCEL_UNCONFIRMED", cancelOperation: saved.intent.requestId,
        ...(response.status.type === "unconfirmed" ? {detail: response.status.detail} : {reason: "cancel_committed_but_start_started"})};
      const current = this.read(target); if (!current || !sameWritingQueueCancellationIntent(current.intent, saved.intent)) return;
      this.state = {...initial(), phase: "unknown", ...current}; this.reserved = true; this.retiring = true;
      await this.clear(true);
      if (this.state.phase !== "ready") throw this.state.cause;
    });
  }
  dispose(): void { this.disposed = true; }
}
