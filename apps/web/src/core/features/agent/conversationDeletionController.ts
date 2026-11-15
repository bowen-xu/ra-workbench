import { agentDeletionErrors, decodeAgentDeletionRequest, decodeAgentDeletionResponse, decodeAgentDeletionStatus,
  type AgentConversationBinding, type AgentConversationSummary, type AgentDeletionRequest, type AgentDeletionStatus } from "@research-assistant/core/agent";
import type { AgentDeletionGateway } from "@research-assistant/data/agent";
import { appStorageScopedKey } from "../../runtime/appStorage";
import { flushStorage, storagePersistence } from "../../runtime/storagePersistence";
import { ReviewedSubmissionOwnerRegistry } from "./reviewedSubmissionOwner";
import { assertConversationOperationStorageCurrent, browserConversationOperationLock, type ConversationOperationLock } from "./conversationOperationLock";
import { announceConversationDeletionRecovery, clearConversationDeletion, deletionRetentions, readConversationDeletion, saveConversationDeletion,
  sameDeletionIntent, type ConversationDeletionRecord } from "./conversationDeletionPending";
import { discoverConversationDeletionBatches, readConversationDeletionBatch, type ConversationDeletionBatch } from "./conversationDeletionBatchPending";

export interface ConversationDeletionPorts {
  readonly deletions: AgentDeletionGateway;
  /** Original draft/queue/steering owners only; no view state or focus changes. */
  readonly cleanup: (id: string) => void | Promise<void>;
  readonly lock?: ConversationOperationLock;
}
export type ConversationDeletionTarget = Pick<AgentConversationSummary, "id" | "title" | "archived" | "durableCount" | "internal">;
export interface ConversationDeletionSnapshot {
  readonly phase: "idle" | "locking" | "persisting" | "checking" | "deleting" | "unknown" | "retryable" | "retained" | "rejected" | "committed" | "finishing" | "ready";
  readonly record: ConversationDeletionRecord | null; readonly status: AgentDeletionStatus | null;
  readonly cause: unknown; readonly localIssue: unknown;
}
const initial = (): ConversationDeletionSnapshot => ({phase: "idle", record: null, status: null, cause: null, localIssue: null});
const busy = (state: ConversationDeletionSnapshot) => ["locking", "persisting", "checking", "deleting", "finishing"].includes(state.phase);
const browserLock = browserConversationOperationLock("DELETION_RECOVERY");
function localFailure(cause: unknown): Error {
  return cause instanceof Error && /^DELETION_RECOVERY_/u.test(cause.message) ? cause : new Error("DELETION_RECOVERY_UNAVAILABLE");
}
/** Retained catalog operation, not a model/task runner. Original confirmations,
 * data epoch and cleanup ownership survive view replacement and lost replies. */
