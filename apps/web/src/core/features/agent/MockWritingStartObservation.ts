import {applyAgentEvents, createAgentConversationProjection, decodeAgentWritingStartResponse,
  type AgentSubmissionPreview, type AgentWritingStartRequest, type AgentWritingStartResponse, type AgentWritingStartObservation} from "@research-assistant/core/agent";
import type {AgentSessionGateway} from "@research-assistant/data/agent";

export function mockWritingStartUnconfirmed(requestId: string, safeCause: string): AgentWritingStartObservation {
  return {type: "unconfirmed", detail: {operationId: requestId, epoch: null, sqlFence: null, phase: "writing-start-status",
    sqliteCode: null, sqliteExtendedCode: null, safeCause}};
}
/** A grant identifies its selected resource, never the historical writing host.
 * Match Native's ordinary/manual read gate without consulting current binding. */
export function mockWritingStartHasHistoricalManuscript(preview: AgentSubmissionPreview, manuscriptId: string): boolean {
  return preview.input.context.some(source => {
    if ((source.kind !== "manuscript" && source.kind !== "retrieval") || source.anchor === null) return false;
    try {
      const anchor: unknown = JSON.parse(source.anchor);
      return !!anchor && typeof anchor === "object" && !Array.isArray(anchor)
        && !("supplementalContext" in anchor && anchor.supplementalContext === true)
        && "manuscriptId" in anchor && anchor.manuscriptId === manuscriptId;
    } catch {return false;}
  });
}
/** Mock storage read only: validate the real journal prefix while retaining an
 * original start/terminal even after rollback removes its visible turn. */
export async function observeMockWritingStart(history: AgentSessionGateway,
  request: Pick<AgentWritingStartRequest, "conversationId" | "requestId" | "contextSnapshotId" | "model">,
  preview: AgentSubmissionPreview | null, expectedKind: "regular" | "compact"): Promise<AgentWritingStartResponse | null> {
  let projection = createAgentConversationProjection(request.conversationId), watermark: number | null = null;
  let started = false, outcome: AgentWritingStartResponse["outcome"] = null;
  do {
    const before = projection.seq, page = await history.readHistory({conversationId: request.conversationId, afterSeq: before,
      limit: Math.min(200, watermark === null ? 200 : watermark - before)});
    watermark ??= page.watermark;
    if (page.watermark < watermark) throw new Error("mock_writing_start_history_prefix_invalid");
    for (const event of page.events.filter(event => event.seq <= watermark!)) {
      projection = applyAgentEvents(projection, [event]);
      if (!preview && event.requestId === request.requestId) throw new Error("mock_writing_start_submission_missing");
      const payload = event.payload;
      if (payload.type === "turnStarted" && (payload.turnId === preview?.turnId || event.requestId === request.requestId)) {
        if (!preview || started || payload.turnId !== preview.turnId || event.requestId !== request.requestId
          || payload.contextSnapshotId !== request.contextSnapshotId || JSON.stringify(payload.model) !== JSON.stringify(request.model)
          || (payload.taskKind ?? "regular") !== expectedKind) throw new Error("mock_writing_start_history_identity_mismatch");
        started = true;
      }
      if (payload.type === "turnFinished" && payload.turnId === preview?.turnId) {
        if (!started || outcome !== null || event.requestId !== request.requestId) throw new Error("mock_writing_start_history_identity_mismatch");
        outcome = payload.outcome;
      }
    }
    if (projection.seq === watermark) break;
    if (projection.seq === before) throw new Error("mock_writing_start_history_prefix_invalid");
  } while (true);
  return !started || !preview ? null : decodeAgentWritingStartResponse({schemaVersion: 1, conversationId: request.conversationId,
    requestId: request.requestId, turnId: preview.turnId, alreadyStarted: true, outcome});
}
