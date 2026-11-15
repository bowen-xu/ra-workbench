import type {
  LlmModelSelection,
  PaperAiReadingAttachmentStatus,
  PaperAiReadingContextItem,
  PaperAiReadingImageRequest,
  PaperAiReadingMessage,
  PaperAiReadingPrepareResult,
  PaperAiReadingRegion,
  PaperAiReadingSessionDetail,
  PaperAiReadingSessionSummary,
  PaperAiReadingTurn,
} from "@research-assistant/core/ai";

type JsonRecord = Readonly<Record<string, unknown>>;

const PROVIDER_IDS = ["deepseek", "qwen", "openai"] as const;
const REASONING_EFFORTS = ["none", "low", "medium", "high", "xhigh", "max"] as const;
const CONTEXT_WINDOW_SOURCES = ["provider_runtime", "provider_catalog", "models_dev"] as const;

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

function boolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function optionalString(value: unknown): string | undefined | null {
  return value === undefined ? null : string(value);
}

function optionalInteger(value: unknown): number | undefined | null {
  return value === undefined ? null : integer(value);
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

function selection(value: unknown): LlmModelSelection | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const providerId = oneOf(data.providerId, PROVIDER_IDS);
  const modelId = string(data.modelId);
  const reasoningEffort = data.reasoningEffort === undefined
    ? null : oneOf(data.reasoningEffort, REASONING_EFFORTS);
  return providerId === undefined || modelId === undefined || reasoningEffort === undefined
    ? undefined
    : { providerId, modelId, ...(reasoningEffort === null ? {} : { reasoningEffort }) };
}

function region(value: unknown): PaperAiReadingRegion | undefined {
  const data = record(value);
  const rect = data === undefined ? undefined : record(data.rect);
  const pageIndex = data === undefined ? undefined : integer(data.pageIndex);
  if (rect === undefined || pageIndex === undefined) return undefined;
  const x = typeof rect.x === "number" ? rect.x : undefined;
  const y = typeof rect.y === "number" ? rect.y : undefined;
  const width = typeof rect.width === "number" ? rect.width : undefined;
  const height = typeof rect.height === "number" ? rect.height : undefined;
  return x === undefined || y === undefined || width === undefined || height === undefined
    ? undefined : { pageIndex, rect: { x, y, width, height } };
}

function contextItem(value: unknown): PaperAiReadingContextItem | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const id = string(data.id); const paperId = string(data.paperId);
  const attachmentId = string(data.attachmentId); const contentSha256 = string(data.contentSha256);
  const parseRunId = string(data.parseRunId); const rawRef = string(data.rawRef);
  const kind = oneOf(data.kind, ["text", "table", "picture", "group", "key_value", "form",
    "field", "field_region"] as const);
  const label = optionalString(data.label); const pageIndex = optionalInteger(data.pageIndex);
  const regions = list(data.regions, region); const sourceTextHash = optionalString(data.sourceTextHash);
  const text = optionalString(data.text);
  if (id === undefined || paperId === undefined || attachmentId === undefined
    || contentSha256 === undefined || parseRunId === undefined || rawRef === undefined
    || kind === undefined || label === undefined || pageIndex === undefined
    || regions === undefined || sourceTextHash === undefined || text === undefined) return undefined;
  const charspan = Array.isArray(data.charspan) && data.charspan.length === 2
    && Number.isSafeInteger(data.charspan[0]) && Number.isSafeInteger(data.charspan[1])
    ? [data.charspan[0] as number, data.charspan[1] as number] as const : undefined;
  return { id, paperId, attachmentId, contentSha256, parseRunId, rawRef, kind, regions,
    ...(label === null ? {} : { label }), ...(pageIndex === null ? {} : { pageIndex }),
    ...(charspan === undefined ? {} : { charspan }),
    ...(sourceTextHash === null ? {} : { sourceTextHash }),
    ...(text === null ? {} : { text }),
    ...(data.tableJson === undefined ? {} : { tableJson: data.tableJson }) };
}

