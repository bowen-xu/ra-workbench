import type {
  LlmReasoningEffort,
  PaperAnalysisAttempt,
  PaperAnalysisMessage,
  PaperAnalysisTurn,
  PaperAnalysisWorkflow,
} from "@research-assistant/core/ai";

type JsonRecord = Readonly<Record<string, unknown>>;

const WORKFLOW_STATUSES = [
  "preparing", "overview_running", "overview_ready", "concepts_running",
  "needs_review", "partial", "failed", "canceled", "superseded",
] as const;
const TURN_KINDS = ["paper_overview", "paper_concept_map"] as const;
const TURN_STATUSES = [
  "queued", "running", "succeeded", "failed", "canceled", "interrupted",
] as const;
const MESSAGE_KINDS = [
  "system_context", "task_prompt", "assistant_candidate", "transition",
] as const;
const REASONING_EFFORTS = ["none", "low", "medium", "high", "xhigh", "max"] as const;

function record(value: unknown): JsonRecord | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonRecord : undefined;
}
function string(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}
function integer(value: unknown): number | undefined {
  return Number.isSafeInteger(value) ? value as number : undefined;
}
function optionalString(value: unknown): string | undefined | null {
  return value === undefined ? null : string(value);
}
function oneOf<const Values extends readonly string[]>(
  value: unknown,
  values: Values,
): Values[number] | undefined {
  return typeof value === "string" && values.includes(value) ? value as Values[number] : undefined;
}
function list<T>(value: unknown, decode: (item: unknown) => T | undefined): readonly T[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const result: T[] = [];
  for (const item of value) {
    const decoded = decode(item);
    if (decoded === undefined) return undefined;
    result.push(decoded);
  }
  return result;
}

function message(value: unknown): PaperAnalysisMessage | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const kind = oneOf(data.kind, MESSAGE_KINDS); const content = string(data.content);
  const sourceTurnId = optionalString(data.sourceTurnId);
  return kind === undefined || content === undefined || sourceTurnId === undefined
    ? undefined : { kind, content, ...(sourceTurnId === null ? {} : { sourceTurnId }) };
}

function optionalEffort(value: unknown): LlmReasoningEffort | null | undefined {
  return value === undefined ? null : oneOf(value, REASONING_EFFORTS);
}

function attempt(value: unknown): PaperAnalysisAttempt | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const turnId = string(data.turnId); const sequence = integer(data.attempt);
  const sharedConversationId = optionalString(data.sharedConversationId);
  const sharedRequestId = optionalString(data.sharedRequestId); const sharedTurnId = optionalString(data.sharedTurnId);
  const sharedTurnOutcome = data.sharedTurnOutcome === undefined ? null
    : oneOf(data.sharedTurnOutcome, ["completed", "failed", "canceled", "interrupted"] as const);
  const sharedAiRunId = optionalString(data.sharedAiRunId); const sharedCandidateId = optionalString(data.sharedCandidateId);
  const sharedTerminalStatus = optionalString(data.sharedTerminalStatus);
  const status = oneOf(data.status, TURN_STATUSES); const normalizedMessages = list(data.normalizedMessages, message);
  const providerId = string(data.providerId); const modelId = string(data.modelId);
  const reasoningEffort = optionalEffort(data.reasoningEffort); const aiRunId = optionalString(data.aiRunId);
  const candidateId = optionalString(data.candidateId); const inputHash = optionalString(data.inputHash);
  const outputHash = optionalString(data.outputHash); const inputTokens = optionalInteger(data.inputTokens);
  const outputTokens = optionalInteger(data.outputTokens); const errorCode = optionalString(data.errorCode);
  const errorMessage = optionalString(data.errorMessage); const createdAt = string(data.createdAt);
  const startedAt = optionalString(data.startedAt); const finishedAt = optionalString(data.finishedAt);
  const recoveryReason = optionalString(data.recoveryReason);
  const recoveryVersion = optionalInteger(data.recoveryVersion);
  const recoveredAt = optionalString(data.recoveredAt);
  if (id === undefined || turnId === undefined || sequence === undefined || sequence < 1
    || status === undefined || normalizedMessages === undefined || providerId === undefined
    || modelId === undefined || reasoningEffort === undefined || aiRunId === undefined
    || sharedConversationId === undefined || sharedRequestId === undefined || sharedTurnId === undefined
    || sharedTurnOutcome === undefined
    || sharedAiRunId === undefined || sharedCandidateId === undefined || sharedTerminalStatus === undefined
    || candidateId === undefined || inputHash === undefined || outputHash === undefined
    || inputTokens === undefined || outputTokens === undefined || errorCode === undefined
    || errorMessage === undefined || createdAt === undefined || startedAt === undefined
    || finishedAt === undefined || recoveryReason === undefined || recoveryVersion === undefined
    || recoveredAt === undefined) return undefined;
  return { id, turnId, attempt: sequence, status, normalizedMessages, providerId, modelId, createdAt,
    ...(sharedConversationId === null ? {} : { sharedConversationId }),
    ...(sharedRequestId === null ? {} : { sharedRequestId }), ...(sharedTurnId === null ? {} : { sharedTurnId }),
    ...(sharedTurnOutcome === null ? {} : { sharedTurnOutcome }),
    ...(sharedAiRunId === null ? {} : { sharedAiRunId }), ...(sharedCandidateId === null ? {} : { sharedCandidateId }),
    ...(sharedTerminalStatus === null ? {} : { sharedTerminalStatus }),
    ...(reasoningEffort === null ? {} : { reasoningEffort }), ...(aiRunId === null ? {} : { aiRunId }),
    ...(candidateId === null ? {} : { candidateId }), ...(inputHash === null ? {} : { inputHash }),
    ...(outputHash === null ? {} : { outputHash }), ...(inputTokens === null ? {} : { inputTokens }),
    ...(outputTokens === null ? {} : { outputTokens }), ...(errorCode === null ? {} : { errorCode }),
    ...(errorMessage === null ? {} : { errorMessage }), ...(startedAt === null ? {} : { startedAt }),
    ...(finishedAt === null ? {} : { finishedAt }),
    ...(recoveryReason === null ? {} : { recoveryReason }),
    ...(recoveryVersion === null ? {} : { recoveryVersion }),
    ...(recoveredAt === null ? {} : { recoveredAt }) };
}

