import { decodeAgentForkRequest, type AgentForkRequest, type AgentConversationBinding } from "@research-assistant/core/agent";
import { appStorageEventKey } from "../../runtime/appStorage";

export type ForkScope = Omit<AgentForkRequest, "childId">;
export interface ConversationForkRecord {
  readonly schemaVersion: 1; readonly intent: AgentForkRequest;
  readonly epoch: string | null; readonly stage: "pending" | "unknown" | "committed";
}
export function forkScope(scope: ForkScope): string {
  return `${scope.binding.kind}:${encodeURIComponent(scope.binding.resourceId)}:${scope.parentId}:${scope.throughTurnId}`;
}
const prefix = "research-assistant:conversation-fork:v1:";
const bindingPrefix = (binding: AgentConversationBinding) => `${prefix}${binding.kind}:${encodeURIComponent(binding.resourceId)}:`;
const key = (scope: ForkScope) => prefix + forkScope(scope);
const changed = "research-assistant:conversation-fork-recovery-changed";
export function announceConversationForkRecovery(storage: Storage): void {
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent(changed, { detail: storage }));
}
export function sameForkIntent(a: AgentForkRequest, b: AgentForkRequest): boolean {
  return a.childId === b.childId && forkScope(a) === forkScope(b);
}
function invalid(): never { throw new Error("FORK_RECOVERY_INVALID"); }
export function readConversationFork(storage: Storage, scope: ForkScope): ConversationForkRecord | null {
  const raw = storage.getItem(key(scope));
  if (raw === null) return null;
  try {
    if (raw.length > 8192) return invalid();
    const row: unknown = JSON.parse(raw);
    if (!row || typeof row !== "object" || Array.isArray(row)) return invalid();
    const record = row as Record<string, unknown>;
    if (record.schemaVersion !== 1 || Object.keys(record).some(name => !["schemaVersion", "intent", "epoch", "stage"].includes(name))
      || !["pending", "unknown", "committed"].includes(String(record.stage))) return invalid();
    const intent = decodeAgentForkRequest(record.intent);
    if (forkScope(intent) !== forkScope(scope)) return invalid();
    if (record.epoch !== null && (typeof record.epoch !== "string"
      || !/^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/u.test(record.epoch)
      || record.epoch === "00000000-0000-0000-0000-000000000000")) return invalid();
    if (record.stage !== "pending" && record.epoch === null) return invalid();
    return { schemaVersion: 1, intent, epoch: record.epoch, stage: record.stage as ConversationForkRecord["stage"] };
  } catch { return invalid(); }
}
export function saveConversationFork(storage: Storage, record: ConversationForkRecord, initial = false): void {
  const previous = readConversationFork(storage, record.intent);
  if ((!previous && !initial) || previous && (!sameForkIntent(previous.intent, record.intent)
    || previous.epoch !== null && previous.epoch !== record.epoch
    || previous.stage === "committed" && record.stage !== "committed")) throw new Error("FORK_RECOVERY_CONFLICT");
  storage.setItem(key(record.intent), JSON.stringify(record));
  announceConversationForkRecovery(storage);
}
/** Only a known committed operation may be explicitly finished by its live view. */
export function clearConversationFork(storage: Storage, record: ConversationForkRecord): void {
  const previous = readConversationFork(storage, record.intent);
  if (previous && (!sameForkIntent(previous.intent, record.intent) || previous.epoch !== record.epoch)) throw new Error("FORK_RECOVERY_CONFLICT");
  storage.removeItem(key(record.intent));
  announceConversationForkRecovery(storage);
}

/** Only this exact binding is inspected; malformed records remain untouched. */
export function discoverConversationForks(storage: Storage, binding: AgentConversationBinding,
  reportIssue?: (cause: unknown) => void): readonly ConversationForkRecord[] {
  const records: ConversationForkRecord[] = [], start = bindingPrefix(binding);
  for (let index = 0; index < storage.length; index++) {
    const name = storage.key(index);
    if (!name?.startsWith(start)) continue;
    try {
      const ids = name.slice(start.length).split(":");
      if (ids.length !== 2) return invalid();
      const [parentId, throughTurnId] = ids;
      if (!parentId || !throughTurnId) return invalid();
      const record = readConversationFork(storage, { parentId, throughTurnId, binding });
      if (record) records.push(record);
    } catch (cause: unknown) { if (reportIssue) reportIssue(cause); else throw cause; }
  }
  return records;
}
/** Invalidation only. Same-window saves and cross-window storage changes never
 * run IPC, flush pending records, or authorize an original mutation. */
export function subscribeConversationForks(storage: Storage, binding: AgentConversationBinding, refresh: () => void): () => void {
  const start = appStorageEventKey(bindingPrefix(binding));
  const local = (event: Event) => { if (event instanceof CustomEvent && event.detail === storage) refresh(); };
  const remote = (event: StorageEvent) => { if (event.key === null || event.key.startsWith(start)) refresh(); };
  window.addEventListener(changed, local); window.addEventListener("storage", remote); window.addEventListener("focus", refresh);
  return () => { window.removeEventListener(changed, local); window.removeEventListener("storage", remote); window.removeEventListener("focus", refresh); };
}
