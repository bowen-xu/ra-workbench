import {decodeAgentWritingQueueEditRequest, type AgentWritingQueueEditRequest, type AgentWritingQueuedStartRequest,
  type AgentWritingQueueResponse, type AgentWritingQueueScope} from "@research-assistant/core/agent";
import type {AgentInputQueueController} from "@research-assistant/data/agent";
import {WritingDispatchAdmissionClosed} from "./writingDispatchAdmission";
type Lease = {readonly controller: AgentInputQueueController<AgentWritingQueueScope, AgentWritingQueueResponse>; readonly release: () => void};
export type WritingQueuedStartResumeResult = "acknowledged" | "knownRejected" | "unknown";
export interface WritingQueuedStartQueuePort {
  read(intent: AgentWritingQueuedStartRequest): Promise<AgentWritingQueueResponse>;
  resume(intent: AgentWritingQueuedStartRequest, original: AgentWritingQueueEditRequest | null,
    beforeSend: (command: AgentWritingQueueEditRequest) => Promise<void>, eligible: () => boolean,
    isAdmissionDisabled?: () => boolean): Promise<WritingQueuedStartResumeResult>;
}
/** The existing queue owner retains/replays Resume; this port never creates a
 * second queue scheduler or takes a queue read as an edit acknowledgement. */
export function writingQueuedStartQueue(acquire: () => Lease | null, read: (scope: AgentWritingQueueScope) => Promise<AgentWritingQueueResponse>): WritingQueuedStartQueuePort {
  return {
    async read(intent) {
      const queue = await read({conversationId: intent.start.conversationId, manuscriptId: intent.start.manuscriptId});
      if (queue.conversationId !== intent.start.conversationId || queue.manuscriptId !== intent.start.manuscriptId) throw new Error("QUEUE_START_QUEUE_READ_FAILED");
      return queue;
    },
    async resume(intent, original, beforeSend, eligible, isAdmissionDisabled) {
      const lease = acquire(); if (!lease) throw new Error("QUEUE_START_QUEUE_RESUME_FAILED");
      try {
        const owner = lease.controller; await owner.reload(); if (!eligible()) return "unknown";
        const state = owner.getSnapshot(), queue = state.queue;
        if (!queue || state.loading || state.editing || queue.conversationId !== intent.start.conversationId || queue.manuscriptId !== intent.start.manuscriptId
          || queue.preparingInputId !== null) throw new Error("QUEUE_START_OTHER_CLAIM");
        if (original) {
          if (!owner.restorePending(original, undefined, command => beforeSend(decodeAgentWritingQueueEditRequest(command)))) throw new Error("QUEUE_START_QUEUE_RESUME_BUSY");
          return await owner.retry() ? "acknowledged" : "unknown";
        }
        if (state.uncertain || state.error) throw new Error("QUEUE_START_QUEUE_RESUME_BUSY");
        if (isAdmissionDisabled?.()) throw new WritingDispatchAdmissionClosed("resume", intent.start.conversationId, intent.start.manuscriptId, intent.start.requestId);
        const acknowledged = await owner.edit({type: "resume"}, undefined, command => beforeSend(decodeAgentWritingQueueEditRequest(command)), undefined, isAdmissionDisabled);
        if (acknowledged) return "acknowledged";
        if (owner.getSnapshot().error?.startsWith("WRITING_RUNTIME_UNAVAILABLE："))
          throw new WritingDispatchAdmissionClosed("resume", intent.start.conversationId, intent.start.manuscriptId, intent.start.requestId);
        return !owner.getPendingRequest() && !owner.getSnapshot().uncertain ? "knownRejected" : "unknown";
      } finally {lease.release();}
    },
  };
}
