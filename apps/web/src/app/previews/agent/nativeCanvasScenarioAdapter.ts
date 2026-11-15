import type { NativeAgentPreview } from "../../../core/runtime/nativeAgentPreview";
import { NATIVE_PREVIEW_OWNER_CAPABILITY } from "../../../core/runtime/nativePreviewOwnerIntent";
import { flushStorage, storagePersistence } from "../../../core/runtime/storagePersistence";
import type { AgentCanvasScenario } from "../../../core/features/agent/agentCanvasScenario";
import { writingDemoMarkdownMessages } from "./WritingDemoMarkdownMessages";
import { writingDemoResponseMessages, writingDemoSingleMessage } from "./WritingDemoMessages";
import { nativeCanvasSamples, nativeCanvasScenarioSupported } from "./nativeCanvasScenarios";
import { canvasFixtureError, decodeNativeCanvasFixture, NATIVE_CANVAS_CAPABILITY, type NativeCanvasChoice,
  type NativeCanvasControl, type NativeCanvasFixture, type NativeCanvasSettings, type NativeCanvasStart } from "./nativeCanvasScenarioTypes";
import { clearNativeCanvasManifest, readNativeCanvasManifest, replaceNativeCanvasManifest, saveNativeCanvasManifest, wireCanvasScenario, type NativeCanvasManifest } from "./nativeCanvasScenarioManifest";
import { nativeCanvasScenarioInput } from "./nativeCanvasScenarioInput";

