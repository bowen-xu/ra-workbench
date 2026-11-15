import { decodeAgentCreateBoundRequest, type AgentConversationBinding, type AgentCreateBoundRequest } from "@research-assistant/core/agent";
import { getAppStorage } from "../../runtime/appStorage";

export type BoundCreationStage = "pending" | "unknown" | "committed";
export function boundCreationScope(binding: AgentConversationBinding): string { return `${binding.kind}:${encodeURIComponent(binding.resourceId)}`; }
const key = (binding: AgentConversationBinding) => `research-assistant:bound-conversation-creation:v1:${boundCreationScope(binding)}`;
export function sameBoundCreation(a: AgentCreateBoundRequest, b: AgentCreateBoundRequest): boolean {
  return a.operationId === b.operationId && a.id === b.id && a.binding.kind === b.binding.kind && a.binding.resourceId === b.binding.resourceId;
}
export function readBoundCreation(binding: AgentConversationBinding, storage: Storage = getAppStorage()): { intent: AgentCreateBoundRequest; stage: BoundCreationStage } | null {
  const raw = storage.getItem(key(binding));
  if (raw === null) return null;
  try {
    const row: unknown = JSON.parse(raw);
    if (!row || typeof row !== "object" || !("schemaVersion" in row) || row.schemaVersion !== 1 || !("intent" in row)
      || !("stage" in row) || typeof row.stage !== "string" || !["pending", "unknown", "committed"].includes(row.stage)
      || Object.keys(row).some(name => !["schemaVersion", "intent", "stage"].includes(name))) throw new Error();
    const intent = decodeAgentCreateBoundRequest(row.intent);
    if (boundCreationScope(intent.binding) !== boundCreationScope(binding)) throw new Error();
    return { intent, stage: row.stage as BoundCreationStage };
  } catch { throw new Error("CREATION_RECOVERY_INVALID"); }
}
export function persistBoundCreation(intent: AgentCreateBoundRequest, stage: BoundCreationStage, storage: Storage = getAppStorage()): void {
  const previous = readBoundCreation(intent.binding, storage);
  if (previous && !sameBoundCreation(previous.intent, intent)) throw new Error("CREATION_RECOVERY_CONFLICT");
  storage.setItem(key(intent.binding), JSON.stringify({ schemaVersion: 1, intent, stage }));
}
/** A local draft gets its own creation phase before any IPC. It is not an Agent submission. */
export function reserveBoundConversation(binding: AgentConversationBinding, id?: string, storage: Storage = getAppStorage()): string {
  const previous = readBoundCreation(binding, storage);
  if (previous) {
    if (id && previous.intent.id !== id.toLowerCase()) throw new Error("CREATION_RECOVERY_CONFLICT");
    return previous.intent.id;
  }
  const intent = decodeAgentCreateBoundRequest({ operationId: crypto.randomUUID(), id: id ?? crypto.randomUUID(), binding });
  persistBoundCreation(intent, "pending", storage); return intent.id;
}
export function acknowledgeBoundCreation(intent: AgentCreateBoundRequest, storage: Storage = getAppStorage()): void {
  const previous = readBoundCreation(intent.binding, storage);
  if (!previous) return;
  if (!sameBoundCreation(previous.intent, intent)) throw new Error("CREATION_RECOVERY_CONFLICT");
  storage.removeItem(key(intent.binding));
}
/** Invalid records remain untouched and are reported by the creation owner. */
export function pendingBoundConversationId(binding: AgentConversationBinding, storage: Storage = getAppStorage()): string | null {
  try { return readBoundCreation(binding, storage)?.intent.id ?? null; } catch { return null; }
}
