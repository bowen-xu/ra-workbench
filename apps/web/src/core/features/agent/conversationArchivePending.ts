import { decodeAgentArchiveRequest, type AgentArchiveRequest, type AgentConversationBinding } from "@research-assistant/core/agent";

const prefix = "research-assistant:conversation-archive:v1:";
const key = (id: string) => prefix + id.toLowerCase();
export interface ConversationArchiveContext {
  readonly binding: AgentConversationBinding | null;
  readonly title: string;
  readonly internal: boolean;
}
export interface ConversationArchiveRecord {
  readonly schemaVersion: 1;
  readonly intent: AgentArchiveRequest;
  readonly context: ConversationArchiveContext;
  readonly stage: "unknown" | "committed";
}
function invalid(): never { throw new Error("ARCHIVE_RECOVERY_INVALID"); }
function decode(value: unknown): ConversationArchiveRecord {
  if (!value || typeof value !== "object" || Array.isArray(value)) return invalid();
  const row = value as Record<string, unknown>;
  if (row.schemaVersion !== 1 || Object.keys(row).some(name => !["schemaVersion", "intent", "context", "stage"].includes(name))
    || !["unknown", "committed"].includes(String(row.stage)) || !row.context || typeof row.context !== "object" || Array.isArray(row.context)) return invalid();
  const context = row.context as Record<string, unknown>;
  if (Object.keys(context).some(name => !["binding", "title", "internal"].includes(name))
    || typeof context.title !== "string" || typeof context.internal !== "boolean") return invalid();
  let binding: AgentConversationBinding | null = null;
  if (context.binding !== null) {
    if (!context.binding || typeof context.binding !== "object" || Array.isArray(context.binding)) return invalid();
    const source = context.binding as Record<string, unknown>;
    if (Object.keys(source).some(name => !["kind", "resourceId"].includes(name))
      || !["manuscript", "paper", "note", "readingNote"].includes(String(source.kind))
      || typeof source.resourceId !== "string" || !source.resourceId) return invalid();
    binding = { kind: source.kind as AgentConversationBinding["kind"], resourceId: source.resourceId };
  }
  return { schemaVersion: 1, intent: decodeAgentArchiveRequest(row.intent),
    context: { binding, title: context.title, internal: context.internal }, stage: row.stage as ConversationArchiveRecord["stage"] };
}
export function sameArchiveIntent(a: AgentArchiveRequest, b: AgentArchiveRequest): boolean {
  return a.operationId === b.operationId && a.id === b.id && a.archived === b.archived && a.expectedWatermark === b.expectedWatermark;
}
export function sameArchiveScope(a: Pick<ConversationArchiveContext, "binding" | "internal">, b: Pick<ConversationArchiveContext, "binding" | "internal">): boolean {
  return a.internal === b.internal && (a.binding === null ? b.binding === null
    : a.binding.kind === b.binding?.kind && a.binding.resourceId === b.binding.resourceId);
}
export function readConversationArchive(storage: Storage, id: string): ConversationArchiveRecord | null {
  const value = storage.getItem(key(id));
  if (value === null) return null;
  if (value.length > 32_768) return invalid();
  let parsed: unknown;
  try { parsed = JSON.parse(value); } catch { return invalid(); }
  let record: ConversationArchiveRecord;
  try { record = decode(parsed); } catch { return invalid(); }
  if (record.intent.id !== id.toLowerCase()) return invalid();
  return record;
}
export function discoverConversationArchives(storage: Storage, scope: Pick<ConversationArchiveContext, "binding" | "internal">,
  reportIssue?: (cause: unknown) => void): readonly ConversationArchiveRecord[] {
  const records: ConversationArchiveRecord[] = [];
  for (let index = 0; index < storage.length; index++) {
    const name = storage.key(index);
    if (!name?.startsWith(prefix)) continue;
    try {
      const record = readConversationArchive(storage, name.slice(prefix.length));
      if (record && sameArchiveScope(record.context, scope)) records.push(record);
    } catch (cause: unknown) { if (reportIssue) reportIssue(cause); else throw cause; }
  }
  return records;
}
export function saveConversationArchive(storage: Storage, record: ConversationArchiveRecord, initial: boolean): void {
  const previous = readConversationArchive(storage, record.intent.id);
  if ((!previous && !initial) || (previous && (!sameArchiveIntent(previous.intent, record.intent)
    || !sameArchiveScope(previous.context, record.context) || previous.stage === "committed" && record.stage !== "committed"))) throw new Error("ARCHIVE_RECOVERY_CONFLICT");
  storage.setItem(key(record.intent.id), JSON.stringify(decode(record)));
}
export function clearConversationArchive(storage: Storage, record: ConversationArchiveRecord, clearing: boolean): void {
  const previous = readConversationArchive(storage, record.intent.id);
  if (!previous) { if (clearing) return; throw new Error("ARCHIVE_RECOVERY_CONFLICT"); }
  if (!sameArchiveIntent(previous.intent, record.intent) || !sameArchiveScope(previous.context, record.context)
    || previous.stage === "committed" && record.stage !== "committed") throw new Error("ARCHIVE_RECOVERY_CONFLICT");
  storage.removeItem(key(record.intent.id));
}