export interface NativeCanvasSnapshot {
  readonly fixture: NativeCanvasFixture | null;
  readonly choice: NativeCanvasChoice;
  readonly settings: NativeCanvasSettings | null;
  readonly busy: boolean;
  readonly issue: unknown;
  readonly needsStart: boolean;
  readonly disposing: boolean;
  readonly ownerDrained: boolean;
}
const idle = (): NativeCanvasSnapshot => ({ fixture: null, choice: "running", settings: null, busy: false, issue: null, needsStart: false, disposing: false, ownerDrained: false });
export interface NativeCanvasDocumentLifecycle {
  /** Preserve the still-mounted file owner while stopping and after draining. */
  prepareClose(): Promise<boolean>;
}
/** Fixture administration only. All Agent lifecycle comes from native history. */
export class NativeCanvasScenarioAdapter {
  private state = idle();
  private manifest: NativeCanvasManifest | null = null;
  private operation: Promise<boolean> | null = null;
  private readonly listeners = new Set<() => void>();
  private autoStartNeeded = false;
  private clearing: NativeCanvasManifest | null = null;
  constructor(private readonly preview: NativeAgentPreview, private readonly storage: Storage,
    private readonly buildInput: (fixture: NativeCanvasFixture) => Promise<NativeCanvasStart> = nativeCanvasScenarioInput,
    private readonly documents?: NativeCanvasDocumentLifecycle) {
    try {
      this.manifest = readNativeCanvasManifest(storage);
      if (this.manifest) {
        this.state = { ...idle(), choice: this.manifest.choice, settings: this.manifest.currentSettings ?? this.manifest.settings };
        if (this.manifest.disposal) this.restoreDisposal(this.manifest);
      }
    } catch (issue) { this.state = { ...idle(), issue }; }
  }
  getSnapshot = (): NativeCanvasSnapshot => this.state;
  subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private update(patch: Partial<NativeCanvasSnapshot>): void {
    this.state = { ...this.state, ...patch }; for (const listener of this.listeners) listener();
  }
  available(): boolean { return this.preview.bootstrap.supported.includes(NATIVE_CANVAS_CAPABILITY); }
  private requireAvailable(): void { if (!this.available()) throw canvasFixtureError("CANVAS_NATIVE_BACKEND_RESTART_REQUIRED"); }
  private fixtureIssue(code: string) {
    const fixture = this.state.fixture;
    return Object.assign(canvasFixtureError(code), fixture ? {fixtureId: fixture.fixtureId, conversationId: fixture.conversationId,
      requestId: fixture.startRequestId, generation: fixture.generation} : {});
  }
  private requireActive(): void {
    if (this.state.disposing || this.manifest?.disposal || this.clearing) throw this.fixtureIssue("CANVAS_FIXTURE_DISPOSING");
    if (this.manifest && this.state.fixture && this.manifest.fixtureId !== this.state.fixture.fixtureId)
      throw this.fixtureIssue("CANVAS_FIXTURE_RECOVERY_REQUIRED");
  }
  private restoreDisposal(manifest: NativeCanvasManifest): void {
    const locator = manifest.disposal;
    if (!locator) throw canvasFixtureError("CANVAS_FIXTURE_LOCATOR_INVALID");
    this.autoStartNeeded = false;
    this.update({ fixture: { schemaVersion: 1, fixtureId: locator.fixtureId, conversationId: locator.conversationId,
      manuscriptId: locator.manuscriptId, activePath: locator.activePath, startRequestId: locator.startRequestId, scenario: manifest.scenario,
      generation: this.preview.bootstrap.generation, created: false }, choice: manifest.choice,
      settings: manifest.currentSettings ?? manifest.settings, needsStart: false, disposing: true });
  }
  private async save(value: NativeCanvasManifest, replace?: NativeCanvasManifest): Promise<void> {
    if (this.clearing) throw canvasFixtureError("CANVAS_FIXTURE_CLEAR_UNCONFIRMED");
    if (replace) replaceNativeCanvasManifest(this.storage, replace, value);
    else saveNativeCanvasManifest(this.storage, value);
    // setItem is only the hydrated projection for IndexedDB. Keep this exact
    // intent in memory even if its durable receipt fails; never invent another F/R.
    this.manifest = value;
    await flushStorage(this.storage);
  }
  private async retryPersistence(): Promise<void> {
    const persistence = storagePersistence(this.storage);
    if (persistence?.error) await persistence.retry();
  }
  private run(action: () => Promise<void>): Promise<boolean> {
    if (this.operation) return this.operation;
    this.update({ busy: true, issue: null });
    const pending = (async () => {
      try { this.requireAvailable(); await action(); return true; }
      catch (issue) { this.update({ issue }); return false; }
    })();
    const completed = pending.finally(() => {
      if (this.operation === completed) this.operation = null;
      this.update({ busy: false });
    });
    this.operation = completed;
    return completed;
  }
  initialize(settings: NativeCanvasSettings): Promise<boolean> {
    return this.run(async () => {
      if (this.clearing) throw canvasFixtureError("CANVAS_FIXTURE_CLEAR_UNCONFIRMED");
      const existing = this.manifest ?? readNativeCanvasManifest(this.storage);
      if (existing) { this.manifest = existing; await this.openSaved(existing); }
      else { await this.open(await this.reserve("running", settings), true); }
    });
  }
  retrySetup(): Promise<boolean> {
    return this.run(async () => {
      if (this.clearing) throw canvasFixtureError("CANVAS_FIXTURE_CLEAR_UNCONFIRMED");
      if (this.state.disposing || this.manifest?.disposal || this.clearing) throw this.fixtureIssue("CANVAS_FIXTURE_DISPOSING");
      await this.retryPersistence();
      const saved = this.manifest ?? readNativeCanvasManifest(this.storage);
      if (!saved) throw canvasFixtureError("CANVAS_FIXTURE_LOCATOR_MISSING");
      this.manifest = saved; await this.openSaved(saved);
    });
  }
  private async openSaved(saved: NativeCanvasManifest): Promise<void> {
    try {await this.open(saved, false);}
    catch (issue) {
      if (saved.rollback && issue && typeof issue === "object" && "capacityRejected" in issue && issue.capacityRejected === true) {
        await this.save(saved.rollback, saved);
        await this.open(saved.rollback, false);
      }
      throw issue;
    }
  }
  private async reserve(choice: NativeCanvasChoice, settings: NativeCanvasSettings, replace?: NativeCanvasManifest): Promise<NativeCanvasManifest> {
    const wire = wireCanvasScenario(choice), scenario: AgentCanvasScenario = wire ? "single" : choice as AgentCanvasScenario;
    if (scenario === "internalRunning" || scenario === "resources" || (scenario === "childCreationUnknown" && !this.preview.bootstrap.supported.includes("child-creation-fault-fixture")) || (scenario === "childCleanupUnknown" && !this.preview.bootstrap.supported.includes("child-cleanup-fault-fixture"))) throw canvasFixtureError("CANVAS_SCENE_UNSUPPORTED");
    if (wire && !nativeCanvasScenarioSupported(wire, this.preview.bootstrap.supported)) throw canvasFixtureError("CANVAS_SCENE_UNSUPPORTED");
    const responseTexts = scenario === "markdown" || scenario === "longHistory" ? [...writingDemoMarkdownMessages] : scenario === "single" ? [writingDemoSingleMessage] : [...writingDemoResponseMessages];
    const {rollback: _oldRollback, ...previous} = replace ?? {};
    const next: NativeCanvasManifest = { schemaVersion: 1, fixtureId: crypto.randomUUID(), choice, scenario, settings, currentSettings: settings,
      stage: "creating", responseTexts, ...(replace ? {rollback:previous as NativeCanvasManifest} : {}), ...(wire && wire !== "compactSuccess" && wire !== "compactLegacy" ? { samples: nativeCanvasSamples(wire) } : {}) };
    await this.save(next, replace); return next;
  }
  private async open(manifest: NativeCanvasManifest, explicitNew: boolean): Promise<void> {
    this.autoStartNeeded = false;
    if (manifest.disposal) { this.restoreDisposal(manifest); return; }
    // Locating an existing owner cannot depend on a fresh local write receipt.
    // A closed native owner only restores its locator; active observation may
    // mount its files, but never starts or replenishes its execution.
    if (!explicitNew && await this.recoverOriginalLocator(manifest, undefined, true)) return;
    // Recovery also waits for any projected but not yet durable original intent.
    await this.save(manifest);
    let fixture: NativeCanvasFixture;
    try {
      fixture = decodeNativeCanvasFixture(await this.preview.invoke("agent_preview_create_scenario", { request: {
        fixtureId: manifest.fixtureId, scenario: manifest.scenario, settings: manifest.settings,
        ...(manifest.responseTexts ? { responseTexts: manifest.responseTexts } : {}),
      } }));
    } catch (issue) {
      const code = typeof issue === "string" ? issue : issue && typeof issue === "object" && "code" in issue ? issue.code : undefined;
      // This exact native rejection happens before a fixture is admitted. An
      // unknown failure may have created it and must retain its locator.
      if (code === "AGENT_PREVIEW_FIXTURE_CAPACITY") throw Object.assign(canvasFixtureError(code), {capacityRejected: true});
      // A failed local intent save must never prevent physical cancellation. If
      // that local marker was lost, recover only the original native locator.
      if (await this.recoverOriginalLocator(manifest, issue)) return;
      throw issue;
    }
    if (fixture.fixtureId !== manifest.fixtureId || fixture.scenario !== manifest.scenario || fixture.generation !== this.preview.bootstrap.generation)
      throw canvasFixtureError("CANVAS_FIXTURE_RESPONSE_MISMATCH");
    if (this.preview.bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY)) {
      if (!this.preview.setConversationOwner) throw canvasFixtureError("CANVAS_FIXTURE_OWNER_UNAVAILABLE");
      const previous = this.state.fixture;
      const sameRuntime = !explicitNew && await this.hasFixtureContinuity(fixture);
      // Dispatch snapshots each request's owner before any asynchronous work.
      // Retire only the UI binding; the original native owner remains alive.
      // A hot retry can retain its scenario only when the native owner proves
      // it is still in the same runtime. This does not reinstall wire data.
      this.preview.setConversationOwner(fixture.conversationId, explicitNew && fixture.created || sameRuntime ? fixture.fixtureId : null, fixture.generation);
      if (previous && previous.conversationId !== fixture.conversationId)
        this.preview.setConversationOwner(previous.conversationId, null, fixture.generation);
    }
    const interactive = manifest.scenario.startsWith("submission") || manifest.scenario.startsWith("metadata")
      || manifest.choice === "wire:compactSuccess" || manifest.choice === "wire:compactInvalid";
    this.update({ fixture, choice: manifest.choice, settings: manifest.currentSettings ?? manifest.settings, needsStart: !interactive && manifest.stage !== "initialized", disposing: false });
    if (manifest.stage === "creating") await this.save({ ...manifest, stage: "ready" });
    if (explicitNew && fixture.created && manifest.choice === "wire:compactSuccess"
      && this.preview.bootstrap.supported.includes("legacy-compaction-config-fixture"))
      await this.control({ type: "remoteCompaction", v2: true });
    // Recovery is read-only with respect to the wire plan. Reinstalling it could
    // replace an already consumed native response queue after a lost reply.
    if (explicitNew && fixture.created && manifest.samples) await this.control({ type: "rawSamples", samples: manifest.samples,
      ...(manifest.choice === "wire:compactInvalid" ? {compactionOnly: true} : {}) });
    this.autoStartNeeded = explicitNew && fixture.created && !interactive;
  }
  private async hasFixtureContinuity(fixture: NativeCanvasFixture): Promise<boolean> {
    try {
      const value = await this.preview.invoke("agent_preview_read_fixture_continuity", {request:{fixtureId:fixture.fixtureId,
        conversationId:fixture.conversationId, expectedGeneration:fixture.generation}});
      if (!value || typeof value !== "object" || Array.isArray(value)) return false;
      const row = value as Record<string, unknown>;
      return row.schemaVersion === 1 && row.fixtureId === fixture.fixtureId && row.conversationId === fixture.conversationId
        && row.generation === fixture.generation && this.preview.bootstrap.generation === fixture.generation
        && row.sameRuntime === true && row.disposed === false;
    } catch {return false;}
  }
  private async recoverOriginalLocator(manifest: NativeCanvasManifest, issue?: unknown, includeActive = false): Promise<boolean> {
    const code = typeof issue === "string" ? issue : issue && typeof issue === "object" && "code" in issue ? issue.code : undefined;
    if (!includeActive && code !== "AGENT_PREVIEW_SCENARIO_CANCELED") return false;
    try {
      const value = await this.preview.invoke("agent_preview_read_scenario", { request: {
        fixtureId: manifest.fixtureId, generation: this.preview.bootstrap.generation,
      } }) as { schemaVersion?: unknown; fixture?: unknown; disposed?: unknown } | null;
      if (value?.schemaVersion !== 1 || typeof value.disposed !== "boolean") return false;
      const fixture = decodeNativeCanvasFixture(value.fixture);
      if (fixture.created || fixture.fixtureId !== manifest.fixtureId || fixture.scenario !== manifest.scenario
        || fixture.generation !== this.preview.bootstrap.generation || (manifest.start && fixture.startRequestId !== manifest.start.requestId)) return false;
      if (!value.disposed) {
        if (includeActive) this.update({fixture, choice:manifest.choice, settings:manifest.currentSettings ?? manifest.settings, needsStart:false});
        return false;
      }
      const recovered = { ...manifest, disposal: { fixtureId: fixture.fixtureId, conversationId: fixture.conversationId,
        manuscriptId: fixture.manuscriptId, activePath: fixture.activePath, startRequestId: fixture.startRequestId } };
      this.manifest = recovered;
      this.restoreDisposal(recovered);
      return true;
    } catch { return false; } // Retain the original Create failure, including its diagnostics.
  }
  private async control(control: NativeCanvasControl, conversationId = this.state.fixture?.conversationId): Promise<void> {
    this.requireAvailable(); const fixture = this.state.fixture;
    if (control.type !== "dispose") this.requireActive();
    if (!fixture || !conversationId) throw canvasFixtureError("CANVAS_FIXTURE_NOT_READY");
    await this.preview.invoke("agent_preview_control_scenario", { request: { fixtureId: fixture.fixtureId, generation: fixture.generation, conversationId, control } });
  }
  setRemoteCompaction(v2: boolean): Promise<boolean> {
    return this.run(async () => { await this.control({ type: "remoteCompaction", v2 }); });
  }
  private async start(): Promise<void> {
    this.requireActive();
    const fixture = this.state.fixture, manifest = this.manifest;
    if (!fixture || !manifest) throw canvasFixtureError("CANVAS_FIXTURE_NOT_READY");
    const start = manifest.start ?? await this.buildInput(fixture);
    if (start.requestId !== fixture.startRequestId) throw canvasFixtureError("CANVAS_FIXTURE_START_MISMATCH");
    this.update({ needsStart: true });
    await this.save({ ...manifest, start, stage: "starting" });
    await this.control({ type: "start", ...start });
    this.update({ needsStart: false });
    await this.save({ ...this.manifest!, stage: "initialized" });
  }
  continueOriginal(): Promise<boolean> {
    return this.run(async () => { this.requireActive(); await this.retryPersistence(); await this.start(); });
  }
  reloadOriginalSamples(): Promise<boolean> {
    return this.run(async () => {
      this.requireActive();
      if (!this.manifest?.samples) throw canvasFixtureError("CANVAS_RAW_PLAN_MISSING");
      // An explicit reload is a new fixture plan, not an idempotent replay of
      // the original admission. SSE comments do not alter provider events.
      const nonce = crypto.randomUUID();
      const samples = this.manifest.samples.map((sample, index) => index === 0
        ? { ...sample, sse: `${sample.sse}\n: preview reload ${nonce}\n\n` } : sample);
      await this.control({ type: "rawSamples", samples,
        ...(this.manifest.choice === "wire:compactInvalid" ? {compactionOnly: true} : {}) });
    });
  }
  activateFixture(fixtureId: string): Promise<boolean> {
    if (this.state.disposing) return Promise.resolve(false);
    if (!this.autoStartNeeded || this.state.fixture?.fixtureId !== fixtureId) return Promise.resolve(true);
    return this.run(async () => { this.autoStartNeeded = false; await this.start(); });
  }
  private async disposeFixture(onOwnerDrained?: () => void): Promise<void> {
    if (!this.manifest) return;
    if (!this.state.fixture) throw canvasFixtureError("CANVAS_FIXTURE_RECOVERY_REQUIRED");
    if (!this.clearing) {
      const fixture = this.state.fixture;
      const manifest = this.manifest;
      this.autoStartNeeded = false;
      const closing = manifest.disposal ? manifest : { ...manifest, disposal: {
        fixtureId: fixture.fixtureId, conversationId: fixture.conversationId, manuscriptId: fixture.manuscriptId,
        activePath: fixture.activePath, startRequestId: fixture.startRequestId,
      } };
      this.manifest = closing;
      this.update({ disposing: true, needsStart: false });
      // Stop is independent of local storage and file-save failures. Observe all
      // results immediately, and keep the owner attached until all are confirmed.
      const results = await Promise.allSettled([
        this.control({ type: "dispose" }),
        (async () => { await this.retryPersistence(); await this.save(closing); })(),
        (async () => { if (this.documents && !await this.documents.prepareClose()) throw this.fixtureIssue("CANVAS_DOCUMENT_PRESERVATION_UNCONFIRMED"); })(),
      ]);
      for (const result of results) if (result.status === "rejected") throw result.reason;
      if (this.documents && !await this.documents.prepareClose()) throw this.fixtureIssue("CANVAS_DOCUMENT_PRESERVATION_UNCONFIRMED");
      // Native and file owners are both drained before the view detaches. Keep
      // that fact through a failed local flush: retry only locator cleanup.
      this.clearing = this.manifest;
      this.update({ ownerDrained: true });
      onOwnerDrained?.();
    }
    // A known drained owner only needs local cleanup. Do not reopen its file
    // gate or repeat Dispose when locator removal's durable receipt failed.
    await this.retryPersistence();
    clearNativeCanvasManifest(this.storage, this.clearing);
    await flushStorage(this.storage);
    this.manifest = null; this.clearing = null; this.autoStartNeeded = false;
    this.update({ fixture: null, needsStart: false, disposing: false, ownerDrained: false });
  }
  select(choice: NativeCanvasChoice, settings: NativeCanvasSettings, onOwnerDrained?: () => void): Promise<boolean> {
    return this.run(async () => {
      const wire = wireCanvasScenario(choice);
      if (choice === "internalRunning" || choice === "resources" || (choice === "childCreationUnknown" && !this.preview.bootstrap.supported.includes("child-creation-fault-fixture")) || (choice === "childCleanupUnknown" && !this.preview.bootstrap.supported.includes("child-cleanup-fault-fixture")) || (wire && !nativeCanvasScenarioSupported(wire, this.preview.bootstrap.supported)))
        throw canvasFixtureError("CANVAS_SCENE_UNSUPPORTED");
      if (this.manifest && this.state.fixture && this.manifest.fixtureId !== this.state.fixture.fixtureId)
        throw this.fixtureIssue("CANVAS_FIXTURE_RECOVERY_REQUIRED");
      if (this.preview.bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY) && !this.state.disposing && !this.manifest?.disposal && !this.clearing) {
        // Switching the visible fixture is not disposal: old runs and their
        // retained file owners must continue until their own lifecycle ends.
        const previous = this.manifest;
        const next = await this.reserve(choice, settings, previous ?? undefined);
        try {await this.open(next, true);}
        catch (issue) {
          if (previous && issue && typeof issue === "object" && "capacityRejected" in issue && issue.capacityRejected === true) {
            // Native explicitly rejected this identity; restore the previous
            // locator only after checking the exact failed local reservation.
            await this.save(previous, next);
          }
          throw issue;
        }
      } else {
        await this.disposeFixture(onOwnerDrained); await this.open(await this.reserve(choice, settings), true);
      }
    });
  }
  updateSettings(settings: NativeCanvasSettings): Promise<boolean> {
    return this.run(async () => {
      this.requireActive();
      const fixture = this.state.fixture, manifest = this.manifest;
      if (!fixture || !manifest) throw canvasFixtureError("CANVAS_FIXTURE_NOT_READY");
      await this.preview.invoke("agent_preview_update_scenario", { request: { fixtureId: fixture.fixtureId, generation: fixture.generation, settings } });
      this.update({ settings });
      await this.save({ ...manifest, currentSettings: settings });
    });
  }
  async fault(control: NativeCanvasControl, conversationId: string): Promise<void> { this.requireActive(); await this.control(control, conversationId); }
  releaseSubmission(conversationId?: string): Promise<boolean> { return this.run(() => this.control({ type: "releaseSubmission" }, conversationId)); }
  releaseMetadata(conversationId?: string): Promise<boolean> { return this.run(() => this.control({ type: "releaseMetadata" }, conversationId)); }
  reset(settings: NativeCanvasSettings, onOwnerDrained: () => void): Promise<boolean> {
    return this.run(async () => {
      const choice = this.state.choice;
      await this.disposeFixture(onOwnerDrained);
      await this.open(await this.reserve(choice, settings), true);
    });
  }
}
