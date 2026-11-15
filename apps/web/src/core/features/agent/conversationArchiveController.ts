import { decodeAgentArchiveProof, decodeAgentArchiveRequest, decodeAgentArchiveStatus,
  type AgentArchiveProof, type AgentArchiveRequest, type AgentArchiveStatus, type AgentConversationSummary,
} from "@research-assistant/core/agent";
import type { AgentArchiveGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { flushStorage, storagePersistence } from "../../runtime/storagePersistence";
import { ReviewedSubmissionOwnerRegistry } from "./reviewedSubmissionOwner";
import { clearConversationArchive, readConversationArchive, saveConversationArchive,
  type ConversationArchiveContext, type ConversationArchiveRecord } from "./conversationArchivePending";

export interface ConversationArchivePorts { readonly archives: AgentArchiveGateway; readonly metadata: Pick<AgentMetadataGateway, "read"> }
export interface ConversationArchiveSnapshot {
  readonly phase: "idle" | "persisting" | "localError" | "saving" | "unknown" | "checking" | "notCommitted" | "committing" | "committed" | "reading" | "readError" | "ready" | "rejected";
  readonly intent: AgentArchiveRequest | null;
  readonly context: ConversationArchiveContext | null;
  readonly proof: AgentArchiveProof | null;
  readonly status: AgentArchiveStatus | null;
  readonly current: AgentConversationSummary | null;
  readonly cause: unknown;
  readonly localIssue: unknown;
  /** A prior durable record confirms a proof was observed even if it must be re-read. */
  readonly recordedCommitted?: boolean;
}
const initial = (): ConversationArchiveSnapshot => ({ phase: "idle", intent: null, context: null, proof: null, status: null, current: null, cause: null, localIssue: null, recordedCommitted: false });
const rejected = (cause: unknown) => typeof cause === "string" && ["invalidInput", "notFound", "conflict", "busy", "tooLarge", "corrupt"].includes(cause);
function localFailure(cause: unknown): Error {
  return cause instanceof Error && ["ARCHIVE_RECOVERY_INVALID", "ARCHIVE_RECOVERY_CONFLICT"].includes(cause.message)
    ? cause : new Error("ARCHIVE_RECOVERY_UNAVAILABLE");
}
export function archiveUnresolved(state: ConversationArchiveSnapshot): boolean {
  return state.intent !== null || !["idle", "ready", "rejected"].includes(state.phase) || state.localIssue !== null;
}
/** A retained catalog owner, with the same status/read split as rename. It never
 * infers current archived state from a proof or refreshes the frozen history W. */
export class ConversationArchiveController {
  private state = initial();
  private readonly listeners = new Set<() => void>();
  private disposed = false;
  private reserved = false;
  private clearing = false;
  private pending: Promise<AgentConversationSummary | null> | null = null;
  constructor(private readonly id: string, private readonly ports: ConversationArchivePorts, private readonly storage: Storage) {
    try {
      const saved = readConversationArchive(storage, id);
      if (saved) { this.reserved = true; this.state = { ...initial(), phase: "unknown", intent: saved.intent, context: saved.context, recordedCommitted: saved.stage === "committed" }; }
    } catch (cause: unknown) { this.state = { ...initial(), phase: "unknown", localIssue: localFailure(cause) }; }
  }
  getSnapshot = (): ConversationArchiveSnapshot => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<ConversationArchiveSnapshot>): void { this.state = { ...this.state, ...patch }; for (const listener of this.listeners) listener(); }
  private record(stage: ConversationArchiveRecord["stage"]): ConversationArchiveRecord {
    if (!this.state.intent || !this.state.context) throw new Error("ARCHIVE_RECOVERY_INVALID");
    return { schemaVersion: 1, intent: this.state.intent, context: this.state.context, stage };
  }
  private async persist(stage: ConversationArchiveRecord["stage"]): Promise<void> {
    saveConversationArchive(this.storage, this.record(stage), !this.reserved);
    this.reserved = true;
    await flushStorage(this.storage);
  }
  private async clear(): Promise<void> {
    const record = this.record(this.state.proof ? "committed" : "unknown");
    clearConversationArchive(this.storage, record, this.clearing); this.clearing = true;
    await flushStorage(this.storage);
    this.reserved = false; this.clearing = false;
    this.update({ intent: null, context: null, localIssue: null });
  }
  private async retained(action: () => Promise<AgentConversationSummary | null>): Promise<AgentConversationSummary | null> {
    if (this.disposed) return null;
    if (this.pending) return this.pending;
    const operation = action(); this.pending = operation;
    try { return await operation; } finally { if (this.pending === operation) this.pending = null; }
  }
  submit(current: AgentConversationSummary, context: ConversationArchiveContext): Promise<AgentConversationSummary | null> {
    return this.retained(async () => {
      if (archiveUnresolved(this.state)) return null;
      let intent: AgentArchiveRequest;
      try {
        intent = decodeAgentArchiveRequest({ operationId: crypto.randomUUID(), id: current.id, archived: !current.archived,
          expectedWatermark: current.archived ? null : current.durableCount });
        if (intent.id !== this.id) throw new Error();
      } catch { this.update({ ...initial(), phase: "rejected", cause: "invalidInput" }); return null; }
      this.update({ ...initial(), phase: "persisting", intent, context: structuredClone(context) });
      return this.mutate();
    });
  }
  private async mutate(): Promise<AgentConversationSummary | null> {
    const intent = this.state.intent;
    if (!intent) return null;
    this.update({ phase: "persisting", cause: null, localIssue: null, status: null });
    try { await this.persist("unknown"); }
    catch (cause: unknown) { this.update({ phase: "localError", localIssue: localFailure(cause) }); return null; }
    this.update({ phase: "saving" });
    try {
      const proof = decodeAgentArchiveProof(await this.ports.archives.setArchived(structuredClone(intent)));
      if (proof.operationId !== intent.operationId || proof.conversationId !== intent.id) throw new Error("ARCHIVE_PROOF_MISMATCH");
      this.update({ phase: "committing", proof, recordedCommitted: true });
      try { await this.persist("committed"); }
      catch (cause: unknown) { this.update({ phase: "committed", localIssue: localFailure(cause) }); return null; }
    } catch (cause: unknown) {
      if (rejected(cause)) {
        this.update({ phase: "rejected", cause });
        try { await this.clear(); } catch (error: unknown) { this.update({ localIssue: localFailure(error) }); }
      } else this.update({ phase: "unknown", cause });
      return null;
    }
    return this.readCurrentValue();
  }
  async check(): Promise<void> {
    await this.retained(async () => { await this.checkValue(); return null; });
  }
  private async checkValue(): Promise<void> {
    const intent = this.state.intent;
    if (this.disposed || !intent || this.state.proof || ["persisting", "saving", "checking", "reading", "localError"].includes(this.state.phase)) return;
    this.update({ phase: "checking", cause: null, status: null });
    try {
      const response = decodeAgentArchiveStatus(await this.ports.archives.archiveStatus(structuredClone(intent)));
      if (response.operationId !== intent.operationId || response.conversationId !== intent.id) throw new Error("ARCHIVE_PROOF_MISMATCH");
      if (response.status.type === "committed") {
        this.update({ phase: "committed", proof: response.status.proof, status: response.status, recordedCommitted: true });
        try { await this.persist("committed"); }
        catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); }
      } else this.update({ phase: response.status.type === "noSqlReceipt" && !this.state.recordedCommitted ? "notCommitted" : "unknown", status: response.status });
      // Inspection is safe while blocked. No implicit metadata read or replay.
    } catch (cause: unknown) { this.update({ phase: "unknown", cause }); }
  }
  retryOriginal(): Promise<AgentConversationSummary | null> {
    return this.retained(async () => !this.state.proof && !this.state.recordedCommitted && this.state.phase === "notCommitted" && this.state.status?.type === "noSqlReceipt"
      && !this.state.status.closed && this.state.intent ? this.mutate() : null);
  }
  readCurrent(): Promise<AgentConversationSummary | null> {
    return this.retained(() => this.readCurrentValue());
  }
  private async readCurrentValue(): Promise<AgentConversationSummary | null> {
    const intent = this.state.intent, context = this.state.context;
    if (this.disposed || !intent || !context || !this.state.proof || !["committing", "committed", "readError", "ready"].includes(this.state.phase)) return null;
    this.update({ phase: "reading", cause: null });
    try {
      const current = await this.ports.metadata.read({ id: intent.id, ...(context.binding ? { binding: structuredClone(context.binding) } : {}) });
      if (current.id !== intent.id) throw new Error("ARCHIVE_METADATA_MISMATCH");
      this.update({ phase: "ready", current });
      try { await this.clear(); } catch (cause: unknown) {
        const issue = localFailure(cause); this.update({ localIssue: issue });
        if (issue.message === "ARCHIVE_RECOVERY_CONFLICT" || issue.message === "ARCHIVE_RECOVERY_INVALID") return null;
      }
      return current;
    } catch (cause: unknown) { this.update({ phase: "readError", cause }); return null; }
  }
  retryPersistence(): Promise<AgentConversationSummary | null> {
    return this.retained(async () => {
      try { if (storagePersistence(this.storage)?.error) await storagePersistence(this.storage)!.retry(); }
      catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return null; }
      if (this.state.phase === "localError") return this.mutate();
      if (this.state.phase === "ready" || this.state.phase === "rejected") {
        try { await this.clear(); return this.state.current; }
        catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return null; }
      }
      if (this.state.proof && this.state.intent) {
        try { await this.persist("committed"); this.update({ localIssue: null }); }
        catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); }
      }
      return null;
    });
  }
  dispose(): void { this.disposed = true; this.listeners.clear(); }
}

