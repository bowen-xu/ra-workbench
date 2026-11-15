import {decodeAgentWritingQueuedStartRequest, decodeAgentWritingQueuedStartStatusResponse, decodeAgentWritingStartResponse,
  decodeAgentWritingQueueEditRequest, sameAgentWritingQueuedStartRequest, type AgentWritingQueuedStartRequest,
  type AgentWritingQueuedStartStatusResponse, type AgentWritingStartResponse, type AgentSubmissionPreview,
  type AgentWritingQueueEditRequest, type AgentWritingQueueScope} from "@research-assistant/core/agent";
import type {AgentInputQueuePreviewGateway} from "@research-assistant/data/agent";
import {flushStorage, storagePersistence} from "../../runtime/storagePersistence";
import {isAgentStartRejection} from "./agentPreparationFailure";
import type {WritingQueuedStartQueuePort} from "./writingQueuedStartQueue";
import {decodeWritingDraftAcknowledgment, type WritingDraftAcknowledgmentPort} from "./writingDraftAcknowledgment";
import type {ConversationDraftAcknowledgmentToken} from "./conversationDraftStore";
import {WritingDispatchAdmissionClosed} from "./writingDispatchAdmission";

export interface WritingQueuedStartSnapshot {
  readonly phase: "idle" | "saving" | "sending" | "unknown" | "checking" | "prepared" | "admitted" | "rejected" | "notFound" | "unconfirmed" | "clearing" | "localError" | "done";
  readonly intent: AgentWritingQueuedStartRequest | null;
  readonly turnId: string | null;
  readonly status: AgentWritingQueuedStartStatusResponse["status"] | null;
  readonly result: AgentWritingStartResponse | null;
  readonly preview: AgentSubmissionPreview | null;
  readonly priorUnknown: boolean;
  readonly knownRejected: boolean;
  readonly recovered: boolean;
  readonly resumeCommand: AgentWritingQueueEditRequest | null;
  readonly resumeAcknowledged: boolean;
  readonly queuePaused: boolean;
  readonly cause: unknown;
  readonly draftAcknowledgment: ConversationDraftAcknowledgmentToken | null;
}
const initial = (): WritingQueuedStartSnapshot => ({phase: "idle", intent: null, turnId: null, status: null, result: null, preview: null,
  priorUnknown: false, knownRejected: false, recovered: false, resumeCommand: null, resumeAcknowledged: false, queuePaused: false, draftAcknowledgment: null, cause: null});
const scope = (intent: AgentWritingQueuedStartRequest): AgentWritingQueueScope => ({conversationId: intent.start.conversationId, manuscriptId: intent.start.manuscriptId});
const key = (value: AgentWritingQueueScope) => `research-assistant:writing-queue-start:v1:${value.manuscriptId.toLowerCase()}:${value.conversationId.toLowerCase()}`;
const sameResume = (a: AgentWritingQueueEditRequest, b: AgentWritingQueueEditRequest) => JSON.stringify(decodeAgentWritingQueueEditRequest(a)) === JSON.stringify(decodeAgentWritingQueueEditRequest(b));

/** One durable substate of the existing submission owner. Native admission and
 * history remain authoritative; this envelope is not a mode or SQL proof. */
