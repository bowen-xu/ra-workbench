import { decodeAgentContextState, type AgentContextState } from "@research-assistant/core/agent";
import { DocumentVersionConflict, type HbsSessionState } from "@research-assistant/core/hbs";
import { ContextConfigurationWriteRevoked } from "@research-assistant/data/agent";
import { openContextConfigurationHbs } from "./contextConfigurationHbs";

type ConfigurationSession = Awaited<ReturnType<typeof openContextConfigurationHbs>>;
type ConfigurationOptions = Parameters<typeof openContextConfigurationHbs>[0];

/** Owns the HBS session independently of a mounted view. The host must retain
 * it while `phase` is not idle and release it only after preservation settles. */
export class ContextConfigurationOwner {
  private readonly listeners = new Set<() => void>();
  private unsubscribe: () => void;
  private operations = 0;
  private settling = false;
  private generation = 0;
  private originalPending: boolean;
  private initializing = false;
  private disposed = false;

  private constructor(private value: ConfigurationSession, originalPending: boolean, private readonly newId: () => string,
    private readonly access: { writable: boolean }, private readonly options: ConfigurationOptions) {
    this.originalPending = originalPending;
    this.unsubscribe = value.session.subscribe(() => this.onSessionChange());
    this.onSessionChange();
  }

  static async open(options: ConfigurationOptions): Promise<ContextConfigurationOwner> {
    const access = { writable: true };
    const effective = { ...options, canWrite: () => access.writable && (options.canWrite?.() ?? true) };
    const value = await openContextConfigurationHbs(effective);
    try {
      const originalPending = await value.port.inspect() === "unconfirmed";
      return new ContextConfigurationOwner(value, originalPending, options.newId, access, effective);
    } catch (reason) {
      value.session.dispose();
      throw reason;
    }
  }

