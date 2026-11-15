import type { AgentConversationBinding, AgentDeletionRequest } from "@research-assistant/core/agent";
import type { AgentDeletionGateway } from "@research-assistant/data/agent";
import { appStorageScopedKey } from "../../runtime/appStorage";
import { flushStorage, storagePersistence } from "../../runtime/storagePersistence";
import { ReviewedSubmissionOwnerRegistry } from "./reviewedSubmissionOwner";
import { assertConversationOperationStorageCurrent, browserConversationOperationLock } from "./conversationOperationLock";
import { acquireConversationDeletionOwner, type ConversationDeletionController, type ConversationDeletionPorts,
  type ConversationDeletionSnapshot } from "./conversationDeletionController";
import { readConversationDeletion, sameDeletionIntent } from "./conversationDeletionPending";
import { clearConversationDeletionBatch, decodeConversationDeletionBatch, deletionBatchScope, discoverConversationDeletionBatches,
  readConversationDeletionBatch, saveConversationDeletionBatch, type ConversationDeletionBatch } from "./conversationDeletionBatchPending";

export interface ConversationDeletionBatchSnapshot {
  readonly phase: "idle" | "saving" | "checking" | "running" | "waiting" | "ready";
  readonly batch: ConversationDeletionBatch | null;
  readonly items: Readonly<Record<string, ConversationDeletionSnapshot>>;
  readonly issue: unknown;
}
const browserLock = browserConversationOperationLock("DELETION_BATCH");
export function deletionBatchMemberSettled(item: ConversationDeletionSnapshot): boolean {
  return !item.localIssue && (item.phase === "ready" || item.phase === "committed"
    || item.phase === "rejected" && item.record?.reason !== "busy");
}
/** Explicitly end only when every outstanding member is known unsent. An
 * unknown/file-partial member can never be abandoned through this action. */
export function canRetainUnsentBatchMembers(state: ConversationDeletionBatchSnapshot): boolean {
  const batch = state.batch;
  if (!batch || batch.phase === "finishing" || state.issue) return false;
  let unsent = false;
  const safe = batch.requests.every(request => {
    const item = state.items[request.conversationId];
    if (!item) {unsent = true; return batch.epoch === null;}
    if (item.localIssue) return false;
    if (item.record?.stage === "pending" || item.record?.stage === "rejected") {unsent = true; return true;}
    return deletionBatchMemberSettled(item);
  });
  return safe && unsent;
}
/** Composition of the existing per-conversation deletion owner. This retains
 * the immutable batch authorization; it never implements member writes. */
