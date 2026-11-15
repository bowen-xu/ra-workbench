import { decodeAgentContextState, type AgentContextState } from "@research-assistant/core/agent";
import type { ContextConfigurationOwner } from "./contextConfigurationOwner";
import type { ContextConfigurationOwners } from "./contextConfigurationOwners";
import type { ContextConfigurationStorageOutbox } from "./contextConfigurationOutbox";
import type { ContextConfigurationSeedOwner } from "./contextConfigurationSeedOwner";

type Lease = { readonly owner: ContextConfigurationOwner; readonly release: () => void };
type OpenResult = { readonly type: "seedPending" } | { readonly type: "ready"; readonly lease: Lease; readonly initializationError?: unknown };

/** Writable startup only. An archived view must use the read-only path. */
export class ContextConfigurationStartup {
  private gate: Promise<"ready" | "seedPending"> | undefined;
  private seedResolved = false;
  private readonly initializations = new WeakMap<ContextConfigurationOwner, Promise<void>>();
  private readonly initial: AgentContextState;

  constructor(private readonly id: string, initial: AgentContextState, private readonly seedInternal: boolean,
    private readonly outbox: ContextConfigurationStorageOutbox, private readonly seed: ContextConfigurationSeedOwner,
    private readonly owners: ContextConfigurationOwners) {
    this.initial = decodeAgentContextState(initial);
  }

  async open(canWrite: () => boolean = () => true): Promise<OpenResult> {
    if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
    if (await this.prepareSeed(canWrite) === "seedPending") return { type: "seedPending" };
    if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
    const lease = await this.owners.acquire(this.id);
    try {
      if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
      const state = lease.owner.getSnapshot();
      if (!state.originalPending && !state.session.preservationError && !state.session.conflicts.length
        && state.session.observed.version === 0) await this.initialize(lease.owner, canWrite);
      if (!canWrite()) throw new Error("CONTEXT_CONFIGURATION_WRITE_REVOKED");
      return { type: "ready", lease };
    } catch (reason) {
      const state = lease.owner.getSnapshot();
      if (canWrite() && (state.originalPending || state.session.pending || state.session.preservationError || state.session.conflicts.length))
        return { type: "ready", lease, initializationError: reason };
      lease.release(); throw reason;
    }
  }

  /** No new seed is generated while the original request remains unknown. */
  async retrySeed(canWrite: () => boolean = () => true): Promise<void> {
    await this.seed.retry(canWrite);
    this.seedResolved = true;
  }

  /** Repair a failed local intent write before opening a configuration owner. */
  async repairLocal(): Promise<void> {
    await this.outbox.repair();
  }

  private prepareSeed(canWrite: () => boolean): Promise<"ready" | "seedPending"> {
    if (this.gate) return this.gate;
    const running = this.prepareSeedOnce(canWrite);
    this.gate = running;
    void running.then(() => { if (this.gate === running) this.gate = undefined; },
      () => { if (this.gate === running) this.gate = undefined; });
    return running;
  }

  private async prepareSeedOnce(canWrite: () => boolean): Promise<"ready" | "seedPending"> {
    const kind = await this.outbox.run(() => this.outbox.kind());
    if (kind === "save") return "ready";
    if (kind === "seed") {
      const status = await this.seed.inspect();
      if (status !== "committed") return "seedPending";
      this.seedResolved = true;
    }
    if (this.seedInternal && !this.seedResolved) {
      await this.seed.start(canWrite);
      this.seedResolved = true;
    }
    return "ready";
  }

  private async initialize(owner: ContextConfigurationOwner, canWrite: () => boolean): Promise<void> {
    let task = this.initializations.get(owner);
    if (!task) {
      task = owner.initialize(this.initial, canWrite).then(() => undefined);
      this.initializations.set(owner, task);
      void task.then(() => { if (this.initializations.get(owner) === task) this.initializations.delete(owner); },
        () => { if (this.initializations.get(owner) === task) this.initializations.delete(owner); });
    }
    await task;
  }
}