  getSnapshot(): { readonly phase: "idle" | "retained"; readonly session: HbsSessionState<AgentContextState>; readonly originalPending: boolean; readonly transportFailure: unknown } {
    const session = this.value.session.getSnapshot();
    const dirty = !this.value.session.adapter.equal(session.content, session.observed.saved)
      || !this.value.session.adapter.equal(session.baselineProjection, session.observed.baseline);
    return { phase: this.operations || this.settling || this.initializing || this.originalPending || session.pending || session.queued || session.preservationError || session.conflicts.length || dirty ? "retained" : "idle",
      session, originalPending: this.originalPending, transportFailure: this.value.lastFailure() };
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** The host updates this when archive or restoration changes write access. */
  setWritable(writable: boolean): void {
    this.requireActive();
    if (this.access.writable === writable) return;
    this.access.writable = writable;
    this.notify();
  }

  /** Preserve later edits even while an earlier save awaits its receipt. */
  async edit(state: AgentContextState): Promise<void> {
    this.requireActive();
    this.requireWritable();
    this.requirePreserved();
    const decoded = decodeAgentContextState(state);
    await this.track(async () => { this.value.session.edit(decoded); await this.value.session.preserve(); });
    this.requirePreserved();
  }

  async save(): Promise<void> {
    this.requireActive();
    this.requireWritable();
    if (this.originalPending) throw new Error("CONTEXT_CONFIGURATION_ORIGINAL_PENDING");
    await this.track(() => this.value.session.save());
    this.requirePreserved();
    const state = this.value.session.getSnapshot();
    if (state.pending || state.queued || state.conflicts.length || !this.value.session.adapter.equal(state.content, state.observed.saved)
      || !this.value.session.adapter.equal(state.baselineProjection, state.observed.baseline))
      throw new Error(`CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED: ${state.message}`);
  }

  /** Reconcile authority notifications through HBS instead of replacing local
   * selections with the latest row. Transport failures remain observable. */
  async refresh(): Promise<void> {
    this.requireActive();
    this.requireWritable();
    this.requirePreserved();
    if (this.originalPending) throw new Error("CONTEXT_CONFIGURATION_ORIGINAL_PENDING");
    await this.track(async () => {
      await this.waitForSynchronization();
      this.value.session.receive(await this.value.port.read());
      await this.waitForSynchronization();
    });
    this.requirePreserved();
  }

  /** Apply an explicit HBS conflict choice, preserving it before returning. */
  async resolveConflict(id: string, action: "incoming" | "keep"): Promise<void> {
    this.requireActive(); this.requireWritable(); this.requirePreserved();
    if (!this.value.session.getSnapshot().conflicts.some(conflict => conflict.id === id))
      throw new Error("CONTEXT_CONFIGURATION_CONFLICT_CHANGED");
    await this.track(async () => {
      this.value.session.resolve(id, action);
      await this.value.session.flushPreservation();
      await this.waitForSynchronization();
    });
    this.requirePreserved();
    const state = this.value.session.getSnapshot();
    if (state.conflicts.some(conflict => conflict.id === id)) throw new Error(`CONTEXT_CONFIGURATION_CONFLICT_UNRESOLVED: ${state.message}`);
  }

  /** Repair local outbox and S′ independently. If loading S′ originally
   * failed, reopen only a clean session so no later memory input is replaced. */
  async repairLocal(): Promise<void> {
    this.requireActive();
    if (this.operations) throw new Error("CONTEXT_CONFIGURATION_RECOVERY_BUSY");
    await this.track(async () => {
      await this.value.outbox.repair();
      await this.value.preservation.repair();
      await this.value.session.preserve();
      if (!this.value.session.getSnapshot().preservationError) return;
      const previous = this.value.session.getSnapshot(), adapter = this.value.session.adapter;
      if (previous.pending || previous.queued || previous.conflicts.length
        || !adapter.equal(previous.content, previous.observed.saved)
        || !adapter.equal(previous.baselineProjection, previous.observed.baseline))
        throw new Error("CONTEXT_CONFIGURATION_PRESERVATION_REOPEN_BLOCKED");
      const replacement = await openContextConfigurationHbs(this.options);
      let originalPending: boolean;
      try {
        const error = replacement.session.getSnapshot().preservationError;
        if (error) throw new Error(error);
        originalPending = await replacement.port.inspect() === "unconfirmed";
      } catch (reason) { replacement.session.dispose(); throw reason; }
      this.unsubscribe();
      this.value.session.dispose();
      this.value = replacement;
      this.originalPending = originalPending;
      this.unsubscribe = replacement.session.subscribe(() => this.onSessionChange());
      this.onSessionChange();
    });
    this.requirePreserved();
  }

  /** A missing row is distinct from a saved empty selection. Create it with
   * the same proven transport, even when HBS sees no document delta. */
  async initialize(initial: AgentContextState, canWrite: () => boolean = () => true): Promise<"created" | "existing"> {
    this.requireActive();
    this.requireWritable();
    this.requirePreserved();
    const value = decodeAgentContextState(initial);
    const state = this.value.session.getSnapshot();
    if (this.initializing || this.operations || this.originalPending || state.pending || state.queued || state.conflicts.length)
      throw new Error("CONTEXT_CONFIGURATION_INITIALIZATION_BLOCKED");
    if (state.observed.version !== 0) return "existing";
    this.initializing = true;
    this.originalPending = true;
    try {
      return await this.track(async () => {
        let result: "created" | "existing" = "created";
        try {
          const receipt = await this.value.port.commitGuarded({ id: this.newId(), expectedVersion: 0, saved: value, baseline: value }, canWrite);
          this.originalPending = false;
          this.value.session.receive(receipt, true);
        } catch (reason) {
          if (reason instanceof ContextConfigurationWriteRevoked) {
            this.originalPending = reason.intentPersisted;
            throw reason;
          } else if (reason instanceof DocumentVersionConflict) {
            this.originalPending = false;
            result = "existing";
            this.value.session.receive(reason.snapshot, true);
          } else {
            // Only an exact committed receipt settles an ambiguous request.
            // An empty outbox alone cannot rule out another owner's delivery.
            let status: "empty" | "committed" | "unconfirmed";
            try { status = await this.value.port.inspect(); } catch { throw reason; }
            if (status !== "committed") throw reason;
            this.originalPending = false;
            this.value.session.receive(await this.value.port.read(), true);
          }
        }
        await this.waitForSynchronization();
        return result;
      });
    } finally { this.initializing = false; this.notify(); }
  }

  /** A remounted session has no in-memory pending request. Retry only the
   * frozen outbox request; reconcile its result before accepting a new save. */
  async retryOriginal(canWrite: () => boolean = () => true): Promise<void> {
    this.requireActive();
    this.requirePreserved();
    if (!this.originalPending) throw new Error("CONTEXT_CONFIGURATION_ORIGINAL_MISSING");
    await this.track(async () => {
      try { await this.value.port.retryGuarded(canWrite); }
      catch (reason) {
        if (!(reason instanceof DocumentVersionConflict)) throw reason;
      }
      this.originalPending = false;
      this.value.session.receive(await this.value.port.read(), true);
      await this.waitForSynchronization();
    });
    this.requirePreserved();
  }

  /** A live session retries its own exact pending request through HBS. */
  async retrySave(): Promise<void> {
    this.requireActive();
    if (this.originalPending) return this.retryOriginal();
    await this.save();
  }

  dispose(): void {
    if (this.disposed) return;
    if (this.getSnapshot().phase !== "idle") throw new Error("CONTEXT_CONFIGURATION_OWNER_RETAINED");
    this.disposed = true;
    this.unsubscribe();
    this.value.session.dispose();
    this.listeners.clear();
  }

  private async track<T>(operation: () => Promise<T>): Promise<T> {
    this.operations += 1;
    this.notify();
    try { return await operation(); }
    finally { await this.value.session.flushPreservation(); this.operations -= 1; this.notify(); }
  }

  private requirePreserved(): void {
    const error = this.value.session.getSnapshot().preservationError;
    if (error) throw new Error(error);
  }

  private requireActive(): void {
    if (this.disposed) throw new Error("CONTEXT_CONFIGURATION_OWNER_DISPOSED");
  }

  private requireWritable(): void {
    if (!this.access.writable) throw new Error("AGENT_CONTEXT_READ_ONLY");
  }

  private onSessionChange(): void {
    this.generation += 1;
    if (!this.settling) {
      this.settling = true;
      // HBS may enqueue a checkpoint immediately after publishing an ACK.
      // Defer the flush until that synchronous notification has returned.
      void Promise.resolve().then(async () => {
        let generation: number;
        do { generation = this.generation; await this.value.session.flushPreservation(); }
        while (generation !== this.generation);
        this.settling = false;
        this.notify();
      });
    }
    this.notify();
  }

  private async waitForSynchronization(): Promise<void> {
    const session = this.value.session;
    await new Promise<void>((resolve, reject) => {
      let unsubscribe: () => void = () => undefined;
      const check = () => {
        const state = session.getSnapshot();
        if (state.preservationError || state.pending && state.message.startsWith("MODEL_RESULT_UNKNOWN")) {
          unsubscribe(); reject(new Error(state.preservationError ?? state.message));
        } else if (!state.pending) { unsubscribe(); resolve(); }
      };
      unsubscribe = session.subscribe(check);
      check();
    });
  }

  private notify(): void { this.listeners.forEach(listener => listener()); }
}
