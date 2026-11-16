import {executionModeForModel, type AgentStartResponse, type AgentPaperQueueResponse, type AgentPaperQueueScope} from "@research-assistant/core/agent";
import type {AgentInputQueueController} from "@research-assistant/data/agent";
import {agentPaperQueueGateway, agentPaperSubmissionGateway} from "../../../../../../apps/web/src/core/public/agent";
import type {PaperPreparation} from "./paperPreparation";
import type {PendingPaperSubmission} from "./paperConversationOwners";

/** Keep start, original queue receipt, and draft completion under the same owner. */
export async function startPreparedPaper(pending: PendingPaperSubmission, preparation: PaperPreparation | null,
  queue: AgentInputQueueController<AgentPaperQueueScope, AgentPaperQueueResponse> | null,
  queueState: AgentPaperQueueResponse | undefined, onStarted: (result: AgentStartResponse) => void, onQueueRemoved: () => void): Promise<void> {
  const mode = executionModeForModel(pending.preview.input.model, pending.preview.input.reasoningEffort);
  if (mode === null) throw new Error("当前模型尚未接入共享阅读 Agent 的已审核执行配置。");
  if (preparation?.unresolved() && preparation.getSnapshot().intent?.requestId !== pending.preview.requestId) throw new Error("另一条论文提交仍待核对");
  const managed = preparation?.getSnapshot().intent?.requestId === pending.preview.requestId ? preparation : null;
  let started: AgentStartResponse;
  if (managed) {
    if (!managed.canRetryFinish()) await managed.confirmStart({conversationId: pending.preview.conversationId, requestId: pending.preview.requestId, paperId: pending.preview.paperId,
      paperContextSnapshotId: pending.preview.paperContextSnapshotId, contextSnapshotId: pending.preview.contextSnapshotId, model: pending.preview.input.model, mode});
    const state = managed.getSnapshot();
    if ((state.phase !== "started" && !managed.canRetryFinish()) || !state.start?.result) throw state.cause ?? new Error("论文启动结果尚待核对");
    started = state.startStatus?.type === "started" ? state.startStatus.result : state.start.result;
  } else started = await agentPaperSubmissionGateway.start(pending.preview, mode);
  onStarted(started);
  if (managed) {
    if (pending.queuedInputId !== undefined && queue && agentPaperQueueGateway.inspectEdit) managed.configureQueue(queue, {inspectEdit: agentPaperQueueGateway.inspectEdit});
    await managed.finish();
    if (managed.getSnapshot().phase !== "complete") throw managed.getSnapshot().cause ?? new Error("论文提交收尾尚待核对");
    if (pending.queuedInputId !== undefined) onQueueRemoved();
  } else if (pending.queuedInputId !== undefined && queueState !== undefined) {
    const removed = await queue?.edit({type: "remove", inputId: pending.queuedInputId}, onQueueRemoved);
    if (!removed) throw new Error("队列消息未能移除，请刷新后重试。");
  }
}