function imageRequest(value: unknown): PaperAiReadingImageRequest | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const contextItemId = string(data.contextItemId); const attachmentId = string(data.attachmentId);
  const pageIndex = integer(data.pageIndex); const regions = list(data.regions, region);
  const label = optionalString(data.label);
  return contextItemId === undefined || attachmentId === undefined || pageIndex === undefined
    || regions === undefined || label === undefined ? undefined
    : { contextItemId, attachmentId, pageIndex, regions,
      ...(label === null ? {} : { label }) };
}

function attachmentStatus(value: unknown): PaperAiReadingAttachmentStatus | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const attachmentId = string(data.attachmentId); const displayName = string(data.displayName);
  const contentSha256 = string(data.contentSha256); const parseRunId = optionalString(data.parseRunId);
  const parseStatus = oneOf(data.parseStatus, ["available", "missing", "stale"] as const);
  return attachmentId === undefined || displayName === undefined || contentSha256 === undefined
    || parseRunId === undefined || parseStatus === undefined ? undefined
    : { attachmentId, displayName, contentSha256, parseStatus,
      ...(parseRunId === null ? {} : { parseRunId }) };
}

function message(value: unknown): PaperAiReadingMessage | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const id = string(data.id); const role = oneOf(data.role, ["user", "assistant"] as const);
  const content = string(data.content);
  return id === undefined || role === undefined || content === undefined
    ? undefined : { id, role, content };
}

function sessionSummary(value: unknown): PaperAiReadingSessionSummary | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const id = string(data.id); const paperId = string(data.paperId);
  const contextSnapshotId = optionalString(data.contextSnapshotId);
  const attachmentIds = list(data.attachmentIds, string); const parseRunIds = list(data.parseRunIds, string);
  const providerId = oneOf(data.providerId, PROVIDER_IDS); const modelId = string(data.modelId);
  const reasoningEffort = data.reasoningEffort === undefined
    ? null : oneOf(data.reasoningEffort, REASONING_EFFORTS);
  const bootstrapKind = oneOf(data.bootstrapKind, ["fresh_paper", "structured_history"] as const);
  const sourceWorkflowId = optionalString(data.sourceWorkflowId);
  const sourceWorkflowRevision = optionalInteger(data.sourceWorkflowRevision);
  const bootstrapHash = optionalString(data.bootstrapHash);
  const conceptRetrievalDefault = boolean(data.conceptRetrievalDefault);
  const title = string(data.title); const status = oneOf(data.status, ["active", "failed", "archived"] as const);
  const createdAt = string(data.createdAt); const updatedAt = string(data.updatedAt);
  const invalidBootstrap = bootstrapKind === "fresh_paper"
    ? sourceWorkflowId !== null || sourceWorkflowRevision !== null
    : sourceWorkflowId === null || sourceWorkflowRevision === null || bootstrapHash === null;
  if (id === undefined || paperId === undefined || contextSnapshotId === undefined
    || attachmentIds === undefined || parseRunIds === undefined || providerId === undefined
    || modelId === undefined || reasoningEffort === undefined || bootstrapKind === undefined
    || sourceWorkflowId === undefined || sourceWorkflowRevision === undefined
    || bootstrapHash === undefined || conceptRetrievalDefault === undefined
    || invalidBootstrap || title === undefined
    || status === undefined || createdAt === undefined || updatedAt === undefined) return undefined;
  return { id, paperId, attachmentIds, parseRunIds, providerId, modelId, bootstrapKind,
    conceptRetrievalDefault,
    title, status, createdAt, updatedAt,
    ...(contextSnapshotId === null ? {} : { contextSnapshotId }),
    ...(reasoningEffort === null ? {} : { reasoningEffort }),
    ...(sourceWorkflowId === null ? {} : { sourceWorkflowId }),
    ...(sourceWorkflowRevision === null ? {} : { sourceWorkflowRevision }),
    ...(bootstrapHash === null ? {} : { bootstrapHash }) };
}