function optionalInteger(value: unknown): number | null | undefined {
  return value === undefined ? null : integer(value);
}

function turn(value: unknown): PaperAnalysisTurn | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const workflowId = string(data.workflowId); const ordinal = integer(data.ordinal);
  const kind = oneOf(data.kind, TURN_KINDS); const status = oneOf(data.status, TURN_STATUSES);
  const candidateId = optionalString(data.candidateId); const createdAt = string(data.createdAt);
  const finishedAt = optionalString(data.finishedAt); const attempts = list(data.attempts, attempt);
  if (id === undefined || workflowId === undefined || ordinal === undefined || kind === undefined
    || status === undefined || candidateId === undefined || createdAt === undefined
    || finishedAt === undefined || attempts === undefined) return undefined;
  return { id, workflowId, ordinal, kind, status, createdAt, attempts,
    ...(candidateId === null ? {} : { candidateId }), ...(finishedAt === null ? {} : { finishedAt }) };
}

export function decodePaperAnalysisWorkflow(value: unknown): PaperAnalysisWorkflow | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const paperId = string(data.paperId); const snapshotId = string(data.contextSnapshotId);
  const activeClientRequestId = optionalString(data.activeClientRequestId);
  const conversationId = optionalString(data.conversationId);
  const status = oneOf(data.status, WORKFLOW_STATUSES); const providerId = string(data.providerId);
  const modelId = string(data.modelId); const reasoningEffort = optionalEffort(data.reasoningEffort);
  const promptSetVersion = string(data.promptSetVersion); const activeTurnKind = data.activeTurnKind === undefined
    ? null : oneOf(data.activeTurnKind, TURN_KINDS); const revision = integer(data.revision);
  const supersedesWorkflowId = optionalString(data.supersedesWorkflowId); const createdAt = string(data.createdAt);
  const updatedAt = string(data.updatedAt); const completedAt = optionalString(data.completedAt);
  const turns = list(data.turns, turn);
  if (id === undefined || paperId === undefined || snapshotId === undefined || status === undefined
    || providerId === undefined || modelId === undefined || reasoningEffort === undefined
    || promptSetVersion === undefined || activeTurnKind === undefined || revision === undefined
    || revision < 1 || supersedesWorkflowId === undefined || createdAt === undefined
    || updatedAt === undefined || completedAt === undefined || turns === undefined
    || activeClientRequestId === undefined || conversationId === undefined || !validWorkflowTurns(turns)) return undefined;
  return { id, paperId, contextSnapshotId: snapshotId, status, providerId, modelId,
    promptSetVersion, revision, createdAt, updatedAt, turns,
    ...(conversationId === null ? {} : { conversationId }),
    ...(activeClientRequestId === null ? {} : { activeClientRequestId }),
    ...(reasoningEffort === null ? {} : { reasoningEffort }),
    ...(activeTurnKind === null ? {} : { activeTurnKind }),
    ...(supersedesWorkflowId === null ? {} : { supersedesWorkflowId }),
    ...(completedAt === null ? {} : { completedAt }) };
}

function validWorkflowTurns(turns: readonly PaperAnalysisTurn[]): boolean {
  // New runs contain only structured reading. Retain the historical two-stage
  // contract without accepting empty, duplicated or reordered stages.
  return (turns.length === 1 || turns.length === 2)
    && turns[0]?.kind === "paper_overview" && turns[0].ordinal === 0
    && (turns.length === 1 || turns[1]?.kind === "paper_concept_map" && turns[1].ordinal === 1);
}
