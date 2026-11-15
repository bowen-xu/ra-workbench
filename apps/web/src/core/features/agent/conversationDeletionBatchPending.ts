import { decodeAgentDeletionRequest, decodeAgentDeletionWriteOptions,
  type AgentConversationBinding, type AgentDeletionRequest } from "@research-assistant/core/agent";
import { announceConversationDeletionRecovery } from "./conversationDeletionPending";

/** The whole authorized set survives before any member may mutate. Finishing
 * revokes further writes, including when local receipt removal later fails. */
export interface ConversationDeletionBatch {
  readonly schemaVersion: 1; readonly batchId: string; readonly binding: AgentConversationBinding | null;
  readonly internal: boolean; readonly epoch: string | null;
  readonly requests: readonly AgentDeletionRequest[];
  readonly phase: "authorized" | "executing" | "finishing"; readonly retained: readonly string[];
}
const prefix = "research-assistant:conversation-deletion-batch:v1:";
const key = (id: string) => prefix + encodeURIComponent(id);
function invalid(): never {throw Error("DELETION_BATCH_INVALID");}
export function deletionBatchScope(binding: AgentConversationBinding | null, internal: boolean): string {
  return JSON.stringify([binding ? [binding.kind, binding.resourceId] : null, internal]);
}
export function decodeConversationDeletionBatch(value: unknown): ConversationDeletionBatch {
  try {
    if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
    const row = value as Record<string, unknown>;
    if (Object.keys(row).some(name => !["schemaVersion", "batchId", "binding", "internal", "epoch", "requests", "phase", "retained"].includes(name))
      || row.schemaVersion !== 1 || typeof row.internal !== "boolean" || !["authorized", "executing", "finishing"].includes(String(row.phase))
      || !Array.isArray(row.requests) || !row.requests.length || !Array.isArray(row.retained)) return invalid();
    const batchId = decodeAgentDeletionWriteOptions({expectedEpoch: row.batchId}).expectedEpoch;
    const epoch = row.epoch === null ? null : decodeAgentDeletionWriteOptions({expectedEpoch: row.epoch}).expectedEpoch;
    const requests = row.requests.map(decodeAgentDeletionRequest), first = requests[0]!;
    const binding = row.binding === null ? null : first.binding;
    if (JSON.stringify(row.binding) !== JSON.stringify(binding)
      || new Set(requests.map(request => request.conversationId)).size !== requests.length
      || new Set([batchId, ...requests.map(request => request.requestId)]).size !== requests.length + 1
      || requests.some(request => !request.force || JSON.stringify(request.binding) !== JSON.stringify(binding))) return invalid();
    const retained = row.retained as unknown[];
    if (retained.some(id => typeof id !== "string" || !requests.some(request => request.conversationId === id))
      || new Set(retained).size !== retained.length || row.phase !== "finishing" && retained.length
      || row.phase === "executing" && epoch === null
      || row.phase === "finishing" && epoch === null && retained.length !== requests.length) return invalid();
    return {schemaVersion: 1, batchId, binding, internal: row.internal, epoch, requests,
      phase: row.phase as ConversationDeletionBatch["phase"], retained: retained as string[]};
  } catch {return invalid();}
}
export function readConversationDeletionBatch(storage: Storage, id: string): ConversationDeletionBatch | null {
  const raw = storage.getItem(key(id)); if (raw === null) return null;
  try {
    const batch = decodeConversationDeletionBatch(JSON.parse(raw));
    if (batch.batchId !== id) return invalid(); return batch;
  } catch {return invalid();}
}
export function discoverConversationDeletionBatches(storage: Storage, report?: (cause: unknown) => void): readonly ConversationDeletionBatch[] {
  const found: ConversationDeletionBatch[] = [];
  for (let index = 0; index < storage.length; index++) {
    const name = storage.key(index); if (!name?.startsWith(prefix)) continue;
    try {
      const batch = readConversationDeletionBatch(storage, decodeURIComponent(name.slice(prefix.length)));
      if (batch) found.push(batch);
    } catch (cause: unknown) {if (report) report(cause); else throw cause;}
  }
  return found;
}
export function saveConversationDeletionBatch(storage: Storage, input: ConversationDeletionBatch): void {
  const batch = decodeConversationDeletionBatch(input), previous = readConversationDeletionBatch(storage, batch.batchId);
  if (previous) {
    const stable = (record: ConversationDeletionBatch) => JSON.stringify({...record, epoch: null, phase: "authorized", retained: []});
    if (stable(previous) !== stable(batch) || previous.epoch !== null && previous.epoch !== batch.epoch
      || previous.phase === "executing" && batch.phase === "authorized"
      || previous.phase === "finishing" && JSON.stringify(previous) !== JSON.stringify(batch)) throw Error("DELETION_BATCH_CONFLICT");
  } else {
    const ids = new Set(batch.requests.map(request => request.conversationId));
    const operations = new Set([batch.batchId, ...batch.requests.map(request => request.requestId)]);
    if (discoverConversationDeletionBatches(storage).some(other => deletionBatchScope(other.binding, other.internal) === deletionBatchScope(batch.binding, batch.internal)
      || operations.has(other.batchId) || other.requests.some(request => ids.has(request.conversationId) || operations.has(request.requestId)))) throw Error("DELETION_BATCH_CONFLICT");
  }
  storage.setItem(key(batch.batchId), JSON.stringify(batch)); announceConversationDeletionRecovery(storage);
}
export function clearConversationDeletionBatch(storage: Storage, input: ConversationDeletionBatch): void {
  const batch = decodeConversationDeletionBatch(input), previous = readConversationDeletionBatch(storage, batch.batchId);
  if (batch.phase !== "finishing" || previous && JSON.stringify(previous) !== JSON.stringify(batch)) throw Error("DELETION_BATCH_CONFLICT");
  storage.removeItem(key(batch.batchId)); announceConversationDeletionRecovery(storage);
}