export class ConversationDeletionController {
  private state = initial(); private operating = false; private disposed = false; private reserved = false;
  private saved: ConversationDeletionRecord | null = null;
  private confirmed = false;
  private readonly listeners = new Set<() => void>();
  constructor(private readonly id: string, private readonly ports: ConversationDeletionPorts, private readonly storage: Storage) {
    try {
      const record = readConversationDeletion(storage, id);
      if (record) {
        this.reserved = true;
        this.saved = record;
        this.state = {...initial(), record, phase: record.stage === "retained" ? "retained" : record.stage === "rejected" ? "rejected" : "unknown"};
      }
    } catch (cause: unknown) { this.state = {...initial(), phase: "unknown", localIssue: localFailure(cause)}; }
  }
  getSnapshot = (): ConversationDeletionSnapshot => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<ConversationDeletionSnapshot>): void { this.state = {...this.state, ...patch}; for (const listener of this.listeners) listener(); }
  hasPendingOperation(): boolean { return this.operating || busy(this.state) || this.state.record !== null && this.state.phase !== "ready"; }
  isOperating(): boolean { return this.operating || busy(this.state); }
  private async exclusive<T>(action: () => Promise<T>, empty: T, retryLocal = false): Promise<T> {
    if (this.disposed || this.operating || busy(this.state)) return empty;
    this.operating = true; const phase = this.state.phase; this.update({phase: "locking"});
    try {
      return await (this.ports.lock ?? browserLock).run(appStorageScopedKey(this.storage,
        `research-assistant:conversation-deletion-lock:v1:${encodeURIComponent(this.id)}`), async () => {
        if (retryLocal) await storagePersistence(this.storage)?.retry();
        await assertConversationOperationStorageCurrent(this.storage, "DELETION_RECOVERY");
        if (this.disposed) return empty;
        if (this.reserved && JSON.stringify(readConversationDeletion(this.storage, this.id)) !== JSON.stringify(this.saved))
          throw new Error("DELETION_RECOVERY_CONFLICT");
        const issue = this.state.localIssue;
        const lockIssue = issue instanceof Error && ["DELETION_RECOVERY_LOCK_BUSY", "DELETION_RECOVERY_LOCK_UNAVAILABLE"].includes(issue.message);
        this.update({phase, ...(lockIssue ? {localIssue: null} : {})}); return action();
      });
    } catch (cause: unknown) { this.update({phase, localIssue: localFailure(cause)}); return empty; }
    finally { this.operating = false; this.update({}); announceConversationDeletionRecovery(this.storage); }
  }
  private async persist(record: ConversationDeletionRecord): Promise<boolean> {
    this.update({record});
    try {
      saveConversationDeletion(this.storage, record, !this.reserved); this.reserved = true; this.saved = record;
      await flushStorage(this.storage);
      if (JSON.stringify(readConversationDeletion(this.storage, this.id)) !== JSON.stringify(record)) throw new Error("DELETION_RECOVERY_CONFLICT");
      this.update({localIssue: null}); return true;
    } catch (cause: unknown) { this.update({localIssue: localFailure(cause)}); return false; }
  }
  /** Only an already confirmed UI action calls submit. No allocation on mount. */
  submit(current: ConversationDeletionTarget, binding: AgentConversationBinding | null, options: {readonly force?: boolean} = {}): Promise<boolean> {
    const frozen = structuredClone({current, binding, force: options.force === true});
    return this.exclusive(async () => {
      if (this.state.phase !== "idle" || this.state.localIssue) return false;
      if (discoverConversationDeletionBatches(this.storage).some(batch => batch.requests.some(request => request.conversationId === this.id)))
        throw new Error("DELETION_RECOVERY_BATCH_RESERVED");
      const previous = readConversationDeletion(this.storage, this.id);
      if (previous) { this.reserved = true; this.saved = previous; this.update({record: previous, phase: "unknown"}); return false; }
      const intent = decodeAgentDeletionRequest({schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: frozen.current.id,
        binding: frozen.binding, expectedTitle: frozen.current.title, expectedDurableCount: frozen.current.durableCount, force: frozen.force});
      if (intent.conversationId !== this.id || !frozen.current.archived) throw new Error("DELETION_RECOVERY_INVALID");
      this.update({phase: "persisting"});
      if (!await this.persist({schemaVersion: 1, intent, internal: Boolean(frozen.current.internal), epoch: null, stage: "pending", reason: null, receipt: null})) {
        this.update({phase: "unknown"}); return false;
      }
      this.update({phase: "unknown"}); await this.checkOwned();
      if (this.getSnapshot().phase === "committed") return this.finishOwned();
      return this.writeOwned();
    }, false);
  }
  /** Local-only reservation from an already durable whole batch. The normal
   * check/retry/finish methods remain the sole member lifecycle. */
  prepareBatchMember(batch: ConversationDeletionBatch, request: AgentDeletionRequest): Promise<boolean> {
    const frozen = structuredClone({batch, request});
    return this.exclusive(async () => {
      if (frozen.request.conversationId !== this.id
        || JSON.stringify(readConversationDeletionBatch(this.storage, frozen.batch.batchId)) !== JSON.stringify(frozen.batch)
        || !frozen.batch.requests.some(item => sameDeletionIntent(item, frozen.request))
        || frozen.batch.epoch === null && frozen.request.requestId !== frozen.batch.requests[0]?.requestId) throw Error("DELETION_RECOVERY_BATCH_RESERVED");
      if (this.state.localIssue) return false;
      if (this.state.record && (!sameDeletionIntent(this.state.record.intent, frozen.request)
        || this.state.record.internal !== frozen.batch.internal || frozen.batch.epoch !== null && this.state.record.epoch !== frozen.batch.epoch)) throw Error("DELETION_RECOVERY_CONFLICT");
      if (this.state.phase === "ready") return true;
      const previous = readConversationDeletion(this.storage, this.id);
      if (previous) {
        if (!sameDeletionIntent(previous.intent, frozen.request) || previous.internal !== frozen.batch.internal
          || frozen.batch.epoch !== null && previous.epoch !== frozen.batch.epoch) throw Error("DELETION_RECOVERY_CONFLICT");
        this.reserved = true; this.saved = previous;
        if (!this.state.record) this.update({record: previous, phase: "unknown"});
        return true;
      }
      const record: ConversationDeletionRecord = {schemaVersion: 1, intent: frozen.request, internal: frozen.batch.internal,
        epoch: frozen.batch.epoch, stage: frozen.batch.phase === "authorized" ? "pending" : "unknown", reason: null, receipt: null};
      this.update({phase: "persisting"}); const saved = await this.persist(record); this.update({phase: "unknown"}); return saved;
    }, false);
  }
  check(): Promise<void> { return this.exclusive(() => this.checkOwned(), undefined); }
  retryOriginal(): Promise<boolean> { return this.exclusive(() => this.writeOwned(), false); }
  confirmForce(): Promise<boolean> {
    return this.exclusive(async () => {
      const record = this.state.record;
      if (!record || record.stage !== "retained" || record.intent.force || this.state.phase !== "retained" || this.state.localIssue) return false;
      this.update({phase: "persisting"});
      if (!await this.persist({...record, intent: {...record.intent, force: true}, stage: "pending", reason: null})) {
        this.update({phase: "unknown"}); return false;
      }
      this.update({phase: "unknown", status: null}); await this.checkOwned();
      return this.getSnapshot().phase === "committed" ? this.finishOwned() : this.writeOwned();
    }, false);
  }
  private async checkOwned(): Promise<void> {
    const record = this.state.record;
    if (!record || busy(this.state)) return;
    // A failed initial local save is repaired before even readonly IPC.
    if (!this.reserved || this.state.localIssue) {
      this.update({phase: "unknown"}); return;
    }
    this.update({phase: "checking", cause: null});
    this.confirmed = false;
    try {
      const result = decodeAgentDeletionStatus(await this.ports.deletions.status(structuredClone(record.intent)));
      if (result.operationId !== record.intent.requestId || result.conversationId !== this.id) throw new Error("DELETION_STATUS_MISMATCH");
      const status = result.status, epoch = status.type === "committed" ? status.proof.epoch : status.epoch;
      if (record.epoch !== null && record.epoch !== epoch) throw new Error("DELETION_RECOVERY_EPOCH_CHANGED");
      if (record.stage === "committed" && status.type !== "committed") throw new Error("DELETION_RECOVERY_PROOF_MISSING");
      this.confirmed = status.type === "committed";
      const receipt = status.type === "committed" ? {schemaVersion: 1 as const, requestId: record.intent.requestId, conversationId: this.id} : record.receipt;
      const next = {...record, epoch, ...(status.type === "committed" ? {stage: "committed" as const, reason: null, receipt} : {})};
      const saved = await this.persist(next);
      this.update({status, phase: !saved ? "unknown" : status.type === "committed" ? "committed"
        : record.stage === "retained" ? "retained" : record.stage === "rejected" ? "rejected" : status.type === "noSqlReceipt" && !status.closed ? "retryable" : "unknown"});
    } catch (cause: unknown) { this.update({phase: "unknown", cause}); }
  }
  private async writeOwned(): Promise<boolean> {
    const record = this.state.record;
    if (!record || record.epoch === null || this.state.localIssue || !(this.state.phase === "retryable" || this.state.phase === "rejected" && record.reason === "busy")
      || this.state.status?.type !== "noSqlReceipt" || this.state.status.closed) return false;
    if (discoverConversationDeletionBatches(this.storage).some(batch => batch.requests.some(request => sameDeletionIntent(request, record.intent))
      && (batch.phase !== "executing" || batch.epoch === null || batch.epoch !== record.epoch))) throw Error("DELETION_RECOVERY_BATCH_RESERVED");
    const knownUnsent = record.stage === "pending" || record.stage === "rejected";
    this.update({phase: "persisting", cause: null});
    if (!await this.persist({...record, stage: "unknown", reason: null})) { this.update({phase: "unknown"}); return false; }
    this.update({phase: "deleting"});
    try {
      const receipt = decodeAgentDeletionResponse(await this.ports.deletions.delete(structuredClone(record.intent), {expectedEpoch: record.epoch}));
      if (receipt.requestId !== record.intent.requestId || receipt.conversationId !== this.id) throw new Error("DELETION_RECEIPT_MISMATCH");
      this.confirmed = true;
      // An exact native ACK confirms the business receipt, including legacy receipts.
      if (!await this.persist({...record, stage: "committed", reason: null, receipt})) { this.update({phase: "committed"}); return false; }
      this.update({phase: "committed"}); return this.finishOwned();
    } catch (cause: unknown) {
      const reason = typeof cause === "string" && cause !== "unavailable" ? agentDeletionErrors.find(value => value === cause) : undefined;
      const retained = reason && !record.intent.force && deletionRetentions.some(value => value === reason);
      const stage = retained ? "retained" : reason && knownUnsent ? "rejected" : "unknown";
      await this.persist({...record, stage, reason: stage === "unknown" ? null : reason!, receipt: null});
      this.update({phase: stage, cause}); return false;
    }
  }
  finish(): Promise<boolean> { return this.exclusive(() => this.finishOwned(), false); }
  private async finishOwned(): Promise<boolean> {
    const record = this.state.record;
    if (!record || record.stage !== "committed" || !this.confirmed || this.state.phase !== "committed" || this.state.localIssue) return false;
    this.update({phase: "finishing"});
    try {
      await this.ports.cleanup(this.id); await flushStorage(this.storage);
      clearConversationDeletion(this.storage, record); this.saved = null; await flushStorage(this.storage);
      this.reserved = false; this.update({phase: "ready", localIssue: null}); return true;
    } catch (cause: unknown) {
      try { saveConversationDeletion(this.storage, record, true); this.reserved = true; this.saved = record; }
      catch (restoreCause: unknown) { this.update({phase: "committed", localIssue: localFailure(restoreCause)}); return false; }
      this.update({phase: "committed", localIssue: localFailure(cause)}); return false;
    }
  }
  retryLocal(): Promise<boolean> {
    return this.exclusive(async () => {
      const record = this.state.record; if (!record) return false;
      if (!await this.persist(record)) return false;
      this.update({phase: record.stage === "committed" ? "committed" : "unknown"});
      if (record.stage === "committed" && this.confirmed) return this.finishOwned();
      this.update({phase: "unknown"}); await this.checkOwned();
      return this.state.phase === "committed" ? this.finishOwned() : false;
    }, false, true);
  }
  cancelKnown(): Promise<boolean> {
    return this.exclusive(async () => {
      const record = this.state.record;
      if (!record || record.stage === "unknown" || record.stage === "committed") return false;
      try {
        clearConversationDeletion(this.storage, record); this.saved = null; await flushStorage(this.storage);
        this.reserved = false; this.update(initial()); return true;
      } catch (cause: unknown) {
        try { saveConversationDeletion(this.storage, record, true); this.reserved = true; this.saved = record; }
        catch (restoreCause: unknown) { this.update({localIssue: localFailure(restoreCause)}); return false; }
        this.update({localIssue: localFailure(cause)}); return false;
      }
    }, false);
  }
  dispose(): void { this.disposed = true; this.listeners.clear(); }
}
const registries = new WeakMap<Storage, WeakMap<AgentDeletionGateway, ReviewedSubmissionOwnerRegistry<ConversationDeletionController>>>();
export function conversationDeletionInProgress(id: string, gateway: AgentDeletionGateway, storage: Storage): boolean {
  return registries.get(storage)?.get(gateway)?.peek(id.toLowerCase())?.isOperating() ?? false;
}
export function acquireConversationDeletionOwner(id: string, ports: ConversationDeletionPorts, storage: Storage) {
  let gateways = registries.get(storage); if (!gateways) { gateways = new WeakMap(); registries.set(storage, gateways); }
  let registry = gateways.get(ports.deletions);
  if (!registry) { registry = new ReviewedSubmissionOwnerRegistry(value => value.hasPendingOperation()); gateways.set(ports.deletions, registry); }
  return registry.acquire(id, () => new ConversationDeletionController(id, ports, storage));
}
export function resetConversationDeletionOwnersForTests(storage: Storage, gateway: AgentDeletionGateway): void {
  registries.get(storage)?.get(gateway)?.resetForTests(); registries.get(storage)?.delete(gateway);
}
