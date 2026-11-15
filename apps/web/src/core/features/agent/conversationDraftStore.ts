/** UI drafts; submitted inputs and queues remain owned by the Agent service. */
import {agentDraftFromInput, agentDraftInput, editAgentDraft, trimmedAgentDraftInput, serializeAgentSourceReferences, restoreAgentSourceDraft,
  type AgentInputDraft, type AgentUserInput, type AgentEvidenceInput} from "@research-assistant/core/agent";
import type {ConversationDraftPersistence, ConversationDraftSnapshot, ConversationDraftSnapshotEntry} from "./conversationDraftPersistence";
import {decodeQueueDraftPlacement, type ConversationInputDraft, type QueueDraftPlacement} from "./queueComposerDraft";
import {releaseRemovedEvidence} from "./evidenceImageGateway";
import type {ConversationDraftStore as ConversationDraftStorePort} from "@research-assistant/workbench-sdk/agent-drafts";
const emptyDraft: AgentInputDraft = {text: "", bindings: []};
export interface ConversationDraftScope {
  readonly hostKind: "manuscript" | "paper" | "note" | "readingNote";
  readonly hostId: string;
  readonly conversationId: string | null;
}
export interface ConversationDraftAcknowledgmentToken {
  readonly schemaVersion: 1;
  readonly originKey: string;
  readonly revision: string;
  readonly sourceInputId?: string;
  readonly sourceQueueRevision?: string;
}
interface DraftVersion {
  revision: string | null;
  originKey: string | null;
  currentKey: string;
  readonly sourceQueueRevision: string | null;
}
const uuid = (value: unknown): value is string => typeof value === "string"
  && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/u.test(value) && !/^0{8}-(?:0{4}-){3}0{12}$/u.test(value);
export function conversationDraftKey(scope: ConversationDraftScope): string {return JSON.stringify([scope.hostKind, scope.hostId, scope.conversationId]);}
function readScope(key: unknown): ConversationDraftScope {
  if (typeof key !== "string") throw new Error("AGENT_DRAFT_ACKNOWLEDGMENT_INVALID");
  const value: unknown = JSON.parse(key);
  if (!Array.isArray(value) || value.length !== 3 || !["manuscript", "paper", "note", "readingNote"].includes(value[0])
    || typeof value[1] !== "string" || (value[2] !== null && typeof value[2] !== "string") || JSON.stringify(value) !== key) throw new Error("AGENT_DRAFT_ACKNOWLEDGMENT_INVALID");
  return {hostKind: value[0], hostId: value[1], conversationId: value[2]};
}
export function decodeConversationDraftAcknowledgmentToken(value: unknown): ConversationDraftAcknowledgmentToken {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("AGENT_DRAFT_ACKNOWLEDGMENT_INVALID");
  const row = value as Record<string, unknown>, queued = Object.hasOwn(row, "sourceInputId") || Object.hasOwn(row, "sourceQueueRevision");
  if (Object.keys(row).sort().join() !== (queued ? "originKey,revision,schemaVersion,sourceInputId,sourceQueueRevision" : "originKey,revision,schemaVersion")
    || row.schemaVersion !== 1 || !uuid(row.revision) || (queued && (!uuid(row.sourceInputId) || !uuid(row.sourceQueueRevision)))) throw new Error("AGENT_DRAFT_ACKNOWLEDGMENT_INVALID");
  readScope(row.originKey);
  return {schemaVersion: 1, originKey: row.originKey as string, revision: row.revision,
    ...(queued ? {sourceInputId: row.sourceInputId as string, sourceQueueRevision: row.sourceQueueRevision as string} : {})};
}

