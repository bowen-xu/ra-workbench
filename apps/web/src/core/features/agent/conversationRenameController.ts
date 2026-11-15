import { decodeAgentRenameProof, decodeAgentRenameRequest, decodeAgentRenameStatus,
  type AgentConversationSummary, type AgentRenameProof, type AgentRenameRequest, type AgentRenameStatus,
} from "@research-assistant/core/agent";
import type { AgentCatalogGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { ReviewedSubmissionOwnerRegistry } from "./reviewedSubmissionOwner";

export interface ConversationRenamePorts {
  readonly catalog: Pick<AgentCatalogGateway, "rename" | "renameStatus">;
  readonly metadata: Pick<AgentMetadataGateway, "read">;
}
export interface ConversationRenameSnapshot {
  readonly phase: "idle" | "saving" | "unknown" | "checking" | "notCommitted" | "committed" | "reading" | "readError" | "ready" | "rejected";
  readonly intent: AgentRenameRequest | null;
  readonly proof: AgentRenameProof | null;
  readonly status: AgentRenameStatus | null;
  readonly current: AgentConversationSummary | null;
  readonly cause: unknown;
  readonly localIssue: unknown;
}
const initial = (): ConversationRenameSnapshot => ({ phase: "idle", intent: null, proof: null, status: null, current: null, cause: null, localIssue: null });
const key = (id: string) => `research-assistant:conversation-rename:v1:${id.toLowerCase()}`;
const rejected = (cause: unknown) => typeof cause === "string" && ["invalidInput", "notFound", "conflict"].includes(cause);
function sameIntent(a: AgentRenameRequest, b: AgentRenameRequest): boolean {
  return a.operationId === b.operationId && a.id === b.id && a.expectedTitle === b.expectedTitle && a.title === b.title;
}
function localFailure(cause: unknown): Error {
  return cause instanceof Error && ["RENAME_RECOVERY_INVALID", "RENAME_RECOVERY_CONFLICT"].includes(cause.message)
    ? cause : new Error("RENAME_RECOVERY_UNAVAILABLE");
}
export function renameUnresolved(state: ConversationRenameSnapshot): boolean {
  return !["idle", "ready", "rejected"].includes(state.phase);
}

/** One catalog intent. The existing owner registry retains it across views;
 * durable SQL proof and the separately read current view are never conflated. */
export class ConversationRenameController {
  private state = initial();
  private readonly listeners = new Set<() => void>();
  private disposed = false;
  constructor(private readonly id: string, private readonly ports: ConversationRenamePorts,
    private readonly storage: Pick<Storage, "getItem" | "setItem" | "removeItem">) {
    try {
      const saved = storage.getItem(key(id));
      if (saved !== null) {
        const row: unknown = JSON.parse(saved);
        if (!row || typeof row !== "object" || !("schemaVersion" in row) || row.schemaVersion !== 1 || !("intent" in row))
          throw new Error("RENAME_RECOVERY_INVALID");
        const intent = decodeAgentRenameRequest(row.intent);
        if (intent.id !== id.toLowerCase()) throw new Error("RENAME_RECOVERY_INVALID");
        // Reopen only offers original status inspection, even if a prior view
        // had already observed a proof. Never replay a mutation on mount.
        this.state = { ...initial(), phase: "unknown", intent };
      }
    } catch { this.state = { ...initial(), phase: "unknown", cause: new Error("RENAME_RECOVERY_INVALID") }; }
  }
  getSnapshot = (): ConversationRenameSnapshot => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<ConversationRenameSnapshot>): void {
    this.state = { ...this.state, ...patch }; for (const listener of this.listeners) listener();
  }
  private persist(intent: AgentRenameRequest, stage: "unknown" | "committed"): void {
    const previous = this.saved();
    if (previous && !sameIntent(previous, intent)) throw new Error("RENAME_RECOVERY_CONFLICT");
    this.storage.setItem(key(this.id), JSON.stringify({ schemaVersion: 1, intent, stage }));
  }
  private saved(): AgentRenameRequest | null {
    const source = this.storage.getItem(key(this.id));
    if (source === null) return null;
    try {
      const row: unknown = JSON.parse(source);
      if (!row || typeof row !== "object" || !("schemaVersion" in row) || row.schemaVersion !== 1 || !("intent" in row)) throw new Error();
      const intent = decodeAgentRenameRequest(row.intent);
      if (intent.id !== this.id) throw new Error();
      return intent;
    } catch { throw new Error("RENAME_RECOVERY_INVALID"); }
  }
  private clear(intent: AgentRenameRequest): void {
    const previous = this.saved();
    if (!previous) return;
    if (!sameIntent(previous, intent)) throw new Error("RENAME_RECOVERY_CONFLICT");
    this.storage.removeItem(key(this.id));
  }
  async submit(expectedTitle: string, title: string): Promise<AgentConversationSummary | null> {
    return this.submitIntent({ operationId: crypto.randomUUID(), id: this.id, expectedTitle, title });
  }
  async submitIntent(value: AgentRenameRequest): Promise<AgentConversationSummary | null> {
    if (this.disposed || renameUnresolved(this.state)) return null;
    let intent: AgentRenameRequest;
    try { intent = decodeAgentRenameRequest(value); if (intent.id !== this.id) throw new Error(); }
    catch { this.update({ ...initial(), phase: "rejected", cause: "invalidInput" }); return null; }
    try { this.persist(intent, "unknown"); }
    catch (cause: unknown) { this.update({ ...initial(), phase: "rejected", cause: localFailure(cause) }); return null; }
    this.update({ ...initial(), intent });
    return this.mutate(intent);
  }
  private async mutate(intent: AgentRenameRequest): Promise<AgentConversationSummary | null> {
    this.update({ phase: "saving", cause: null, status: null });
    try {
      const proof = decodeAgentRenameProof(await this.ports.catalog.rename(structuredClone(intent)));
      if (proof.operationId !== intent.operationId || proof.conversationId !== intent.id) throw new Error("RENAME_PROOF_MISMATCH");
      this.update({ phase: "committed", proof });
      try { this.persist(intent, "committed"); }
      catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return null; }
    } catch (cause: unknown) {
      if (rejected(cause)) {
        let localIssue: unknown = null;
        try { this.clear(intent); } catch (error: unknown) { localIssue = localFailure(error); }
        this.update({ phase: "rejected", cause, localIssue });
      } else this.update({ phase: "unknown", cause });
      return null;
    }
    return this.readCurrent();
  }
  async check(): Promise<void> {
    const intent = this.state.intent;
    if (this.disposed || !intent || ["saving", "checking", "reading"].includes(this.state.phase)) return;
    this.update({ phase: "checking", cause: null });
    try {
      const result = decodeAgentRenameStatus(await this.ports.catalog.renameStatus(structuredClone(intent)));
      if (result.operationId !== intent.operationId || result.conversationId !== intent.id) throw new Error("RENAME_PROOF_MISMATCH");
      if (result.status.type === "committed") {
        this.update({ phase: "committed", proof: result.status.proof, status: result.status });
        try { this.persist(intent, "committed"); }
        catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); }
      } else this.update({ phase: result.status.type === "noSqlReceipt" ? "notCommitted" : "unknown", status: result.status });
      // Status is safe while blocked. It must never silently start a catalog
      // read, materialization, mutation retry or user callback.
    } catch (cause: unknown) { this.update({ phase: "unknown", cause }); }
  }
  async retryOriginal(): Promise<AgentConversationSummary | null> {
    if (this.disposed || this.state.phase !== "notCommitted" || this.state.status?.type !== "noSqlReceipt"
      || this.state.status.closed || !this.state.intent) return null;
    return this.mutate(this.state.intent);
  }
  async readCurrent(): Promise<AgentConversationSummary | null> {
    const intent = this.state.intent;
    if (this.disposed || !intent || !this.state.proof || !["committed", "readError"].includes(this.state.phase)) return null;
    this.update({ phase: "reading", cause: null });
    try {
      const current = await this.ports.metadata.read({ id: this.id });
      if (current.id !== this.id.toLowerCase()) throw new Error("RENAME_METADATA_MISMATCH");
      let localIssue: unknown = null;
      try { this.clear(intent); } catch (cause: unknown) { localIssue = localFailure(cause); }
      this.update({ phase: "ready", current, localIssue }); return current;
    } catch (cause: unknown) { this.update({ phase: "readError", cause }); return null; }
  }
  dismiss(): boolean {
    if (this.disposed || renameUnresolved(this.state)) return false;
    this.update(initial()); return true;
  }
  dispose(): void { this.disposed = true; this.listeners.clear(); }
}

const registries = new WeakMap<Storage, ReviewedSubmissionOwnerRegistry<ConversationRenameController>>();
export function acquireConversationRenameOwner(id: string, ports: ConversationRenamePorts, storage: Storage) {
  let registry = registries.get(storage);
  if (!registry) {
    registry = new ReviewedSubmissionOwnerRegistry(renameUnresolvedController);
    registries.set(storage, registry);
  }
  return registry.acquire(id.toLowerCase(), () => new ConversationRenameController(id.toLowerCase(), ports, storage));
}
function renameUnresolvedController(controller: ConversationRenameController): boolean { return renameUnresolved(controller.getSnapshot()); }
export function resetConversationRenameOwnersForTests(storage: Storage): void { registries.get(storage)?.resetForTests(); registries.delete(storage); }