export class WritingQueuedStart {
  private state = initial();
  private disposed = false;
  private reserved = false;
  private clearing = false;
  private replacing: AgentWritingQueuedStartRequest | null = null;
  private pending: Promise<void> | null = null;
  private drafts: WritingDraftAcknowledgmentPort | undefined;
  private draftConsumed = false;
  private delivered = false;
  private mayHaveDispatched = false;
  private freshAdmissionRequired = false;
  private retiringUnstarted = false;
  constructor(private readonly gateway: AgentInputQueuePreviewGateway, private readonly storage: Storage,
    private readonly changed: (state: WritingQueuedStartSnapshot) => void,
    private readonly observed: (intent: AgentWritingQueuedStartRequest, result: AgentWritingStartResponse) => void | Promise<void>,
    private readonly cleanupRelated: (intent: AgentWritingQueuedStartRequest) => Promise<void>, ownScope?: AgentWritingQueueScope,
    private readonly isAdmissionDisabled: () => boolean = () => false) {
    if (ownScope) this.restore(ownScope);
  }
  getSnapshot = () => this.state;
  configureDraftAcknowledgments(port?: WritingDraftAcknowledgmentPort): void {this.drafts = port;}
  unresolved = () => this.state.phase !== "idle" && this.state.phase !== "done";
  private update(patch: Partial<WritingQueuedStartSnapshot>): void {if (!this.disposed) {this.state = {...this.state, ...patch}; this.changed(this.state);}}
  private read(ownScope: AgentWritingQueueScope) {
    const raw = this.storage.getItem(key(ownScope)); if (raw === null) return null;
    try {
      const row: unknown = JSON.parse(raw);
      if (!row || typeof row !== "object" || Object.keys(row).some(key => !["schemaVersion", "intent", "turnId", "knownRejected", "result", "resumeCommand", "resumeAcknowledged", "draftAcknowledgment"].includes(key))
        || !("schemaVersion" in row) || (row.schemaVersion !== 1 && row.schemaVersion !== 2) || !("intent" in row) || !("turnId" in row)
        || (row.schemaVersion === 2 && !("draftAcknowledgment" in row)) || (row.schemaVersion === 1 && "draftAcknowledgment" in row && row.draftAcknowledgment !== null)
        || !("knownRejected" in row) || typeof row.knownRejected !== "boolean") throw new Error();
      const intent = decodeAgentWritingQueuedStartRequest(row.intent);
      if (intent.start.conversationId !== ownScope.conversationId.toLowerCase() || intent.start.manuscriptId !== ownScope.manuscriptId.toLowerCase()) throw new Error();
      const turnId = decodeAgentWritingQueuedStartRequest({...intent, inputId: row.turnId}).inputId;
      const result = "result" in row && row.result !== null ? decodeAgentWritingStartResponse(row.result) : null;
      if (result && (result.conversationId !== intent.start.conversationId || result.requestId !== intent.start.requestId || result.turnId !== turnId)) throw new Error();
      const resumeCommand = "resumeCommand" in row && row.resumeCommand !== null ? decodeAgentWritingQueueEditRequest(row.resumeCommand) : null;
      if (resumeCommand && (resumeCommand.edit.type !== "resume" || resumeCommand.conversationId !== intent.start.conversationId || resumeCommand.manuscriptId !== intent.start.manuscriptId)) throw new Error();
      if ("resumeAcknowledged" in row && typeof row.resumeAcknowledged !== "boolean") throw new Error();
      return {intent, turnId, result, knownRejected: row.knownRejected, resumeCommand, resumeAcknowledged: "resumeAcknowledged" in row && row.resumeAcknowledged === true,
        draftAcknowledgment: decodeWritingDraftAcknowledgment(row.schemaVersion === 2 && "draftAcknowledgment" in row ? row.draftAcknowledgment : null, scope(intent))};
    } catch {throw new Error("QUEUE_START_RECOVERY_INVALID");}
  }
  restore(ownScope: AgentWritingQueueScope): boolean {
    if (this.disposed || this.unresolved()) return true;
    try {const saved = this.read(ownScope); if (saved) {this.reserved = true; this.draftConsumed = false; this.delivered = false; this.mayHaveDispatched = true; this.freshAdmissionRequired = false;
      this.update({...initial(), ...saved, phase: "unknown", priorUnknown: !saved.knownRejected, recovered: true});}}
    catch (cause: unknown) {this.update({...initial(), phase: "localError", recovered: true, cause});}
    return this.unresolved();
  }
  private async persist(): Promise<void> {
    const intent = this.state.intent; if (!intent || !this.state.turnId) throw new Error("QUEUE_START_RECOVERY_INVALID");
    const saved = this.read(scope(intent));
    const replacement = this.replacing && saved?.knownRejected && sameAgentWritingQueuedStartRequest(saved.intent, this.replacing);
    if ((this.reserved && !saved) || (saved && ((!sameAgentWritingQueuedStartRequest(saved.intent, intent) && !replacement)
      || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)))) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
    this.storage.setItem(key(scope(intent)), JSON.stringify({schemaVersion: 2, intent, turnId: this.state.turnId, knownRejected: this.state.knownRejected, draftAcknowledgment: this.state.draftAcknowledgment,
      result: this.state.result, resumeCommand: this.state.resumeCommand, resumeAcknowledged: this.state.resumeAcknowledged}));
    this.reserved = true; await flushStorage(this.storage); this.replacing = null;
  }
  private async retained(run: () => Promise<void>): Promise<void> {
    if (this.disposed) return; if (this.pending) return this.pending;
    const pending = run(); this.pending = pending;
    try {await pending;} finally {if (this.pending === pending) this.pending = null;}
  }
  begin(value: AgentWritingQueuedStartRequest, turnId: string, draftAcknowledgment?: ConversationDraftAcknowledgmentToken | null): Promise<void> {
    return this.retained(async () => {
      if (this.unresolved()) return;
      const intent = decodeAgentWritingQueuedStartRequest(value); if (this.restore(scope(intent))) return;
      this.draftConsumed = false; this.delivered = false; this.mayHaveDispatched = false; this.freshAdmissionRequired = false;
      this.update({...initial(), phase: "saving", intent, turnId: decodeAgentWritingQueuedStartRequest({...intent, inputId: turnId}).inputId,
        draftAcknowledgment: decodeWritingDraftAcknowledgment(draftAcknowledgment, scope(intent))});
      await this.send();
    });
  }
  private async send(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed || this.state.result) return;
    this.update({phase: "saving", cause: null});
    try {await this.persist(); if (this.disposed) return;
      if (this.state.draftAcknowledgment) {if (!this.drafts) throw new Error("QUEUE_START_DRAFT_ACKNOWLEDGMENT_UNAVAILABLE"); await this.drafts.flush();}
    } catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
    if (this.disposed) return;
    const request = structuredClone(intent);
    this.update({phase: "sending"});
    if (this.disposed) return;
    if ((!this.mayHaveDispatched || this.freshAdmissionRequired || this.state.knownRejected && !this.state.priorUnknown) && this.isAdmissionDisabled()) {
      this.update({phase: "localError", cause: new WritingDispatchAdmissionClosed("start", intent.start.conversationId, intent.start.manuscriptId, intent.start.requestId)}); return;
    }
    this.mayHaveDispatched = true; this.freshAdmissionRequired = false;
    try {await this.finish(decodeAgentWritingStartResponse(await this.gateway.start(request)));}
    catch (cause: unknown) {
      if (this.disposed) return;
      const rejected = !this.state.priorUnknown && isAgentStartRejection(cause);
      this.update({phase: rejected ? "rejected" : "unknown", cause, knownRejected: rejected, priorUnknown: !rejected});
      if (rejected) try {await this.persist();} catch (local: unknown) {this.update({phase: "localError", cause: local});}
    }
  }
  private async finish(result: AgentWritingStartResponse): Promise<void> {
    const intent = this.state.intent;
    if (!intent || result.conversationId !== intent.start.conversationId || result.requestId !== intent.start.requestId || result.turnId !== this.state.turnId) throw new Error("QUEUE_START_RESULT_MISMATCH");
    if (this.disposed) return;
    this.update({result, phase: "clearing", cause: null});
    try {
      if (!this.clearing) {
        await this.persist(); if (this.disposed) return;
        if (!this.delivered) {await this.observed(intent, result); if (this.disposed) return; this.delivered = true;}
        if (this.state.draftAcknowledgment && !this.draftConsumed) {
          if (!this.drafts) throw new Error("QUEUE_START_DRAFT_ACKNOWLEDGMENT_UNAVAILABLE");
          await this.drafts.consume(this.state.draftAcknowledgment, scope(intent)); if (this.disposed) return;
          this.draftConsumed = true;
        }
        const held = this.read(scope(intent));
        if (!held || !sameAgentWritingQueuedStartRequest(held.intent, intent)
          || JSON.stringify(held.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
        await this.cleanupRelated(intent); if (this.disposed) return;
        const saved = this.read(scope(intent));
        if (!saved || !sameAgentWritingQueuedStartRequest(saved.intent, intent)
          || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
        this.storage.removeItem(key(scope(intent))); this.clearing = true;
      } else {
        const saved = this.read(scope(intent));
        if (saved && (!sameAgentWritingQueuedStartRequest(saved.intent, intent)
          || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment))) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
        if (saved) this.storage.removeItem(key(scope(intent)));
      }
      await flushStorage(this.storage);
      this.reserved = false; this.clearing = false; this.update({phase: "done", resumeCommand: null, resumeAcknowledged: false});
    } catch (cause: unknown) {this.update({phase: "localError", cause});}
  }
  private async inspect(): Promise<void> {
    const intent = this.state.intent; if (!intent || this.disposed) return;
    if (this.state.result) {await this.finish(this.state.result); return;}
    const priorCause = this.state.cause;
    this.update({phase: "checking", cause: null, recovered: true});
    try {
      const response = decodeAgentWritingQueuedStartStatusResponse(await this.gateway.startStatus(structuredClone(intent)));
      if (!sameAgentWritingQueuedStartRequest(response.intent, intent)) throw new Error("QUEUE_START_STATUS_MISMATCH");
      if (this.disposed) return;
      if (response.status.type === "started") {this.update({status: response.status}); await this.finish(response.status.result); return;}
      if ((response.status.type === "prepared" || response.status.type === "admitted") && response.status.preview.turnId !== this.state.turnId) throw new Error("QUEUE_START_RESULT_MISMATCH");
      this.update({phase: response.status.type, status: response.status,
        preview: response.status.type === "prepared" || response.status.type === "admitted" ? response.status.preview : null,
        cause: priorCause instanceof WritingDispatchAdmissionClosed ? priorCause : null});
    } catch (cause: unknown) {this.update({phase: "unknown", cause});}
  }
  check = (): Promise<void> => this.retained(() => this.inspect());
  hasUnattemptedStart(): boolean {return this.unresolved() && (!this.mayHaveDispatched || this.freshAdmissionRequired) && !this.state.result && this.state.intent !== null;}
  async retireVerifiedUnstarted(): Promise<boolean> {
    if (this.pending) {await this.pending; return this.retireVerifiedUnstarted();}
    let retired = false;
    await this.retained(async () => {
      if (!this.hasUnattemptedStart()) return;
      await this.inspect();
      const intent = this.state.intent;
      if (this.disposed || !this.hasUnattemptedStart() || !intent || this.state.status?.type !== "prepared") return;
      try {
        const saved = this.read(scope(intent));
        if (!saved && !this.retiringUnstarted || saved && (saved.result !== null || saved.resumeCommand !== null || saved.turnId !== this.state.turnId
          || !sameAgentWritingQueuedStartRequest(saved.intent, intent)
          || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment))) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
        if (saved) this.storage.removeItem(key(scope(intent)));
        this.retiringUnstarted = true;
        await flushStorage(this.storage);
        this.reserved = false; this.retiringUnstarted = false; this.update({phase: "done", cause: null}); retired = true;
      } catch (cause: unknown) {this.update({phase: "localError", cause});}
    });
    return retired;
  }
  retry(queue: WritingQueuedStartQueuePort, allowResume = false): Promise<void> {
    return this.retained(async () => {
      try {if (storagePersistence(this.storage)?.error) await storagePersistence(this.storage)!.retry();
        if (this.state.draftAcknowledgment && this.drafts) await this.drafts.retry();}
      catch (cause: unknown) {this.update({phase: "localError", cause}); return;}
      if (this.disposed) return;
      await this.inspect(); if (this.disposed || this.state.result || !this.state.intent || !(this.state.phase === "prepared" || this.state.phase === "admitted")) return;
      const original = this.state.intent;
      let freshResume = false;
      try {
        let current = await queue.read(original); if (this.disposed) return;
        if (this.state.phase === "prepared" && this.state.knownRejected && !this.state.priorUnknown) {
          if (current.preparingInputId !== original.inputId) throw new Error("QUEUE_START_CLAIM_CHANGED");
          if (this.isAdmissionDisabled())
            throw new WritingDispatchAdmissionClosed(current.revision === original.expectedRevision ? "start" : "rebase", original.start.conversationId, original.start.manuscriptId, original.start.requestId);
          this.freshAdmissionRequired = true;
          this.replacing = original; this.update({intent: {...original, expectedRevision: current.revision}, knownRejected: false});
          await this.persist(); if (this.disposed) return;
        }
        if (this.state.phase === "admitted" && (current.paused || this.state.resumeCommand)) {
          if (!allowResume) {this.update({queuePaused: current.paused, cause: new Error(this.state.resumeCommand ? "QUEUE_START_RESUME_UNCONFIRMED" : "QUEUE_START_QUEUE_PAUSED")}); return;}
          if (current.preparingInputId !== null) throw new Error("QUEUE_START_OTHER_CLAIM");
          if (this.state.resumeCommand && this.state.resumeAcknowledged) {
            this.update({resumeCommand: null, resumeAcknowledged: false}); await this.persist();
            this.update({queuePaused: current.paused, cause: current.paused ? new Error("QUEUE_START_QUEUE_PAUSED") : null});
            return;
          }
          if (!this.state.resumeCommand && this.isAdmissionDisabled())
            throw new WritingDispatchAdmissionClosed("resume", original.start.conversationId, original.start.manuscriptId, original.start.requestId);
          freshResume = this.state.resumeCommand === null;
          const resumed = await queue.resume(original, this.state.resumeCommand, async command => {
            if (this.disposed || !this.state.intent || !sameAgentWritingQueuedStartRequest(original, this.state.intent)) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
            if (this.state.resumeCommand && !sameResume(this.state.resumeCommand, command)) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
            this.update({resumeCommand: command, resumeAcknowledged: false}); await this.persist();
            if (this.disposed || !this.state.intent || !sameAgentWritingQueuedStartRequest(original, this.state.intent)) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
          }, () => !this.disposed && !!this.state.intent && sameAgentWritingQueuedStartRequest(original, this.state.intent), this.isAdmissionDisabled);
          if (this.disposed) return;
          if (resumed === "knownRejected") {
            this.update({resumeCommand: null, resumeAcknowledged: false}); await this.persist();
            this.update({cause: new Error("QUEUE_START_RESUME_REJECTED")}); return;
          }
          if (resumed !== "acknowledged") {this.update({cause: new Error("QUEUE_START_RESUME_UNCONFIRMED")}); return;}
          this.update({resumeAcknowledged: true}); await this.persist(); if (this.disposed) return;
          current = await queue.read(original); if (this.disposed) return;
          this.update({resumeCommand: null, resumeAcknowledged: false, queuePaused: current.paused}); await this.persist();
          if (current.paused) {this.update({cause: new Error("QUEUE_START_QUEUE_PAUSED")}); return;}
          await this.inspect(); if (this.disposed || this.state.result || this.state.phase !== "admitted") return;
        }
        if (current.paused && this.state.phase === "admitted") {this.update({queuePaused: true, cause: new Error("QUEUE_START_QUEUE_PAUSED")}); return;}
        this.update({queuePaused: false}); await this.send();
      } catch (cause: unknown) {
        if (freshResume && cause instanceof WritingDispatchAdmissionClosed && cause.step === "resume" && this.state.resumeCommand) {
          const command = this.state.resumeCommand;
          try {
            const saved = this.read(scope(original));
            if (!saved || !sameAgentWritingQueuedStartRequest(saved.intent, original) || saved.result
              || !saved.resumeCommand || !sameResume(saved.resumeCommand, command)
              || JSON.stringify(saved.draftAcknowledgment) !== JSON.stringify(this.state.draftAcknowledgment)) throw new Error("QUEUE_START_RECOVERY_CONFLICT");
            this.update({resumeCommand: null, resumeAcknowledged: false}); await this.persist();
          } catch (local: unknown) {this.update({phase: "localError", cause: local}); return;}
        }
        this.update({phase: "localError", cause});
      }
    });
  }
  dispose(): void {this.disposed = true;}
}
