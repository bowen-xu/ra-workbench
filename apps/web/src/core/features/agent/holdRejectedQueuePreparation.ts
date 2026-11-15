import {sameAgentWritingQueuePreviewRequest, type AgentWritingQueuePreviewRequest, type AgentWritingQueueResponse, type AgentWritingQueueScope} from "@research-assistant/core/agent";
import type {AgentInputQueueController} from "@research-assistant/data/agent";
type QueueOwner = AgentInputQueueController<AgentWritingQueueScope, AgentWritingQueueResponse>;
/** Use the existing durable queue owner so returning a manual failed attempt
 * cannot release the same input into automatic FIFO. No new scheduler exists. */
export async function holdRejectedQueuePreparation(lease: {readonly controller: QueueOwner; readonly release: () => void} | null,
  intent: AgentWritingQueuePreviewRequest, eligible: () => boolean = () => true): Promise<void> {
  if (!lease) throw new Error("QUEUE_PREPARE_QUEUE_HOLD_FAILED");
  try {
    if (!eligible()) throw new Error("QUEUE_PREPARE_QUEUE_HOLD_FAILED");
    const owner = lease.controller; await owner.reload();
    if (!eligible()) throw new Error("QUEUE_PREPARE_QUEUE_HOLD_FAILED");
    const state = owner.getSnapshot(), queue = state.queue, input = queue?.inputs.find(input => input.inputId === intent.inputId);
    if (!queue || state.loading || state.editing || state.uncertain || state.error) throw new Error("QUEUE_PREPARE_QUEUE_HOLD_FAILED");
    if (queue.conversationId !== intent.submission.conversationId || queue.manuscriptId !== intent.submission.context.manuscriptId || !input
      || !sameAgentWritingQueuePreviewRequest(intent, {...intent, submission: {...intent.submission, text: input.text, userInput: input.userInput ?? []}}))
      throw new Error("QUEUE_PREPARE_INPUT_CHANGED");
    if (queue.preparingInputId === intent.inputId) throw new Error("QUEUE_PREPARE_CLAIM_CHANGED");
    if (!input.paused && !await owner.edit({type: "pauseInput", inputId: intent.inputId})) throw new Error("QUEUE_PREPARE_QUEUE_HOLD_FAILED");
  } finally {lease.release();}
}