const registries = new WeakMap<Storage, WeakMap<AgentArchiveGateway, WeakMap<ConversationArchivePorts["metadata"], ReviewedSubmissionOwnerRegistry<ConversationArchiveController>>>>();
const testRegistries = new WeakMap<Storage, Set<ReviewedSubmissionOwnerRegistry<ConversationArchiveController>>>();
export function acquireConversationArchiveOwner(id: string, ports: ConversationArchivePorts, storage: Storage) {
  let archives = registries.get(storage); if (!archives) { archives = new WeakMap(); registries.set(storage, archives); }
  let readers = archives.get(ports.archives); if (!readers) { readers = new WeakMap(); archives.set(ports.archives, readers); }
  let registry = readers.get(ports.metadata);
  if (!registry) {
    registry = new ReviewedSubmissionOwnerRegistry(controller => archiveUnresolved(controller.getSnapshot())); readers.set(ports.metadata, registry);
    let owned = testRegistries.get(storage); if (!owned) { owned = new Set(); testRegistries.set(storage, owned); } owned.add(registry);
  }
  return registry.acquire(id.toLowerCase(), () => new ConversationArchiveController(id.toLowerCase(), ports, storage));
}
export function resetConversationArchiveOwnersForTests(storage: Storage): void {
  for (const registry of testRegistries.get(storage) ?? []) registry.resetForTests();
  testRegistries.delete(storage); registries.delete(storage);
}
