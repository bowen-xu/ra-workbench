import { decodeAgentForkRequest, decodeAgentForkResponse, decodeAgentForkStatus, decodeAgentRenameResponse,
  type AgentConversationSummary, type AgentForkProof, type AgentForkStatus } from "@research-assistant/core/agent";
import type { AgentForkGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { ReviewedSubmissionOwnerRegistry } from "./reviewedSubmissionOwner";
import { flushStorage, storagePersistence } from "../../runtime/storagePersistence";
import { assertForkStorageCurrent, browserConversationForkLock, forkLockName, type ConversationForkLock } from "./conversationForkLock";
import { announceConversationForkRecovery, clearConversationFork, forkScope, readConversationFork, saveConversationFork, sameForkIntent,
  type ConversationForkRecord, type ForkScope } from "./conversationForkPending";

export interface ConversationForkPorts { readonly fork: AgentForkGateway; readonly metadata: Pick<AgentMetadataGateway, "read">; readonly lock?: ConversationForkLock }
export interface ConversationForkSnapshot {
  readonly phase: "idle" | "locking" | "saving" | "unknown" | "checking" | "notCommitted" | "committed" | "reading" | "readError" | "ready" | "finishing";
  readonly record: ConversationForkRecord | null; readonly proof: AgentForkProof | null;
  readonly status: AgentForkStatus | null; readonly current: AgentConversationSummary | null;
  readonly cause: unknown; readonly localIssue: unknown;
}
const initial = (): ConversationForkSnapshot => ({ phase: "idle", record: null, proof: null, status: null, current: null, cause: null, localIssue: null });
const busy = (state: ConversationForkSnapshot) => ["locking", "saving", "checking", "reading", "finishing"].includes(state.phase);
function localFailure(cause: unknown): Error {
  return cause instanceof Error && /^FORK_RECOVERY_/u.test(cause.message) ? cause : new Error("FORK_RECOVERY_UNAVAILABLE");
}
/** The retained operation owns IPC; views cannot replace its frozen request or
 * turn an uncertain result into a new child. Reopening never mutates on mount. */
export class ConversationForkController {
  private state = initial(); private disposed = false; private operating = false;
  private readonly listeners = new Set<() => void>();
  constructor(private readonly scope: ForkScope, private readonly ports: ConversationForkPorts, private readonly storage: Storage) {
    try {
      const record = readConversationFork(storage, scope);
      if (record) this.state = { ...initial(), record, phase: "unknown" };
    } catch (cause: unknown) { this.state = { ...initial(), phase: "unknown", localIssue: localFailure(cause) }; }
  }
  getSnapshot = (): ConversationForkSnapshot => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<ConversationForkSnapshot>): void { this.state = { ...this.state, ...patch }; for (const listener of this.listeners) listener(); }
  hasPendingOperation(): boolean { return this.operating || busy(this.state); }
  private async exclusive<T>(operation: () => Promise<T>, empty: T, retryPersistence = false): Promise<T> {
    if (this.disposed || this.operating || busy(this.state)) return empty;
    this.operating = true;
    const phase = this.state.phase;
    this.update({phase: "locking"});
    try {
      return await (this.ports.lock ?? browserConversationForkLock).run(forkLockName(this.storage, this.scope), async () => {
        if (retryPersistence) await storagePersistence(this.storage)?.retry();
        await assertForkStorageCurrent(this.storage);
        if (this.disposed) return empty;
        const issue = this.state.localIssue;
        const cleared = issue instanceof Error && ["FORK_RECOVERY_LOCK_BUSY", "FORK_RECOVERY_LOCK_UNAVAILABLE"].includes(issue.message);
        this.update({phase, ...(cleared ? {localIssue: null} : {})}); return operation();
      });
    } catch (cause: unknown) { this.update({phase, localIssue: localFailure(cause)}); return empty; }
    finally { this.operating = false; this.update({}); announceConversationForkRecovery(this.storage); }
  }
  submit(): Promise<AgentConversationSummary | null> { return this.exclusive(() => this.submitOwned(), null); }
  check(): Promise<void> { return this.exclusive(() => this.checkOwned(), undefined); }
  retryOriginal(): Promise<AgentConversationSummary | null> { return this.exclusive(() => this.retryOwned(), null); }
  readCurrent(): Promise<AgentConversationSummary | null> { return this.exclusive(() => this.readCurrentOwned(), null); }
  retryLocal(): Promise<void> { return this.exclusive(() => this.retryLocalOwned(), undefined, true); }
  finish(adopt?: (current: AgentConversationSummary) => void): Promise<boolean> { return this.exclusive(() => this.finishOwned(adopt), false); }
  private async persist(record: ConversationForkRecord, first = false): Promise<boolean> {
    try {
      saveConversationFork(this.storage, record, first); this.update({ record });
      await flushStorage(this.storage);
      const saved = readConversationFork(this.storage, record.intent);
      if (!saved || !sameForkIntent(saved.intent, record.intent) || saved.epoch !== record.epoch || saved.stage !== record.stage)
        throw new Error("FORK_RECOVERY_CONFLICT");
      this.update({ localIssue: null }); return true;
    }
    catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return false; }
  }
  private async submitOwned(): Promise<AgentConversationSummary | null> {
    if (this.disposed || this.state.phase !== "idle") return null;
    try {
      const previous = readConversationFork(this.storage, this.scope);
      if (previous) { this.update({ record: previous, phase: "unknown" }); return null; }
      const intent = decodeAgentForkRequest({ ...this.scope, childId: crypto.randomUUID() });
      this.update({ phase: "saving" });
      if (!await this.persist({ schemaVersion: 1, intent, epoch: null, stage: "pending" }, true)) {
        this.update({ phase: this.state.record ? "unknown" : "idle" }); return null;
      }
    } catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return null; }
    // First read freezes the environment before any write IPC.
    this.update({ phase: "unknown" }); await this.checkOwned();
    return this.retryOwned();
  }
  private async checkOwned(): Promise<void> {
    const record = this.state.record;
    if (this.disposed || !record || busy(this.state)) return;
    this.update({ phase: "checking", cause: null });
    try {
      const result = decodeAgentForkStatus(await this.ports.fork.status(structuredClone(record.intent)));
      if (result.operationId !== record.intent.childId) throw new Error("FORK_STATUS_MISMATCH");
      const status = result.status, epoch = status.type === "committed" ? status.proof.epoch : status.epoch;
      if (record.epoch !== null && record.epoch !== epoch) throw new Error("FORK_RECOVERY_EPOCH_CHANGED");
      // A prior proof must never be downgraded into permission to publish again.
      if (record.stage === "committed" && status.type !== "committed") throw new Error("FORK_RECOVERY_PROOF_MISSING");
      const next = { ...record, epoch, stage: status.type === "committed" ? "committed" as const : record.stage };
      const saved = await this.persist(next);
      this.update({ status, proof: status.type === "committed" ? status.proof : null,
        phase: saved ? status.type === "committed" ? "committed" : status.type === "noSqlReceipt" ? "notCommitted" : "unknown" : "unknown" });
    } catch (cause: unknown) { this.update({ phase: "unknown", cause }); }
  }
  private async retryOwned(): Promise<AgentConversationSummary | null> {
    const record = this.state.record;
    if (this.disposed || !record || record.epoch === null || this.state.phase !== "notCommitted" || this.state.localIssue
      || this.state.status?.type !== "noSqlReceipt" || this.state.status.closed) return null;
    this.update({ phase: "saving", cause: null });
    if (!await this.persist({ ...record, stage: "unknown" })) { this.update({ phase: "unknown" }); return null; }
    try {
      const result = decodeAgentForkResponse(await this.ports.fork.fork(structuredClone(record.intent), {expectedEpoch: record.epoch}));
      if (result.conversation.id !== record.intent.childId || result.parentId !== record.intent.parentId
        || result.throughTurnId !== record.intent.throughTurnId) throw new Error("FORK_RESULT_MISMATCH");
    } catch (cause: unknown) { this.update({ phase: "unknown", cause }); return null; }
    this.update({ phase: "unknown" });
    await this.checkOwned();
    return this.readCurrentOwned();
  }
  private async readCurrentOwned(): Promise<AgentConversationSummary | null> {
    const record = this.state.record;
    if (this.disposed || !record || !this.state.proof || !["committed", "readError", "ready"].includes(this.state.phase)) return null;
    this.update({ phase: "reading", cause: null });
    try {
      const current = decodeAgentRenameResponse({ schemaVersion: 1,
        conversation: await this.ports.metadata.read({ id: record.intent.childId, binding: structuredClone(record.intent.binding) }) }).conversation;
      if (current.id !== record.intent.childId) throw new Error("FORK_METADATA_MISMATCH");
      this.update({ phase: "ready", current }); return current;
    } catch (cause: unknown) { this.update({ phase: "readError", cause }); return null; }
  }
  private async retryLocalOwned(): Promise<void> {
    if (this.disposed || busy(this.state)) return;
    this.update({ phase: "unknown", localIssue: null });
    await this.checkOwned();
  }
  private async finishOwned(adopt?: (current: AgentConversationSummary) => void): Promise<boolean> {
    const record = this.state.record;
    if (this.disposed || !record || !this.state.proof || busy(this.state)
      || !(this.state.phase === "ready" || this.state.phase === "readError" && this.state.cause === "notFound")) return false;
    try {
      const saved = readConversationFork(this.storage, this.scope);
      if (saved && (!sameForkIntent(saved.intent, record.intent) || saved.epoch !== record.epoch)) throw new Error("FORK_RECOVERY_CONFLICT");
      this.update({ phase: "finishing" });
      if (adopt && this.state.current) adopt(this.state.current);
      clearConversationFork(this.storage, record);
      await flushStorage(this.storage);
      this.update(initial()); return true;
    } catch (cause: unknown) {
      // A failed durable removal must remain discoverable in this storage view.
      // Do not overwrite a different intent installed by another window.
      try { saveConversationFork(this.storage, record, true); }
      catch (restoreCause: unknown) { this.update({ phase: this.state.current ? "ready" : "readError", localIssue: localFailure(restoreCause) }); return false; }
      this.update({ phase: this.state.current ? "ready" : "readError", localIssue: localFailure(cause) }); return false;
    }
  }
  dispose(): void { this.disposed = true; this.listeners.clear(); }
}
const registries = new WeakMap<Storage, ReviewedSubmissionOwnerRegistry<ConversationForkController>>();
export function conversationForkInProgress(scope: ForkScope, storage: Storage): boolean {
  return registries.get(storage)?.peek(forkScope(scope))?.hasPendingOperation() ?? false;
}
export function acquireConversationForkOwner(scope: ForkScope, ports: ConversationForkPorts, storage: Storage) {
  let registry = registries.get(storage);
  if (!registry) { registry = new ReviewedSubmissionOwnerRegistry(value => value.hasPendingOperation()); registries.set(storage, registry); }
  return registry.acquire(forkScope(scope), () => new ConversationForkController(scope, ports, storage));
}
export function resetConversationForkOwnersForTests(storage: Storage): void { registries.get(storage)?.resetForTests(); registries.delete(storage); }
