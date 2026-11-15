import {decodeAgentWritingQueueScope, type AgentWritingQueueScope} from "@research-assistant/core/agent";
import {flushStorage} from "../../runtime/storagePersistence";

export interface WritingSendStopIntent extends AgentWritingQueueScope {
  readonly requestId: string;
}

const key = (scope: AgentWritingQueueScope) =>
  `research-assistant:writing-send-stop:v1:${scope.manuscriptId}:${scope.conversationId}`;

/** The stop request refers to an existing submission, never to a new run. */
export class WritingSendStopIntents {
  constructor(private readonly storage: Storage) {}

  read(scope: AgentWritingQueueScope): WritingSendStopIntent | null {
    const own = decodeAgentWritingQueueScope({conversationId: scope.conversationId, manuscriptId: scope.manuscriptId}), raw = this.storage.getItem(key(own));
    if (raw === null) return null;
    try {
      const value: unknown = JSON.parse(raw);
      if (!value || typeof value !== "object" || Array.isArray(value)
        || Object.keys(value).some(field => !["schemaVersion", "conversationId", "manuscriptId", "requestId"].includes(field))
        || !("schemaVersion" in value) || value.schemaVersion !== 1 || !("requestId" in value)) throw new Error();
      if (!("conversationId" in value) || !("manuscriptId" in value)) throw new Error();
      const decoded = decodeAgentWritingQueueScope({conversationId: value.conversationId, manuscriptId: value.manuscriptId});
      const requestId = decodeAgentWritingQueueScope({conversationId: value.requestId, manuscriptId: own.manuscriptId}).conversationId;
      if (decoded.conversationId !== own.conversationId || decoded.manuscriptId !== own.manuscriptId) throw new Error();
      return {...decoded, requestId};
    } catch { throw new Error("WRITING_SEND_STOP_INTENT_INVALID"); }
  }

  async save(intent: WritingSendStopIntent): Promise<void> {
    const own = decodeAgentWritingQueueScope({conversationId: intent.conversationId, manuscriptId: intent.manuscriptId});
    const requestId = decodeAgentWritingQueueScope({conversationId: intent.requestId, manuscriptId: own.manuscriptId}).conversationId;
    const previous = this.read(own);
    if (previous && previous.requestId !== requestId) throw new Error("WRITING_SEND_STOP_INTENT_CONFLICT");
    this.storage.setItem(key(own), JSON.stringify({schemaVersion: 1, ...own, requestId}));
    await flushStorage(this.storage);
  }

  async clear(intent: WritingSendStopIntent): Promise<void> {
    const current = this.read(intent);
    if (current && current.requestId !== intent.requestId) throw new Error("WRITING_SEND_STOP_INTENT_CONFLICT");
    if (current) this.storage.removeItem(key(intent));
    await flushStorage(this.storage);
  }
}
