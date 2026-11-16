import type {AgentPaperQueueResponse, AgentPaperQueueScope} from "@research-assistant/core/agent";
import type {AgentInputQueueController} from "@research-assistant/data/agent";
import {agentPaperQueueGateway} from "../../../../../../apps/web/src/core/public/agent";
import {setPaperOwnerPendingSubmission, type PaperConversationOwner} from "./paperConversationOwners";

/** Inspect without starting again; only an already proven start can be finished. */
export async function inspectPaperPreparation(owner: PaperConversationOwner,
  queue: AgentInputQueueController<AgentPaperQueueScope, AgentPaperQueueResponse> | null,
  onStarted: (conversationId: string, turnId: string) => void): Promise<"ready" | "complete" | "unconfirmed"> {
  const preparation = owner.preparation; if (!preparation) return "unconfirmed";
  if (!preparation.canRetryFinish()) await preparation.check();
  const state = preparation.getSnapshot();
  if (state.phase === "ready" && state.result && state.intent && state.context) {
    setPaperOwnerPendingSubmission(owner, {preview: state.result, text: state.intent.text, sourceCount: state.context.sourceCount,
      imageCount: state.context.imageCount, manualConfirmation: true,
      ...(state.intent.retrieval ? {retrievalTraceId: state.intent.retrieval.traceId} : {}),
      ...(state.context.queuedInputId ? {queuedInputId: state.context.queuedInputId} : {})});
    return "ready";
  }
  if ((state.phase === "started" || preparation.canRetryFinish()) && state.start?.result) {
    if (state.context?.queuedInputId && queue && agentPaperQueueGateway.inspectEdit) preparation.configureQueue(queue, {inspectEdit: agentPaperQueueGateway.inspectEdit});
    const observed = state.startStatus?.type === "started" ? state.startStatus.result : state.start.result;
    if (observed.outcome === null) onStarted(observed.conversationId, observed.turnId);
    await preparation.finish();
    if (preparation.getSnapshot().phase !== "complete") throw preparation.getSnapshot().cause ?? new Error("论文提交收尾尚待核对");
    setPaperOwnerPendingSubmission(owner, undefined); return "complete";
  }
  if (state.cause) throw state.cause;
  return "unconfirmed";
}
