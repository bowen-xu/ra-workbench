import {decodeAgentContextQueueEditRequest, type AgentContextQueueEditRequest, type AgentQueueState} from "@research-assistant/core/agent";
import type {AgentInputQueueController} from "@research-assistant/data/agent";

type Scope = {conversationId: string};
type Lease = {controller: AgentInputQueueController<Scope, AgentQueueState>; release: () => void};
export interface IndependentQueueResume {
  read(conversationId: string): Promise<AgentQueueState>;
  resume(conversationId: string, epoch: string, original: AgentContextQueueEditRequest | null,
    beforeSend: (request: AgentContextQueueEditRequest) => Promise<void>, eligible: () => boolean): Promise<"acknowledged" | "knownRejected" | "unknown">;
}
/** Reuse the conversation queue owner and its exact receipt retry. */
export function independentQueueResume(acquire: () => Lease | null, read: (scope: Scope) => Promise<AgentQueueState>): IndependentQueueResume {
  return {
    async read(conversationId) {
      const queue = await read({conversationId});
      if (queue.conversationId !== conversationId) throw new Error("CONTEXT_QUEUE_READ_MISMATCH");
      return queue;
    },
    async resume(conversationId, epoch, original, beforeSend, eligible) {
      const lease = acquire(); if (!lease) throw new Error("CONTEXT_QUEUE_OWNER_UNAVAILABLE");
      try {
        const owner = lease.controller; await owner.reload();
        if (!eligible()) return "unknown";
        const state = owner.getSnapshot(), queue = state.queue;
        if (!queue || state.loading || state.editing || queue.conversationId !== conversationId || queue.preparingInputId !== null)
          throw new Error("CONTEXT_QUEUE_OTHER_CLAIM");
        const save = async (request: Scope & import("@research-assistant/core/agent").AgentQueueCommand) => {
          if (!eligible()) throw new Error("CONTEXT_QUEUE_RECOVERY_CHANGED");
          await beforeSend(decodeAgentContextQueueEditRequest(request));
          if (!eligible()) throw new Error("CONTEXT_QUEUE_RECOVERY_CHANGED");
        };
        if (original) {
          if (original.conversationId !== conversationId || original.expectedEpoch !== epoch || original.edit.type !== "resume") throw new Error("CONTEXT_QUEUE_RESUME_MISMATCH");
          if (!owner.restorePending(original, undefined, save)) throw new Error("CONTEXT_QUEUE_OWNER_BUSY");
          return await owner.retry() ? "acknowledged" : "unknown";
        }
        if (!queue.paused) return "unknown";
        if (state.uncertain || state.error) throw new Error("CONTEXT_QUEUE_OWNER_BUSY");
        const acknowledged = await owner.edit({type: "resume"}, undefined, save, epoch);
        if (acknowledged) return "acknowledged";
        return !owner.getPendingRequest() && !owner.getSnapshot().uncertain ? "knownRejected" : "unknown";
      } finally {lease.release();}
    },
  };
}