export class ConversationDeletionBatchController {
  private state: ConversationDeletionBatchSnapshot = {phase: "idle", batch: null, items: {}, issue: null};
  private saved: ConversationDeletionBatch | null = null;
  private operating = false; private disposed = false;
  private readonly members = new Map<string, ReturnType<typeof acquireConversationDeletionOwner>>();
  private readonly listeners = new Set<() => void>();
  constructor(private readonly binding: AgentConversationBinding | null, private readonly internal: boolean,
    private readonly ports: ConversationDeletionPorts, private readonly storage: Storage) {
    try {
      const found = discoverConversationDeletionBatches(storage).filter(batch => deletionBatchScope(batch.binding, batch.internal) === deletionBatchScope(binding, internal));
      if (found.length > 1) throw Error("DELETION_BATCH_CONFLICT");
      if (found[0]) {this.saved = found[0]; this.state = {...this.state, phase: "waiting", batch: found[0]};}
    } catch (issue: unknown) {this.state = {...this.state, phase: "waiting", issue};}
  }
  getSnapshot = (): ConversationDeletionBatchSnapshot => this.state;
  subscribe = (listener: () => void): (() => void) => {this.listeners.add(listener); return () => this.listeners.delete(listener);};
  private update(patch: Partial<ConversationDeletionBatchSnapshot>): void {this.state = {...this.state, ...patch}; for (const listener of this.listeners) listener();}
  hasPendingOperation(): boolean {return this.operating || this.state.batch !== null && this.state.phase !== "ready";}
  private async exclusive(action: () => Promise<boolean>, repair = false): Promise<boolean> {
    if (this.disposed || this.operating || this.state.phase === "ready") return false;
    this.operating = true; this.update({phase: "saving"});
    try {
      return await (this.ports.lock ?? browserLock).run(appStorageScopedKey(this.storage,
        `research-assistant:conversation-deletion-batch-lock:v1:${deletionBatchScope(this.binding, this.internal)}`), async () => {
        if (repair) await storagePersistence(this.storage)?.retry();
        await assertConversationOperationStorageCurrent(this.storage, "DELETION_BATCH");
        if (this.disposed) return false;
        if (this.saved && JSON.stringify(readConversationDeletionBatch(this.storage, this.saved.batchId)) !== JSON.stringify(this.saved)) throw Error("DELETION_BATCH_CONFLICT");
        this.update({issue: null}); return action();
      });
    } catch (issue: unknown) {this.update({issue}); return false;}
    finally {this.operating = false; if (this.getSnapshot().phase !== "ready") this.update({phase: "waiting"}); else this.update({});}
  }
  private async persist(batch: ConversationDeletionBatch): Promise<void> {
    this.update({batch});
    // Different host scopes may overlap in the global catalog. Serialize the
    // whole-manifest reservation, including its durable ACK, across those scopes.
    await (this.ports.lock ?? browserLock).run(appStorageScopedKey(this.storage, "research-assistant:conversation-deletion-batch-admission:v1"), async () => {
      await assertConversationOperationStorageCurrent(this.storage, "DELETION_BATCH");
      saveConversationDeletionBatch(this.storage, batch); this.saved = batch;
      await flushStorage(this.storage);
      if (JSON.stringify(readConversationDeletionBatch(this.storage, batch.batchId)) !== JSON.stringify(batch)) throw Error("DELETION_BATCH_CONFLICT");
    });
  }
  start(requests: readonly AgentDeletionRequest[]): Promise<boolean> {
    const original = structuredClone(requests);
    return this.exclusive(async () => {
      if (this.state.batch) return false;
      const batch = decodeConversationDeletionBatch({schemaVersion: 1, batchId: crypto.randomUUID(), binding: this.binding,
        internal: this.internal, epoch: null, phase: "authorized", retained: [], requests: original});
      await this.persist(batch); return this.runOwned();
    });
  }
  private member(request: AgentDeletionRequest): ConversationDeletionController {
    const record = readConversationDeletion(this.storage, request.conversationId);
    if (record && !sameDeletionIntent(record.intent, request)) throw Error("DELETION_BATCH_CONFLICT");
    let lease = this.members.get(request.conversationId);
    if (!lease) {
      lease = acquireConversationDeletionOwner(request.conversationId, this.ports, this.storage);
      const original = lease.controller.getSnapshot().record;
      if (original && !sameDeletionIntent(original.intent, request)) {lease.release(); throw Error("DELETION_BATCH_CONFLICT");}
      this.members.set(request.conversationId, lease);
    }
    return lease.controller;
  }
  private capture(request: AgentDeletionRequest, controller: ConversationDeletionController): void {
    this.update({items: {...this.state.items, [request.conversationId]: controller.getSnapshot()}});
  }
  private async prepare(repair = false): Promise<boolean> {
    let batch = this.state.batch; if (!batch) return false;
    // Reaffirm reliability before readonly IPC as well as before any member write.
    await this.persist(batch);
    if (batch.epoch === null && batch.phase === "authorized") {
      const first = batch.requests[0]!, controller = this.member(first);
      if (repair && controller.getSnapshot().localIssue) await controller.retryLocal();
      if (!await controller.prepareBatchMember(batch, first)) {this.capture(first, controller); return false;}
      await controller.check(); this.capture(first, controller);
      const observed = controller.getSnapshot();
      if (observed.localIssue || observed.cause || observed.record?.epoch === null || !observed.record) return false;
      batch = {...batch, epoch: observed.record.epoch};
      await this.persist(batch);
    }
    // Reserve the whole set through the same member locks before the first
    // mutation. A conflict or local-save failure cannot shrink the frozen set.
    for (const request of batch.requests) {
      if (batch.phase === "finishing" && batch.retained.includes(request.conversationId) && !readConversationDeletion(this.storage, request.conversationId)) continue;
      const controller = this.member(request);
      if (repair && controller.getSnapshot().localIssue) await controller.retryLocal();
      const reserved = await controller.prepareBatchMember(batch, request); this.capture(request, controller);
      if (!reserved) return false;
    }
    return true;
  }
  check(): Promise<boolean> {return this.exclusive(() => this.inspectOwned());}
  retryLocal(): Promise<boolean> {return this.exclusive(() => this.inspectOwned(true), true);}
  private async inspectOwned(repair = false): Promise<boolean> {
    if (!await this.prepare(repair)) return false;
    const batch = this.state.batch!; this.update({phase: "checking"});
    for (const request of batch.requests) {
      if (batch.phase === "finishing" && batch.retained.includes(request.conversationId)) continue;
      const controller = this.member(request);
      if (controller.getSnapshot().phase !== "ready") await controller.check();
      this.capture(request, controller);
    }
    return false; // Inspection never continues a deletion or completes local cleanup.
  }
  continueOriginal(): Promise<boolean> {return this.exclusive(() => this.runOwned());}
  private async runOwned(): Promise<boolean> {
    if (this.state.batch?.phase === "finishing") return this.finishOwned();
    if (!await this.prepare()) return false;
    let batch = this.state.batch!;
    if (batch.phase === "authorized") {batch = {...batch, phase: "executing"}; await this.persist(batch);}
    this.update({phase: "running"});
    for (const request of batch.requests) {
      const controller = this.member(request);
      if (controller.getSnapshot().phase !== "ready") {
        await controller.check();
        const state = controller.getSnapshot();
        if (state.phase === "committed") await controller.finish();
        else if (state.phase === "retryable" || state.phase === "rejected" && state.record?.reason === "busy") await controller.retryOriginal();
      }
      this.capture(request, controller);
      if (controller.getSnapshot().localIssue) return false;
    }
    if (batch.requests.every(request => this.state.items[request.conversationId]?.phase === "ready")) return this.finishOwned();
    return false;
  }
  finish(): Promise<boolean> {return this.exclusive(() => this.finishOwned());}
  retainUnsent(): Promise<boolean> {return this.exclusive(() => this.finishOwned(true));}
  private async finishOwned(retainUnsent = false): Promise<boolean> {
    let batch = this.state.batch; if (!batch) return false;
    if (batch.phase !== "finishing") {
      if (retainUnsent ? !canRetainUnsentBatchMembers(this.state)
        : !batch.requests.every(request => {const item = this.state.items[request.conversationId]; return item && deletionBatchMemberSettled(item);} )) return false;
      batch = {...batch, phase: "finishing", retained: batch.requests.filter(request => {
        const item = this.state.items[request.conversationId];
        return item?.phase === "rejected" || retainUnsent && (!item || item.record?.stage === "pending");
      }).map(request => request.conversationId)};
      // Once durable, no member can be retried even if cleanup/removal fails.
      await this.persist(batch);
    }
    if (!await this.prepare()) return false;
    for (const request of batch.requests) {
      if (batch.retained.includes(request.conversationId)) {
        if (!readConversationDeletion(this.storage, request.conversationId)) continue;
        const controller = this.member(request);
        if (!await controller.cancelKnown()) {this.capture(request, controller); return false;}
      } else {
        const controller = this.member(request);
        if (controller.getSnapshot().phase !== "ready") {
          await controller.check();
          if (!await controller.finish()) {this.capture(request, controller); return false;}
        }
        this.capture(request, controller);
      }
    }
    try {
      clearConversationDeletionBatch(this.storage, batch); await flushStorage(this.storage);
      this.saved = null; this.update({phase: "ready", issue: null}); return true;
    } catch (issue: unknown) {
      // Restore the revoked authorization, never an executable original batch.
      await this.persist(batch); throw issue;
    }
  }
  dispose(): void {this.disposed = true; for (const lease of this.members.values()) lease.release(); this.members.clear(); this.listeners.clear();}
}
const registries = new WeakMap<Storage, WeakMap<AgentDeletionGateway, ReviewedSubmissionOwnerRegistry<ConversationDeletionBatchController>>>();
export function acquireConversationDeletionBatchOwner(binding: AgentConversationBinding | null, internal: boolean, ports: ConversationDeletionPorts, storage: Storage) {
  let gateways = registries.get(storage); if (!gateways) {gateways = new WeakMap(); registries.set(storage, gateways);}
  let registry = gateways.get(ports.deletions);
  if (!registry) {registry = new ReviewedSubmissionOwnerRegistry(owner => owner.hasPendingOperation()); gateways.set(ports.deletions, registry);}
  return registry.acquire(deletionBatchScope(binding, internal), () => new ConversationDeletionBatchController(structuredClone(binding), internal, ports, storage));
}
export function resetConversationDeletionBatchOwnersForTests(storage: Storage, gateway: AgentDeletionGateway): void {
  registries.get(storage)?.get(gateway)?.resetForTests(); registries.get(storage)?.delete(gateway);
}
