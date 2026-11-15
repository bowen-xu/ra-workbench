import type {AgentSubmissionPreview, AgentWritingPrepareRequest, AgentWritingStartResponse, AgentTurnOutcome, AgentWritingQueueResponse, AgentWorkspaceScope, AgentWorkspaceBuffer} from "@research-assistant/core/agent";
import type {WritingQueueCancellationSnapshot} from "./writingQueueCancellation";
import type {WritingQueuePreparationSnapshot} from "./writingQueuePreparation";
import type {WritingQueuedStartSnapshot} from "./writingQueuedStart";
import type {WritingPreparationSnapshot} from "./writingPreparation";

export interface WritingSubmissionSnapshot {
  readonly phase: "idle" | "preparing" | "preview" | "starting" | "started" | "canceling" | "error";
  readonly queue: AgentWritingQueueResponse | null;
  readonly request: AgentWritingPrepareRequest | null;
  readonly preview: AgentSubmissionPreview | null;
  readonly admission: AgentWritingStartResponse | null;
  readonly error: "prepare" | "workspace" | "start" | "startRejected" | "stop" | "cancel" | "recover" | null;
  readonly stopPending: boolean;
  readonly stopRequested: boolean;
  /** A user-requested stop belongs to this exact submission until resolved. */
  readonly cancelSendRequested?: boolean;
  readonly sendStopIntentUnavailable?: boolean;
  readonly sendStopFailed?: boolean;
  readonly sendStopNeedsReview?: boolean;
  readonly heldQueueInputId?: string | null;
  readonly queueStopHoldUnavailable?: boolean;
  readonly outcome: AgentTurnOutcome | null;
  readonly cancellation?: WritingQueueCancellationSnapshot;
  readonly preparation?: WritingQueuePreparationSnapshot;
  readonly workspaceFailure?: unknown;
  readonly startRecovery?: WritingQueuedStartSnapshot;
  readonly writingPreparation?: WritingPreparationSnapshot;
}
export const initialWritingSubmissionSnapshot: WritingSubmissionSnapshot = {
  phase: "idle", queue: null, request: null, preview: null, admission: null, error: null,
  stopPending: false, stopRequested: false, outcome: null,
};
export interface WritingSubmissionAdmission {
  readonly scope: AgentWorkspaceScope;
  readonly turnId: string;
}

export function writingPreparationSubmissionPatch(state: WritingPreparationSnapshot, previous: WritingSubmissionSnapshot): Partial<WritingSubmissionSnapshot> {
  const busy = ["saving", "sending", "checking", "clearing"].includes(state.phase);
  if (state.phase === "retired") return {...initialWritingSubmissionSnapshot, writingPreparation: state};
  if (state.start) return {
    writingPreparation: state, phase: state.start.result ? "started" : busy ? "starting" : "error",
    error: state.phase === "complete" || busy ? null : state.start.knownRejected ? "startRejected" : "start",
    request: state.recovered ? null : state.intent, preview: state.result,
    ...(state.start.result ? {admission: state.start.result, outcome: previous.outcome ?? state.start.result.outcome} : {}),
  };
  const ready = ["ready", "complete"].includes(state.phase);
  return {writingPreparation: state, phase: ready ? "preview" : busy ? "preparing" : "error", error: ready || busy ? null : "prepare",
    request: state.recovered ? null : state.intent, ...(ready && state.result ? {preview: state.result} : {})};
}

export function writingQueuedStartSubmissionPatch(state: WritingQueuedStartSnapshot): Partial<WritingSubmissionSnapshot> {
  const busy = ["saving", "sending", "checking", "clearing"].includes(state.phase);
  return {startRecovery: state, phase: state.result ? "started" : busy ? "starting" : "error",
    error: state.phase === "done" || busy ? null : state.knownRejected ? "startRejected" : "start",
    ...(state.preview ? {preview: state.preview} : {}), ...(state.recovered ? {request: null} : {})};
}

export function isOriginalWritingPreviewCurrent(current: AgentSubmissionPreview | null, preview: AgentSubmissionPreview): boolean {
  return !!current && current.requestId === preview.requestId && current.conversationId === preview.conversationId
    && current.manuscriptId === preview.manuscriptId && current.turnId === preview.turnId && current.contextSnapshotId === preview.contextSnapshotId;
}

export function writingSubmissionWorkspaceBuffers(requestId: string, queued: WritingQueuePreparationSnapshot | undefined,
  ordinary: WritingPreparationSnapshot): readonly AgentWorkspaceBuffer[] | null {
  if (queued?.intent?.submission.requestId === requestId) return structuredClone(queued.workspaceBuffers);
  return ordinary.intent?.requestId === requestId ? structuredClone(ordinary.workspaceBuffers) : null;
}