/** The existing root owner holds content and local draft provenance together. */
export class ConversationDraftStore implements ConversationDraftStorePort {
  private persistenceError = "";
  private persistenceReadable = true;
  private readonly versions = new Map<string, DraftVersion>();
  private readonly drafts = new Map<string, ConversationInputDraft>();
  private readonly listeners = new Map<string, Set<() => void>>();
  private readonly deletedSuffixes = new Set<string>();
  private savedGeneration = 0;
  private flushedGeneration = 0;
  private mutation = 0;
  private savedMutation = 0;
  private evidencePending: {readonly before: readonly AgentEvidenceInput[]; generation: number | null}[] = [];
  constructor(private readonly persistence?: ConversationDraftPersistence) {
    try {
      const snapshot = persistence?.load() ?? new Map();
      const revisions = new Set<string>(), origins = new Set<string>();
      for (const [key, entry] of snapshot) {
        const current = readScope(key);
        if (entry.revision !== null) {
          const origin = readScope(entry.originKey);
          if (!uuid(entry.revision) || revisions.has(entry.revision) || origin.hostKind !== current.hostKind || origin.hostId !== current.hostId
            || (origin.conversationId !== null && origin.conversationId !== current.conversationId)) throw new Error("Invalid draft provenance");
          revisions.add(entry.revision);
        } else if (entry.originKey !== null || entry.sourceQueueRevision !== null) throw new Error("Invalid legacy draft provenance");
        if (entry.sourceQueueRevision !== null) {
          if (!entry.draft.queuePlacement || !uuid(entry.sourceQueueRevision) || origins.has(entry.sourceQueueRevision)) throw new Error("Invalid queue provenance");
          origins.add(entry.sourceQueueRevision);
        }
      }
      for (const [key, entry] of snapshot) {
        this.drafts.set(key, entry.draft); this.versions.set(key, {revision: entry.revision, originKey: entry.originKey, sourceQueueRevision: entry.sourceQueueRevision, currentKey: key});
      }
    } catch {this.persistenceReadable = false; this.persistenceError = "无法恢复草稿；原存储保留，本次输入仅保存在内存，请检查本机存储。";}
  }
  error(): string {return this.persistenceError;}
  read(key: string): string {return this.readStructured(key).text;}
  readStructured(key: string): ConversationInputDraft {return this.drafts.get(key) ?? emptyDraft;}
  input(key: string, text: string): readonly AgentUserInput[] {
    const draft = this.readStructured(key); return draft.text.trim() === text ? trimmedAgentDraftInput(draft) : [];
  }
  private snapshot(): Map<string, ConversationDraftSnapshotEntry> {
    return new Map([...this.drafts].map(([key, draft]) => {const version = this.versions.get(key)!;
      return [key, {draft, revision: version.revision, originKey: version.originKey, sourceQueueRevision: version.sourceQueueRevision}];}));
  }
  private save(snapshot: ConversationDraftSnapshot = this.snapshot()): number {
    if (!this.persistenceReadable) throw new Error("AGENT_DRAFT_RECOVERY_UNREADABLE");
    try {
      this.persistence?.save(snapshot); const generation = ++this.savedGeneration; this.savedMutation = this.mutation;
      for (const item of this.evidencePending) if (item.generation === null) item.generation = generation;
      this.persistenceError = ""; return generation;
    } catch (cause: unknown) {this.persistenceError = "草稿暂未保存到本机；请保留输入并检查存储。"; throw new Error("AGENT_DRAFT_PERSISTENCE_FAILED", {cause});}
  }
  private async flushSaved(generation: number): Promise<void> {
    try {
      await this.persistence?.flush?.(); this.flushedGeneration = Math.max(this.flushedGeneration, generation);
      if (this.savedGeneration === generation && this.savedMutation === this.mutation) this.persistenceError = "";
      const ready = this.evidencePending.filter(item => item.generation !== null && item.generation <= generation);
      this.evidencePending = this.evidencePending.filter(item => item.generation === null || item.generation > generation);
      if (ready.length) releaseRemovedEvidence(ready.flatMap(item => item.before), [...this.drafts.values()].flatMap(draft => draft.evidence ?? []));
    } catch (cause: unknown) {
      if (generation >= this.flushedGeneration) this.persistenceError = "草稿暂未保存到本机；请保留输入并检查存储。";
      throw new Error("AGENT_DRAFT_PERSISTENCE_FAILED", {cause});
    }
  }
  async flush(): Promise<void> {await this.flushSaved(this.save());}
  async retryPersistence(): Promise<void> {
    try {await this.persistence?.retry?.();} catch (cause: unknown) {
      this.persistenceError = "草稿暂未保存到本机；请保留输入并检查存储。"; throw new Error("AGENT_DRAFT_PERSISTENCE_FAILED", {cause});
    }
    await this.flush();
  }
  private publish(key: string): void {for (const listener of this.listeners.get(key) ?? []) listener();}
  private persistEdit(): void {
    try {
      const generation = this.save();
      // A failed background flush remains observable through error() and retryPersistence().
      if (this.evidencePending.length) void this.flushSaved(generation).catch(() => undefined);
    } catch {return; /* save() retains the actionable persistence error and all pending image handles. */}
  }
  private removedEvidence(previous: ConversationInputDraft, next: ConversationInputDraft, generation: number | null = null): void {
    const retained = new Set(next.evidence?.flatMap(item => item.image ? [item.image.captureId] : []) ?? []);
    const before = previous.evidence?.filter(item => item.image && !retained.has(item.image.captureId));
    if (before?.length) this.evidencePending.push({before, generation});
  }
  private acknowledgedText(key: string, text: string): string {
    try {const scope = readScope(key), draft = this.readStructured(key);
      if (serializeAgentSourceReferences(draft, {kind: scope.hostKind, resourceId: scope.hostId, conversationId: scope.conversationId ?? ""}).text === text) return draft.text.trim();
    } catch { /* Legacy closure callers retain the old visible-text contract. */ }
    return text;
  }
  /** Fresh capture only. Loading or recovering an old operation never calls this. */
  captureAcknowledgmentToken(key: string, text: string): ConversationDraftAcknowledgmentToken | null {
    const version = this.versions.get(key); if (!version || !this.persistenceReadable || this.read(key).trim() !== this.acknowledgedText(key, text)) return null;
    readScope(key);
    if (version.revision === null) {
      const revision = crypto.randomUUID(), candidate = this.snapshot(), entry = candidate.get(key)!;
      candidate.set(key, {...entry, revision, originKey: key});
      try {this.save(candidate);} catch {return null;}
      version.revision = revision; version.originKey = key;
    }
    const sourceInputId = this.readStructured(key).queuePlacement?.input.inputId;
    return {schemaVersion: 1, originKey: key, revision: version.revision,
      ...(sourceInputId && version.sourceQueueRevision ? {sourceInputId, sourceQueueRevision: version.sourceQueueRevision} : {})};
  }
  /** Native admission is checked by the caller; this is only the local draft CAS. */
  async consumeAcknowledgment(value: ConversationDraftAcknowledgmentToken, acceptedScope: ConversationDraftScope): Promise<"cleared" | "preserved"> {
    const token = decodeConversationDraftAcknowledgmentToken(value), origin = readScope(token.originKey), key = conversationDraftKey(acceptedScope);
    readScope(key);
    if (origin.hostKind !== acceptedScope.hostKind || origin.hostId !== acceptedScope.hostId
      || (origin.conversationId !== null && origin.conversationId !== acceptedScope.conversationId)) return "preserved";
    const version = this.versions.get(key), previous = this.drafts.get(key);
    if (!version || !previous) {await this.flush(); return "cleared";}
    if (token.originKey !== key && (origin.conversationId !== null || version.originKey !== token.originKey)) return "preserved";
    const clear = version.revision === token.revision;
    const clearPlacement = !!token.sourceInputId && previous.queuePlacement?.input.inputId === token.sourceInputId
      && version.sourceQueueRevision === token.sourceQueueRevision;
    if (!clear && !clearPlacement) {await this.flush(); return "preserved";}
    const candidate = this.snapshot(), next: ConversationInputDraft = clear ? emptyDraft : {...agentDraftFromInput(previous.text, agentDraftInput(previous))};
    const retain = next.text !== "" || !!next.evidence?.length || !!next.unboundReferences?.length;
    const nextVersion: DraftVersion | null = retain ? {...version, revision: crypto.randomUUID(), sourceQueueRevision: null} : null;
    if (nextVersion) candidate.set(key, {draft: next, ...nextVersion}); else candidate.delete(key);
    const generation = this.save(candidate);
    if (this.versions.get(key) !== version) {await this.flush(); return "preserved";}
    if (nextVersion) {this.drafts.set(key, next); this.versions.set(key, nextVersion);} else {this.drafts.delete(key); this.versions.delete(key);}
    ++this.mutation; this.savedMutation = this.mutation; this.removedEvidence(previous, next, generation); this.publish(key);
    await this.flushSaved(generation); return nextVersion ? "preserved" : "cleared";
  }
  /** Same-session callbacks retain exact revision and queue-origin ownership. */
  captureAcknowledgment(key: string, text: string): () => void {
    const acknowledgedText = this.acknowledgedText(key, text), version = this.versions.get(key);
    const sourceInputId = this.readStructured(key).queuePlacement?.input.inputId, sourceQueueRevision = version?.sourceQueueRevision;
    return () => {
      if (!version) return; const currentKey = version.currentKey, current = this.versions.get(currentKey);
      if (current === version && this.read(currentKey).trim() === acknowledgedText) this.updateStructured(currentKey, emptyDraft, null);
      else if (sourceInputId && sourceQueueRevision && current?.sourceQueueRevision === sourceQueueRevision
        && this.readStructured(currentKey).queuePlacement?.input.inputId === sourceInputId) this.updateStructured(currentKey, this.readStructured(currentKey), null);
    };
  }
  adoptPending(scope: Omit<ConversationDraftScope, "conversationId">, conversationId: string): void {
    const from = conversationDraftKey({...scope, conversationId: null}), to = conversationDraftKey({...scope, conversationId});
    const value = this.drafts.get(from), version = this.versions.get(from); if (!value || !version || this.drafts.has(to)) return;
    this.drafts.set(to, value); version.currentKey = to; this.versions.set(to, version); this.drafts.delete(from); this.versions.delete(from);
    ++this.mutation; this.persistEdit(); this.publish(from); this.publish(to);
  }
  forgetConversation(id: string): void {
    const suffix = `,${JSON.stringify(id)}]`;
    for (const key of this.drafts.keys()) if (key.endsWith(suffix)) this.updateStructured(key, emptyDraft, null);
    this.deletedSuffixes.add(suffix);
  }
  update(key: string, next: string | ((current: string) => string)): void {
    if ([...this.deletedSuffixes].some(suffix => key.endsWith(suffix))) return;
    const previous = this.read(key), value = typeof next === "function" ? next(previous) : next;
    if (value !== previous) this.updateStructured(key, editAgentDraft(this.readStructured(key), value));
  }
  updateStructured(key: string, value: AgentInputDraft, placement: QueueDraftPlacement | null | undefined = this.readStructured(key).queuePlacement): void {
    this.setStructured(key, value, placement, false);
  }
  private setStructured(key: string, value: AgentInputDraft, placement: QueueDraftPlacement | null | undefined, renewedQueue: boolean): void {
    if ([...this.deletedSuffixes].some(suffix => key.endsWith(suffix))) return;
    const previous = this.readStructured(key), old = this.versions.get(key), checked = agentDraftFromInput(value.text, agentDraftInput(value));
    if (!renewedQueue && value === previous && placement === previous.queuePlacement) return;
    const next: ConversationInputDraft = {...checked, ...(placement ? {queuePlacement: decodeQueueDraftPlacement(placement)} : {})};
    const empty = checked.text === "" && !placement && !checked.evidence?.length && !checked.unboundReferences?.length;
    if (empty) {this.versions.delete(key); this.drafts.delete(key);}
    else {
      this.versions.set(key, {revision: crypto.randomUUID(), originKey: old?.originKey ?? key, currentKey: key,
        sourceQueueRevision: placement ? (renewedQueue || placement !== previous.queuePlacement ? crypto.randomUUID() : old?.sourceQueueRevision ?? null) : null});
      this.drafts.set(key, next);
    }
    ++this.mutation; this.removedEvidence(previous, next); this.persistEdit(); this.publish(key);
  }
  restoreQueue(key: string, placement: QueueDraftPlacement): void {
    this.setStructured(key, restoreAgentSourceDraft(agentDraftFromInput(placement.input.text, placement.input.userInput)), placement, true);
  }
  subscribe(key: string, listener: () => void): () => void {
    const listeners = this.listeners.get(key) ?? new Set<() => void>(); listeners.add(listener); this.listeners.set(key, listeners);
    return () => {listeners.delete(listener); if (!listeners.size) this.listeners.delete(key);};
  }
}
