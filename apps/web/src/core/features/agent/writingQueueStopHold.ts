import {decodeAgentWritingQueueScope, type AgentWritingQueueScope} from "@research-assistant/core/agent";
import {flushStorage} from "../../runtime/storagePersistence";

export interface WritingQueueStopHold extends AgentWritingQueueScope {readonly inputId: string}
const key = (scope: AgentWritingQueueScope) => `research-assistant:writing-queue-stop:v1:${scope.manuscriptId}:${scope.conversationId}`;

/** Prevents automatic dispatch while the original queue pause is unresolved. */
export class WritingQueueStopHolds {
  constructor(private readonly storage: Storage) {}
  read(scope: AgentWritingQueueScope): WritingQueueStopHold | null {
    const own = decodeAgentWritingQueueScope({conversationId:scope.conversationId,manuscriptId:scope.manuscriptId});
    const raw = this.storage.getItem(key(own)); if (raw === null) return null;
    try {
      const value: unknown = JSON.parse(raw);
      if (!value || typeof value !== "object" || Array.isArray(value)
        || Object.keys(value).some(field => !["schemaVersion","conversationId","manuscriptId","inputId"].includes(field))
        || !("schemaVersion" in value) || value.schemaVersion !== 1 || !("conversationId" in value)
        || !("manuscriptId" in value) || !("inputId" in value)) throw new Error();
      const decoded = decodeAgentWritingQueueScope({conversationId:value.conversationId,manuscriptId:value.manuscriptId});
      const inputId = decodeAgentWritingQueueScope({conversationId:value.inputId,manuscriptId:own.manuscriptId}).conversationId;
      if (decoded.conversationId !== own.conversationId || decoded.manuscriptId !== own.manuscriptId) throw new Error();
      return {...decoded,inputId};
    } catch {throw new Error("WRITING_QUEUE_STOP_HOLD_INVALID");}
  }
  async save(value: WritingQueueStopHold): Promise<void> {
    const scope = decodeAgentWritingQueueScope({conversationId:value.conversationId,manuscriptId:value.manuscriptId});
    const inputId = decodeAgentWritingQueueScope({conversationId:value.inputId,manuscriptId:scope.manuscriptId}).conversationId;
    const original = this.read(scope);
    if (original && original.inputId !== inputId) throw new Error("WRITING_QUEUE_STOP_HOLD_CONFLICT");
    this.storage.setItem(key(scope),JSON.stringify({schemaVersion:1,...scope,inputId}));
    await flushStorage(this.storage);
  }
  async clear(value: WritingQueueStopHold): Promise<void> {
    const original = this.read(value);
    if (original && original.inputId !== value.inputId) throw new Error("WRITING_QUEUE_STOP_HOLD_CONFLICT");
    if (original) this.storage.removeItem(key(value));
    await flushStorage(this.storage);
  }
}