function turn(value: unknown): PaperAiReadingTurn | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const id = string(data.id); const sessionId = string(data.sessionId); const ordinal = integer(data.ordinal);
  const userMessage = string(data.userMessage); const assistantMessage = string(data.assistantMessage);
  const messages = list(data.messages, message); const providerId = oneOf(data.providerId, PROVIDER_IDS);
  const modelId = string(data.modelId); const reasoningEffort = data.reasoningEffort === undefined
    ? null : oneOf(data.reasoningEffort, REASONING_EFFORTS);
  const contextWindowTokens = optionalInteger(data.contextWindowTokens);
  const contextWindowSource = data.contextWindowSource === undefined
    ? null : oneOf(data.contextWindowSource, CONTEXT_WINDOW_SOURCES);
  const status = oneOf(data.status, ["running", "succeeded", "failed", "canceled"] as const);
  const inputHash = optionalString(data.inputHash); const outputHash = optionalString(data.outputHash);
  const inputTokens = optionalInteger(data.inputTokens); const outputTokens = optionalInteger(data.outputTokens);
  const errorCode = optionalString(data.errorCode); const errorMessage = optionalString(data.errorMessage);
  const conceptRetrievalEnabled = boolean(data.conceptRetrievalEnabled);
  const conceptRetrievalTraceId = optionalString(data.conceptRetrievalTraceId);
  const createdAt = string(data.createdAt); const finishedAt = optionalString(data.finishedAt);
  if (id === undefined || sessionId === undefined || ordinal === undefined || userMessage === undefined
    || assistantMessage === undefined || messages === undefined || providerId === undefined
    || modelId === undefined || reasoningEffort === undefined || contextWindowTokens === undefined
    || contextWindowSource === undefined || status === undefined || inputHash === undefined
    || outputHash === undefined || inputTokens === undefined || outputTokens === undefined
    || errorCode === undefined || errorMessage === undefined
    || conceptRetrievalEnabled === undefined || conceptRetrievalTraceId === undefined
    || (conceptRetrievalEnabled !== (conceptRetrievalTraceId !== null)) || createdAt === undefined
    || finishedAt === undefined) return undefined;
  return { id, sessionId, ordinal, userMessage, assistantMessage, messages, providerId, modelId,
    status, conceptRetrievalEnabled, createdAt, ...(reasoningEffort === null ? {} : { reasoningEffort }),
    ...(contextWindowTokens === null ? {} : { contextWindowTokens }),
    ...(contextWindowSource === null ? {} : { contextWindowSource }),
    ...(inputHash === null ? {} : { inputHash }), ...(outputHash === null ? {} : { outputHash }),
    ...(inputTokens === null ? {} : { inputTokens }), ...(outputTokens === null ? {} : { outputTokens }),
    ...(errorCode === null ? {} : { errorCode }), ...(errorMessage === null ? {} : { errorMessage }),
    ...(conceptRetrievalTraceId === null ? {} : { conceptRetrievalTraceId }),
    ...(finishedAt === null ? {} : { finishedAt }) };
}

export function decodePaperAiReadingSessionDetail(
  value: unknown,
): PaperAiReadingSessionDetail | undefined {
  const summary = sessionSummary(value); const data = record(value);
  if (summary === undefined || data === undefined) return undefined;
  const turns = list(data.turns, turn);
  return turns === undefined ? undefined : { ...summary, turns };
}

export function decodePaperAiReadingPrepareResult(
  value: unknown,
): PaperAiReadingPrepareResult | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const session = data.session === undefined ? null : sessionSummary(data.session);
  const attachmentStatuses = list(data.attachmentStatuses, attachmentStatus);
  const contextItems = list(data.contextItems, contextItem);
  const imageRequests = list(data.imageRequests, imageRequest);
  const textItemCount = integer(data.textItemCount); const tableItemCount = integer(data.tableItemCount);
  const pictureItemCount = integer(data.pictureItemCount);
  const model = data.model === undefined ? null : selection(data.model);
  return session === undefined || attachmentStatuses === undefined || contextItems === undefined
    || imageRequests === undefined || textItemCount === undefined || tableItemCount === undefined
    || pictureItemCount === undefined || model === undefined ? undefined
    : { attachmentStatuses, contextItems, imageRequests, textItemCount, tableItemCount,
      pictureItemCount, ...(session === null ? {} : { session }),
      ...(model === null ? {} : { model }) };
}

export function decodePaperAiReadingSessionList(
  value: unknown,
): readonly PaperAiReadingSessionSummary[] | undefined {
  return list(value, sessionSummary);
}
