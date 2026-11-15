import { decodeAgentCreateBoundProof, decodeAgentCreateBoundStatus, type AgentConversationBinding, type AgentConversationSummary,
  type AgentCreateBoundRequest, type AgentCreateBoundProof, type AgentCreateBoundStatus } from "@research-assistant/core/agent";
import type { AgentCreationGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { ReviewedSubmissionOwnerRegistry } from "./reviewedSubmissionOwner";
import { acknowledgeBoundCreation, boundCreationScope, persistBoundCreation, readBoundCreation, reserveBoundConversation } from "./boundConversationCreationPending";
import { sameBoundCreation } from "./boundConversationCreationPending";

export interface ConversationCreationPorts { readonly creation: AgentCreationGateway; readonly metadata: Pick<AgentMetadataGateway, "read"> }
export interface ConversationCreationSnapshot {
  readonly phase: "idle" | "saving" | "unknown" | "checking" | "notCommitted" | "committing" | "committed" | "reading" | "readError" | "ready" | "rejected";
  readonly intent: AgentCreateBoundRequest | null; readonly proof: AgentCreateBoundProof | null;
  readonly status: AgentCreateBoundStatus | null; readonly current: AgentConversationSummary | null;
  readonly cause: unknown; readonly localIssue: unknown;
  readonly adopted: boolean;
}
const initial = (): ConversationCreationSnapshot => ({ phase: "idle", intent: null, proof: null, status: null, current: null, cause: null, localIssue: null, adopted: false });
function localFailure(cause: unknown): Error {
  return cause instanceof Error && ["CREATION_RECOVERY_INVALID", "CREATION_RECOVERY_CONFLICT"].includes(cause.message)
    ? cause : new Error("CREATION_RECOVERY_UNAVAILABLE");
}
export function creationUnresolved(state: ConversationCreationSnapshot): boolean { return !!state.intent || !["idle", "rejected"].includes(state.phase); }
export function creationCanFinish(state: ConversationCreationSnapshot): boolean {
  return !!state.proof && ((state.phase === "ready" && state.current?.archived === true) || (state.phase === "readError" && state.cause === "notFound"));
}

/** Keeps the original bound creation and its current read separate. The host
 * acknowledges adoption only after its own live-scope checks and selection. */
export class ConversationCreationController {
  private state = initial(); private disposed = false;
  private pending: Promise<AgentConversationSummary | null> | null = null;
  private readonly listeners = new Set<() => void>();
  constructor(private readonly binding: AgentConversationBinding, private readonly ports: ConversationCreationPorts, private readonly storage: Storage) {
    try {
      const saved = readBoundCreation(binding, storage);
      if (saved) this.state = { ...initial(), intent: saved.intent, phase: saved.stage === "pending" ? "idle" : "unknown" };
    } catch (cause: unknown) { this.state = { ...initial(), phase: "unknown", localIssue: localFailure(cause) }; }
  }
  getSnapshot = (): ConversationCreationSnapshot => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<ConversationCreationSnapshot>): void { this.state = { ...this.state, ...patch }; for (const listener of this.listeners) listener(); }
  async submit(id?: string): Promise<AgentConversationSummary | null> {
    if (this.disposed) return null;
    if (this.pending) return this.pending;
    if (this.state.adopted) { this.update({ localIssue: new Error("CREATION_RECOVERY_CLEANUP_REQUIRED") }); return null; }
    if (this.state.phase === "ready") return this.state.current;
    if (!["idle", "rejected"].includes(this.state.phase)) return null;
    try {
      reserveBoundConversation(this.binding, id, this.storage);
      const saved = readBoundCreation(this.binding, this.storage)!;
      if (saved.stage !== "pending") { this.update({ intent: saved.intent, phase: "unknown" }); return null; }
      this.update({ ...initial(), intent: saved.intent });
    } catch (cause: unknown) { this.update({ phase: "unknown", localIssue: localFailure(cause) }); return null; }
    const promise = this.mutate(this.state.intent!); this.pending = promise;
    try { return await promise; } finally { if (this.pending === promise) this.pending = null; }
  }
  private async mutate(intent: AgentCreateBoundRequest): Promise<AgentConversationSummary | null> {
    try { persistBoundCreation(intent, "unknown", this.storage); }
    catch (cause: unknown) { this.update({ phase: "unknown", localIssue: localFailure(cause) }); return null; }
    this.update({ phase: "saving", cause: null, status: null });
    try {
      const proof = decodeAgentCreateBoundProof(await this.ports.creation.createBound(structuredClone(intent)));
      if (proof.operationId !== intent.operationId || proof.conversationId !== intent.id) throw new Error("CREATION_PROOF_MISMATCH");
      this.update({ phase: "committing", proof });
      try { persistBoundCreation(intent, "committed", this.storage); }
      catch (cause: unknown) { this.update({ phase: "committed", localIssue: localFailure(cause) }); return null; }
    } catch (cause: unknown) {
      if (typeof cause === "string" && ["invalidInput", "notFound", "conflict"].includes(cause)) {
        this.update({ phase: "rejected", cause });
        try { persistBoundCreation(intent, "pending", this.storage); }
        catch (error: unknown) { this.update({ localIssue: localFailure(error) }); }
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
      const result = decodeAgentCreateBoundStatus(await this.ports.creation.createBoundStatus(structuredClone(intent)));
      if (result.operationId !== intent.operationId || result.conversationId !== intent.id) throw new Error("CREATION_PROOF_MISMATCH");
      if (result.status.type === "committed") {
        this.update({ phase: "committed", proof: result.status.proof, status: result.status });
        try { persistBoundCreation(intent, "committed", this.storage); }
        catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); }
      } else this.update({ phase: result.status.type === "noSqlReceipt" ? "notCommitted" : "unknown", status: result.status });
    } catch (cause: unknown) { this.update({ phase: "unknown", cause }); }
  }
  async retryOriginal(): Promise<AgentConversationSummary | null> {
    if (this.disposed || this.state.phase !== "notCommitted" || this.state.status?.type !== "noSqlReceipt" || this.state.status.closed || !this.state.intent) return null;
    return this.mutate(this.state.intent);
  }
  async readCurrent(): Promise<AgentConversationSummary | null> {
    const intent = this.state.intent;
    if (this.disposed || !intent || !this.state.proof || !["committing", "committed", "readError", "ready"].includes(this.state.phase)) return null;
    this.update({ phase: "reading", cause: null });
    try {
      const current = await this.ports.metadata.read({ id: intent.id, binding: structuredClone(intent.binding) });
      if (current.id !== intent.id) throw new Error("CREATION_METADATA_MISMATCH");
      this.update({ phase: "ready", current }); return current;
    } catch (cause: unknown) { this.update({ phase: "readError", cause }); return null; }
  }
  consumeAdoption(adopt: () => boolean): boolean {
    if (this.disposed || this.state.phase !== "ready" || !this.state.intent || this.state.current?.archived) return false;
    const intent = this.state.intent;
    try {
      const saved = readBoundCreation(intent.binding, this.storage);
      if (!saved || !sameBoundCreation(saved.intent, intent)) throw new Error("CREATION_RECOVERY_CONFLICT");
    } catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return false; }
    try { if (!adopt()) return false; }
    catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return false; }
    this.update({ adopted: true });
    try { acknowledgeBoundCreation(intent, this.storage); this.update(initial()); }
    catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); }
    return true;
  }
  clearAdopted(): boolean {
    if (this.disposed || !this.state.adopted || !this.state.intent) return false;
    try { acknowledgeBoundCreation(this.state.intent, this.storage); this.update(initial()); return true; }
    catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return false; }
  }
  finishKnown(): boolean {
    if (this.disposed || !this.state.intent || !creationCanFinish(this.state)) return false;
    try { acknowledgeBoundCreation(this.state.intent, this.storage); this.update(initial()); return true; }
    catch (cause: unknown) { this.update({ localIssue: localFailure(cause) }); return false; }
  }
  dispose(): void { this.disposed = true; this.listeners.clear(); }
}
const registries = new WeakMap<Storage, ReviewedSubmissionOwnerRegistry<ConversationCreationController>>();
export function acquireConversationCreationOwner(binding: AgentConversationBinding, ports: ConversationCreationPorts, storage: Storage) {
  let registry = registries.get(storage);
  if (!registry) { registry = new ReviewedSubmissionOwnerRegistry(value => creationUnresolved(value.getSnapshot())); registries.set(storage, registry); }
  return registry.acquire(boundCreationScope(binding), () => new ConversationCreationController(binding, ports, storage));
}
export function resetConversationCreationOwnersForTests(storage: Storage): void { registries.get(storage)?.resetForTests(); registries.delete(storage); }
