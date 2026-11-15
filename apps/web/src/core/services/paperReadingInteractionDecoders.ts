import type {
  PaperAiReadingCancelResult,
  PaperAiReadingMessage,
  PaperAiReadingSteerResult,
  PaperAiReadingTurnDebugData,
  PaperReadingBootstrapOptions,
} from "@research-assistant/core/ai";

type JsonRecord = Readonly<Record<string, unknown>>;

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

function oneOf<const Values extends readonly string[]>(
  value: unknown,
  values: Values,
): Values[number] | undefined {
  return typeof value === "string" && values.includes(value) ? value as Values[number] : undefined;
}

function list<T>(value: unknown, decode: (item: unknown) => T | undefined): readonly T[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const decoded = value.map(decode);
  return decoded.some((item) => item === undefined) ? undefined : decoded as readonly T[];
}

function message(value: unknown): PaperAiReadingMessage | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const id = string(data.id);
  const role = oneOf(data.role, ["user", "assistant"] as const);
  const content = string(data.content);
  return id === undefined || role === undefined || content === undefined
    ? undefined : { id, role, content };
}

export function decodePaperAiReadingSteerResult(
  value: unknown,
): PaperAiReadingSteerResult | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const status = oneOf(data.status, ["accepted", "notReady", "unsupported", "notActive"] as const);
  const resultMessage = data.message === undefined ? null : message(data.message);
  if (status === undefined || resultMessage === undefined) return undefined;
  if ((status === "accepted") !== (resultMessage !== null)) return undefined;
  return { status, ...(resultMessage === null ? {} : { message: resultMessage }) };
}

export function decodePaperAiReadingCancelResult(
  value: unknown,
): PaperAiReadingCancelResult | undefined {
  const data = record(value);
  if (data === undefined || typeof data.accepted !== "boolean") return undefined;
  const resubmitMessages = list(data.resubmitMessages, message);
  return resubmitMessages === undefined ? undefined : { accepted: data.accepted, resubmitMessages };
}

export function decodePaperAiReadingTurnDebugData(
  value: unknown,
): PaperAiReadingTurnDebugData | undefined {
  const data = record(value);
  const turnId = data === undefined ? undefined : string(data.turnId);
  if (data === undefined || turnId === undefined
    || !("rawRequest" in data) || !("rawResponse" in data)) return undefined;
  return { turnId, rawRequest: data.rawRequest ?? null, rawResponse: data.rawResponse ?? null };
}

export function decodePaperReadingBootstrapOptions(
  value: unknown,
): PaperReadingBootstrapOptions | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const defaultKind = oneOf(data.defaultKind, ["fresh_paper", "structured_history"] as const);
  const unavailableReason = data.unavailableReason === undefined
    ? null : string(data.unavailableReason);
  const history = data.structuredHistory === undefined ? null : record(data.structuredHistory);
  if (defaultKind === undefined || unavailableReason === undefined || history === undefined) return undefined;
  if (history === null) {
    return defaultKind === "fresh_paper"
      ? { defaultKind, ...(unavailableReason === null ? {} : { unavailableReason }) }
      : undefined;
  }
  const workflowId = string(history.workflowId);
  const workflowRevision = integer(history.workflowRevision);
  const workflowStatus = oneOf(history.workflowStatus, ["preparing", "overview_running",
    "overview_ready", "concepts_running", "needs_review", "partial", "failed", "canceled",
    "superseded"] as const);
  const providerId = oneOf(history.providerId, ["deepseek", "qwen", "openai"] as const);
  const modelId = string(history.modelId);
  const completedTurns = list(history.completedTurns, (item) =>
    oneOf(item, ["paper_overview", "paper_concept_map"] as const));
  const attachmentCount = integer(history.attachmentCount);
  const updatedAt = string(history.updatedAt);
  if (workflowId === undefined || workflowRevision === undefined || workflowStatus === undefined
    || providerId === undefined || modelId === undefined || completedTurns === undefined
    || attachmentCount === undefined || updatedAt === undefined || unavailableReason !== null
    || workflowRevision < 1 || attachmentCount < 1
    || !completedTurns.includes("paper_overview")
    || new Set(completedTurns).size !== completedTurns.length) {
    return undefined;
  }
  return { defaultKind, structuredHistory: { workflowId, workflowRevision, workflowStatus,
    providerId, modelId, completedTurns, attachmentCount, updatedAt } };
}
