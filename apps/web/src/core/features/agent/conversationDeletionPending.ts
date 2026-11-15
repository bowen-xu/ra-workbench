import { decodeAgentDeletionRequest, decodeAgentDeletionResponse, decodeAgentDeletionWriteOptions,
  agentDeletionErrors, type AgentDeletionError, type AgentDeletionRequest, type AgentDeletionResponse, type AgentConversationBinding } from "@research-assistant/core/agent";
import { appStorageScopedKey } from "../../runtime/appStorage";

export const deletionRetentions = ["branchHistory", "proposalApplications", "executionWorkspace"] as const;
export interface ConversationDeletionRecord {
  readonly schemaVersion: 1; readonly intent: AgentDeletionRequest; readonly internal: boolean;
  readonly epoch: string | null;
  readonly stage: "pending" | "unknown" | "retained" | "rejected" | "committed";
  readonly reason: AgentDeletionError | null; readonly receipt: AgentDeletionResponse | null;
}
const prefix = "research-assistant:conversation-deletion:v1:";
const key = (id: string) => prefix + encodeURIComponent(id);
const changed = "research-assistant:conversation-deletion-recovery-changed";
function invalid(): never { throw new Error("DELETION_RECOVERY_INVALID"); }
export function announceConversationDeletionRecovery(storage: Storage): void {
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent(changed, {detail: storage}));
}
function decode(value: unknown): ConversationDeletionRecord {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const row = value as Record<string, unknown>;
  if (row.schemaVersion !== 1 || Object.keys(row).some(name => !["schemaVersion", "intent", "internal", "epoch", "stage", "reason", "receipt"].includes(name))
    || typeof row.internal !== "boolean" || !["pending", "unknown", "retained", "rejected", "committed"].includes(String(row.stage))) return invalid();
  const intent = decodeAgentDeletionRequest(row.intent), stage = row.stage as ConversationDeletionRecord["stage"];
  const epoch = row.epoch === null ? null : decodeAgentDeletionWriteOptions({expectedEpoch: row.epoch}).expectedEpoch;
  if (stage !== "pending" && epoch === null) return invalid();
  const receipt = row.receipt === null ? null : decodeAgentDeletionResponse(row.receipt);
  if ((stage === "committed") !== (receipt !== null) || receipt &&
    (receipt.requestId !== intent.requestId || receipt.conversationId !== intent.conversationId)) return invalid();
  const reason = row.reason === null ? null : agentDeletionErrors.find(error => error === row.reason) ?? invalid();
  if (["retained", "rejected"].includes(stage) !== (reason !== null)
    || stage === "retained" && (intent.force || !deletionRetentions.some(error => error === reason))) return invalid();
  return {schemaVersion: 1, intent, internal: row.internal, epoch, stage, reason, receipt};
}
export function sameDeletionIntent(a: AgentDeletionRequest, b: AgentDeletionRequest): boolean {
  return JSON.stringify(decodeAgentDeletionRequest(a)) === JSON.stringify(decodeAgentDeletionRequest(b));
}
export function readConversationDeletion(storage: Storage, id: string): ConversationDeletionRecord | null {
  const raw = storage.getItem(key(id)); if (raw === null) return null;
  try {
    if (raw.length > 16_384) return invalid();
    const record = decode(JSON.parse(raw));
    if (record.intent.conversationId !== id) return invalid();
    return record;
  } catch { return invalid(); }
}
export function saveConversationDeletion(storage: Storage, input: ConversationDeletionRecord, initial = false): void {
  const record = decode(input), previous = readConversationDeletion(storage, record.intent.conversationId);
  const forceConfirmation = previous?.stage === "retained" && !previous.intent.force && record.intent.force && record.stage === "pending"
    && sameDeletionIntent({...previous.intent, force: true}, record.intent);
  if ((!previous && !initial) || previous &&
    (previous.internal !== record.internal || !sameDeletionIntent(previous.intent, record.intent) && !forceConfirmation
      || previous.epoch !== null && previous.epoch !== record.epoch
      || previous.stage !== "pending" && record.stage === "pending" && !forceConfirmation
      || previous.stage === "committed" && record.stage !== "committed")) throw new Error("DELETION_RECOVERY_CONFLICT");
  storage.setItem(key(record.intent.conversationId), JSON.stringify(record)); announceConversationDeletionRecovery(storage);
}
/** Unknown/partial outcomes cannot be abandoned through local cleanup. */
export function clearConversationDeletion(storage: Storage, input: ConversationDeletionRecord): void {
  const record = decode(input);
  const previous = readConversationDeletion(storage, record.intent.conversationId);
  if (record.stage === "unknown" || previous && (JSON.stringify(previous) !== JSON.stringify(record))) throw new Error("DELETION_RECOVERY_CONFLICT");
  storage.removeItem(key(record.intent.conversationId)); announceConversationDeletionRecovery(storage);
}
export function sameDeletionHost(record: ConversationDeletionRecord, binding: AgentConversationBinding | null, internal: boolean): boolean {
  const original = record.intent.binding;
  return record.internal === internal && (original === null ? binding === null : original.kind === binding?.kind && original.resourceId === binding.resourceId);
}
export function discoverConversationDeletions(storage: Storage, binding: AgentConversationBinding | null, internal: boolean,
  reportIssue?: (cause: unknown) => void): readonly ConversationDeletionRecord[] {
  return discoverAllConversationDeletions(storage, reportIssue).filter(record => sameDeletionHost(record, binding, internal));
}
/** Cross-host pending identity guard; views still filter before displaying titles. */
export function discoverAllConversationDeletions(storage: Storage,
  reportIssue?: (cause: unknown) => void): readonly ConversationDeletionRecord[] {
  const records: ConversationDeletionRecord[] = [];
  for (let index = 0; index < storage.length; index++) {
    const name = storage.key(index); if (!name?.startsWith(prefix)) continue;
    try {
      const record = readConversationDeletion(storage, decodeURIComponent(name.slice(prefix.length)));
      if (record) records.push(record);
    } catch (cause: unknown) { if (reportIssue) reportIssue(cause); else throw cause; }
  }
  return records;
}
/** Local discovery invalidation only; never authorizes backend I/O. */
export function subscribeConversationDeletions(storage: Storage, refresh: () => void): () => void {
  const start = appStorageScopedKey(storage, "research-assistant:conversation-deletion");
  const local = (event: Event) => { if (event instanceof CustomEvent && event.detail === storage) refresh(); };
  const remote = (event: StorageEvent) => { if (event.key === null || event.key.startsWith(start)) refresh(); };
  window.addEventListener(changed, local); window.addEventListener("storage", remote); window.addEventListener("focus", refresh);
  return () => { window.removeEventListener(changed, local); window.removeEventListener("storage", remote); window.removeEventListener("focus", refresh); };
}
