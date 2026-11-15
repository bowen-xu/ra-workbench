import {
  AI_SUGGESTION_CANDIDATE_KINDS,
  decodePaperTemplateSnapshot, paperTemplateRevision,
  AI_RUN_STATUSES,
  AI_STAGE_KINDS,
  AI_TASK_KINDS,
  PROFILED_OVERVIEW_FIELDS,
  type AiCredentialSummary,
  type AiEvidencePreview,
  type AiProviderCapabilities,
  type AiProviderOverview,
  type AiProviderSettings,
  type AiProviderTestResult,
  type AiOverviewRetryContext,
  type AiOverviewProfileContext,
  type AiMatrixSynthesisContext,
  type AiRun,
  type AiRunStage,
  type AiScopeItem,
  type AiTaskPreview,
  type LlmConfiguration,
  type ChatGptAccountStatus,
  type ChatGptLoginStart,
  type LlmModelDescriptor,
  type LlmModelRoute,
  type LlmModelSelection,
  type LlmProviderConfig,
  type LlmProviderModels,
} from "@research-assistant/core/ai";

export { decodePaperAnalysisWorkflow } from "./paperAnalysisDecoders";
export {
  decodePaperAiReadingCancelResult,
  decodePaperAiReadingSteerResult,
  decodePaperAiReadingTurnDebugData,
} from "./paperReadingInteractionDecoders";
export {
  decodePaperAiReadingPrepareResult,
  decodePaperAiReadingSessionDetail,
  decodePaperAiReadingSessionList,
} from "./paperReadingSessionDecoders";

type JsonRecord = Readonly<Record<string, unknown>>;

function record(value: unknown): JsonRecord | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonRecord : undefined;
}
function string(value: unknown): string | undefined { return typeof value === "string" ? value : undefined; }
function boolean(value: unknown): boolean | undefined { return typeof value === "boolean" ? value : undefined; }
function integer(value: unknown): number | undefined { return Number.isSafeInteger(value) ? value as number : undefined; }
function optionalString(value: unknown): string | undefined | null { return value === undefined ? null : string(value); }
function optionalInteger(value: unknown): number | undefined | null { return value === undefined ? null : integer(value); }
function oneOf<const Values extends readonly string[]>(value: unknown, values: Values): Values[number] | undefined {
  return typeof value === "string" && values.includes(value) ? value as Values[number] : undefined;
}
function optionalSuggestionKind(value: unknown) {
  return value === undefined ? null : oneOf(value, AI_SUGGESTION_CANDIDATE_KINDS);
}

export function decodeList<T>(value: unknown, decode: (item: unknown) => T | undefined): readonly T[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const result: T[] = [];
  for (const item of value) { const decoded = decode(item); if (decoded === undefined) return undefined; result.push(decoded); }
  return result;
}

export function decodeAiCredentialSummary(value: unknown): AiCredentialSummary | undefined {
  const data = record(value); if (data === undefined) return undefined;
  if ("apiKey" in data) return undefined;
  const providerId = oneOf(data.providerId, ["deepseek", "qwen", "qwen-embedding"] as const);
  const configured = boolean(data.configured); const maskedValue = optionalString(data.maskedValue);
  if (providerId === undefined || configured === undefined || maskedValue === undefined) return undefined;
  return { providerId, configured, ...(maskedValue === null ? {} : { maskedValue }) };
}

const LLM_PROVIDER_IDS = ["deepseek", "qwen", "openai"] as const;
const LLM_MODEL_KINDS = ["text", "embedding", "vision"] as const;
const LLM_REASONING_EFFORTS = ["none", "low", "medium", "high", "xhigh", "max"] as const;
const LLM_CONTEXT_WINDOW_SOURCES = ["provider_runtime", "provider_catalog", "models_dev"] as const;

function decodeLlmSelection(value: unknown): LlmModelSelection | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const providerId = oneOf(data.providerId, LLM_PROVIDER_IDS); const modelId = string(data.modelId);
  const reasoningEffort = data.reasoningEffort === undefined ? null : oneOf(data.reasoningEffort, LLM_REASONING_EFFORTS);
  return providerId === undefined || modelId === undefined || reasoningEffort === undefined ? undefined
    : { providerId, modelId, ...(reasoningEffort === null ? {} : { reasoningEffort }) };
}

function decodeLlmRoute(value: unknown): LlmModelRoute | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const routeKind = oneOf(data.routeKind, ["default", "use_case"] as const);
  const routeId = string(data.routeId); const label = string(data.label);
  const modelKind = oneOf(data.modelKind, LLM_MODEL_KINDS);
  const selection = data.selection === null ? null : decodeLlmSelection(data.selection);
  if (routeKind === undefined || routeId === undefined || label === undefined || modelKind === undefined
    || selection === undefined) return undefined;
  return { routeKind, routeId, label, modelKind, ...(selection === null ? {} : { selection }) };
}

function decodeLlmProvider(value: unknown): LlmProviderConfig | undefined {
  const data = record(value); if (data === undefined || "apiKey" in data) return undefined;
  const providerId = oneOf(data.providerId, LLM_PROVIDER_IDS); const displayName = string(data.displayName);
  const authMode = oneOf(data.authMode, ["api-key", "chatgpt-login"] as const);
  const protocol = oneOf(data.protocol, ["openai-compatible", "codex-cli"] as const);
  const modelKinds = decodeList(data.modelKinds, (item) => oneOf(item, LLM_MODEL_KINDS));
  const defaultBaseUrls = decodeList(data.defaultBaseUrls, string); const baseUrl = string(data.baseUrl);
  const usesDefaultBaseUrl = boolean(data.usesDefaultBaseUrl); const enabled = boolean(data.enabled);
  const timeoutMs = integer(data.timeoutMs); const credentialConfigured = boolean(data.credentialConfigured);
  const credentialMask = optionalString(data.credentialMask);
  if (providerId === undefined || displayName === undefined || authMode === undefined || protocol === undefined || modelKinds === undefined
    || defaultBaseUrls === undefined || baseUrl === undefined || usesDefaultBaseUrl === undefined
    || enabled === undefined || timeoutMs === undefined || credentialConfigured === undefined
    || credentialMask === undefined) return undefined;
  return { providerId, displayName, authMode, protocol, modelKinds, defaultBaseUrls, baseUrl,
    usesDefaultBaseUrl, enabled, timeoutMs, credentialConfigured,
    ...(credentialMask === null ? {} : { credentialMask }) };
}

export function decodeLlmConfiguration(value: unknown): LlmConfiguration | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const providers = decodeList(data.providers, decodeLlmProvider);
  const defaultModels = decodeList(data.defaultModels, decodeLlmRoute);
  const useCases = decodeList(data.useCases, decodeLlmRoute);
  return providers === undefined || defaultModels === undefined || useCases === undefined
    ? undefined : { providers, defaultModels, useCases };
}

export function decodeLlmProviderModels(value: unknown): LlmProviderModels | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const providerId = oneOf(data.providerId, LLM_PROVIDER_IDS);
  const models = decodeList(data.models, decodeLlmModelDescriptor);
  const status = data.catalog === undefined ? undefined : record(data.catalog);
  const source = status && oneOf(status.source, ["remote", "cache", "bundled"] as const);
  const stale = status && boolean(status.stale), fetchedAt = status && optionalInteger(status.fetchedAt);
  if (data.catalog !== undefined && (!status || source === undefined || stale === undefined || fetchedAt == null || fetchedAt < 0)) return undefined;
  return providerId === undefined || models === undefined ? undefined : { providerId, models,
    ...(status ? {catalog: {source: source!, stale: stale!, fetchedAt: fetchedAt!}} : {}) };
}

function decodeLlmModelDescriptor(value: unknown): LlmModelDescriptor | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const displayName = string(data.displayName);
  const description = string(data.description); const isDefault = boolean(data.isDefault);
  const supportsFastMode = data.supportsFastMode === undefined ? null : boolean(data.supportsFastMode);
  const defaultReasoningEffort = data.defaultReasoningEffort === undefined
    ? null : oneOf(data.defaultReasoningEffort, LLM_REASONING_EFFORTS);
  const supportedReasoningEfforts = decodeList(data.supportedReasoningEfforts,
    (item) => oneOf(item, LLM_REASONING_EFFORTS));
  const contextWindowTokens = optionalInteger(data.contextWindowTokens);
  const contextWindowSource = data.contextWindowSource === undefined
    ? null : oneOf(data.contextWindowSource, LLM_CONTEXT_WINDOW_SOURCES);
  return id === undefined || displayName === undefined || description === undefined
    || isDefault === undefined || supportsFastMode === undefined || defaultReasoningEffort === undefined
    || supportedReasoningEfforts === undefined || contextWindowTokens === undefined
    || contextWindowSource === undefined ? undefined : {
      id, displayName, description, isDefault, supportedReasoningEfforts,
      ...(supportsFastMode === null ? {} : { supportsFastMode }),
      ...(defaultReasoningEffort === null ? {} : { defaultReasoningEffort }),
      ...(contextWindowTokens === null ? {} : { contextWindowTokens }),
      ...(contextWindowSource === null ? {} : { contextWindowSource }),
    };
}

export function decodeChatGptAccountStatus(value: unknown): ChatGptAccountStatus | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const runtimeAvailable = boolean(data.runtimeAvailable); const configured = boolean(data.configured);
  const authType = string(data.authType); const email = optionalString(data.email);
  const planType = optionalString(data.planType);
  return runtimeAvailable === undefined || configured === undefined || authType === undefined
    || email === undefined || planType === undefined ? undefined : {
      runtimeAvailable, configured, authType,
      ...(email === null ? {} : { email }), ...(planType === null ? {} : { planType }),
    };
}

export function decodeChatGptLoginStart(value: unknown): ChatGptLoginStart | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const loginId = string(data.loginId); const authUrl = string(data.authUrl);
  return loginId === undefined || authUrl === undefined ? undefined : { loginId, authUrl };
}

export function decodeAiProviderSettings(value: unknown): AiProviderSettings | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const providerId = oneOf(data.providerId, ["fixture-generation", "deepseek", "qwen-embedding"] as const);
  const providerKind = oneOf(data.providerKind, ["generation", "embedding"] as const);
  const thinkingMode = oneOf(data.thinkingMode, ["disabled", "enabled"] as const);
  const enabled = boolean(data.enabled); const baseUrl = string(data.baseUrl); const region = string(data.region);
  const modelId = string(data.modelId); const timeoutMs = integer(data.timeoutMs); const isCloud = boolean(data.isCloud);
  const credentialConfigured = boolean(data.credentialConfigured); const credentialMask = optionalString(data.credentialMask);
  const updatedAt = string(data.updatedAt);
  if (providerId === undefined || providerKind === undefined || thinkingMode === undefined || enabled === undefined
    || baseUrl === undefined || region === undefined || modelId === undefined || timeoutMs === undefined
    || isCloud === undefined || credentialConfigured === undefined || credentialMask === undefined || updatedAt === undefined) return undefined;
  return { providerId, providerKind, thinkingMode, enabled, baseUrl, region, modelId, timeoutMs, isCloud,
    credentialConfigured, updatedAt, ...(credentialMask === null ? {} : { credentialMask }) };
}

function decodeCapabilities(value: unknown): AiProviderCapabilities | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const providerId = string(data.providerId); const providerKind = oneOf(data.providerKind, ["generation", "embedding"] as const);
  const isCloud = boolean(data.isCloud); const supportsStructuredOutput = boolean(data.supportsStructuredOutput);
  const supportsStreaming = boolean(data.supportsStreaming); const supportsCancellation = boolean(data.supportsCancellation);
  const supportsUsage = boolean(data.supportsUsage);
  const supportsVisionInput = data.supportsVisionInput === undefined ? null : boolean(data.supportsVisionInput);
  if (providerId === undefined || providerKind === undefined || isCloud === undefined || supportsStructuredOutput === undefined
    || supportsStreaming === undefined || supportsCancellation === undefined || supportsUsage === undefined
    || supportsVisionInput === undefined) return undefined;
  return { providerId, providerKind, isCloud, supportsStructuredOutput, supportsStreaming, supportsCancellation,
    supportsUsage, ...(supportsVisionInput === null ? {} : { supportsVisionInput }) };
}

export function decodeAiProviderOverview(value: unknown): AiProviderOverview | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const settings = decodeAiProviderSettings(data.settings); const capabilities = decodeCapabilities(data.capabilities);
  return settings === undefined || capabilities === undefined ? undefined : { settings, capabilities };
}

export function decodeAiProviderTestResult(value: unknown): AiProviderTestResult | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const providerId = string(data.providerId); const modelId = string(data.modelId);
  const reachable = boolean(data.reachable); const message = string(data.message);
  return providerId === undefined || modelId === undefined || reachable === undefined || message === undefined
    ? undefined : { providerId, modelId, reachable, message };
}

function decodeScopeItem(value: unknown): AiScopeItem | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const ordinal = integer(data.ordinal); const itemKind = oneOf(data.itemKind, ["paper", "attachment", "note", "anchor"] as const);
  const objectId = string(data.objectId); const parentObjectId = optionalString(data.parentObjectId); const contentSha256 = optionalString(data.contentSha256);
  const objectRevision = optionalInteger(data.objectRevision); const snapshotHash = string(data.snapshotHash); const createdAt = string(data.createdAt);
  if (id === undefined || ordinal === undefined || itemKind === undefined || objectId === undefined || parentObjectId === undefined
    || contentSha256 === undefined || objectRevision === undefined || snapshotHash === undefined || createdAt === undefined) return undefined;
  return { id, ordinal, itemKind, objectId, snapshotHash, createdAt, ...(parentObjectId === null ? {} : { parentObjectId }),
    ...(contentSha256 === null ? {} : { contentSha256 }), ...(objectRevision === null ? {} : { objectRevision }) };
}

function decodeEvidencePreview(value: unknown): AiEvidencePreview | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const citationKey = string(data.citationKey); const anchorId = string(data.anchorId);
  const paperTitle = string(data.paperTitle); const attachmentId = string(data.attachmentId);
  const contentSha256 = string(data.contentSha256); const pageIndex = integer(data.pageIndex);
  const pageLabel = optionalString(data.pageLabel); const quoteExact = string(data.quoteExact);
  const contextText = string(data.contextText);
  const relationKind = oneOf(data.relationKind, ["direct_quote", "context"] as const);
  if (citationKey === undefined || anchorId === undefined || paperTitle === undefined
    || attachmentId === undefined || contentSha256 === undefined || pageIndex === undefined
    || pageLabel === undefined || quoteExact === undefined || contextText === undefined
    || relationKind === undefined) return undefined;
  return { citationKey, anchorId, paperTitle, attachmentId, contentSha256, pageIndex,
    quoteExact, contextText, relationKind, ...(pageLabel === null ? {} : { pageLabel }) };
}

export function decodeAiTaskPreview(value: unknown): AiTaskPreview | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const taskKind = oneOf(data.taskKind, AI_TASK_KINDS); const providerId = string(data.providerId); const modelId = string(data.modelId);
  const reasoningEffort = data.reasoningEffort === undefined ? null : oneOf(data.reasoningEffort, LLM_REASONING_EFFORTS);
  const isCloud = boolean(data.isCloud); const scopeHash = string(data.scopeHash); const inputItemCount = integer(data.inputItemCount);
  const scopeItems = decodeList(data.scopeItems, decodeScopeItem); const requiresCloudConsent = boolean(data.requiresCloudConsent);
  const evidencePreview = decodeList(data.evidencePreview, decodeEvidencePreview);
  const overviewProfile = decodeOverviewProfile(data.overviewProfile);
  const suggestionCandidateKind = optionalSuggestionKind(data.suggestionCandidateKind);
  const matrixSynthesisContext = decodeMatrixSynthesisContext(data.matrixSynthesisContext);
  return taskKind === undefined || providerId === undefined || modelId === undefined || reasoningEffort === undefined || isCloud === undefined || scopeHash === undefined
    || inputItemCount === undefined || scopeItems === undefined || requiresCloudConsent === undefined
    || evidencePreview === undefined || overviewProfile === undefined
    || suggestionCandidateKind === undefined || matrixSynthesisContext === undefined
    || overviewProfile !== null && taskKind !== "paper_overview"
    || (taskKind === "paper_suggestions") !== (suggestionCandidateKind !== null)
    || (taskKind === "matrix_synthesis") !== (matrixSynthesisContext !== null) ? undefined
    : { taskKind, providerId, modelId, isCloud, scopeHash, inputItemCount, scopeItems,
      ...(reasoningEffort === null ? {} : { reasoningEffort }),
      requiresCloudConsent, evidencePreview,
      ...(overviewProfile === null ? {} : { overviewProfile }),
      ...(suggestionCandidateKind === null ? {} : { suggestionCandidateKind }),
      ...(matrixSynthesisContext === null ? {} : { matrixSynthesisContext }) };
}

function decodeStage(value: unknown): AiRunStage | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const runId = string(data.runId); const stageKind = oneOf(data.stageKind, AI_STAGE_KINDS);
  const attempt = integer(data.attempt); const status = oneOf(data.status, ["queued", "running", "succeeded", "failed", "canceled", "discarded"] as const);
  const inputHash = optionalString(data.inputHash); const outputHash = optionalString(data.outputHash);
  const inputTokens = optionalInteger(data.inputTokens); const outputTokens = optionalInteger(data.outputTokens);
  const durationMs = optionalInteger(data.durationMs); const errorCode = optionalString(data.errorCode);
  const errorMessage = optionalString(data.errorMessage); const startedAt = optionalString(data.startedAt);
  const finishedAt = optionalString(data.finishedAt); const createdAt = string(data.createdAt);
  if (id === undefined || runId === undefined || stageKind === undefined || attempt === undefined
    || status === undefined || inputHash === undefined || outputHash === undefined
    || inputTokens === undefined || outputTokens === undefined || durationMs === undefined
    || errorCode === undefined || errorMessage === undefined || startedAt === undefined
    || finishedAt === undefined || createdAt === undefined) return undefined;
  return { id, runId, stageKind, attempt, status, createdAt,
    ...(inputHash === null ? {} : { inputHash }), ...(outputHash === null ? {} : { outputHash }),
    ...(inputTokens === null ? {} : { inputTokens }), ...(outputTokens === null ? {} : { outputTokens }),
    ...(durationMs === null ? {} : { durationMs }), ...(errorCode === null ? {} : { errorCode }),
    ...(errorMessage === null ? {} : { errorMessage }), ...(startedAt === null ? {} : { startedAt }),
    ...(finishedAt === null ? {} : { finishedAt }) };
}

function decodeOverviewRetry(value: unknown): AiOverviewRetryContext | null | undefined {
  if (value === undefined) return null;
  const data = record(value); if (data === undefined) return undefined;
  const candidateId = string(data.candidateId); const baseRevision = integer(data.baseRevision);
  const targetFields = decodeList(data.targetFields, (field) => typeof field === "string" && /^[a-z][a-z0-9_-]{0,63}$/.test(field) ? field : undefined);
  return candidateId === undefined || baseRevision === undefined || targetFields === undefined
    || targetFields.length === 0 || new Set(targetFields).size !== targetFields.length
    ? undefined : { candidateId, baseRevision, targetFields };
}

function decodeOverviewProfile(value: unknown): AiOverviewProfileContext | null | undefined {
  if (value === undefined) return null;
  const data = record(value); if (data === undefined) return undefined;
  const paperId = string(data.paperId); const selectionRevision = integer(data.selectionRevision);
  const profileRefs = decodeList(data.profileRefs, (value) => {
    const reference = record(value); if (reference === undefined) return undefined;
    const profileId = string(reference.profileId); const profileVersion = integer(reference.profileVersion);
    return profileId === undefined || profileId.length === 0 || profileVersion === undefined
      || profileVersion < 1 ? undefined : { profileId, profileVersion };
  });
  if (data.paperTemplate !== undefined) {
    try {
      const paperTemplate = decodePaperTemplateSnapshot(data.paperTemplate);
      const fields = decodeList(data.fieldKinds, string);
      if (paperId === undefined || paperId.length === 0 || selectionRevision !== paperTemplateRevision(paperTemplate) || profileRefs?.length !== 0 || fields === undefined || JSON.stringify(fields) !== JSON.stringify(paperTemplate.fields.map(field => field.id))) return undefined;
      return { paperId, selectionRevision, profileRefs, fieldKinds: fields, paperTemplate };
    } catch { return undefined; }
  }
  const fieldKinds = decodeList(data.fieldKinds, (field) => oneOf(field, PROFILED_OVERVIEW_FIELDS));
  const refKeys = profileRefs?.map((reference) =>
    `${reference.profileId}:${reference.profileVersion}`) ?? [];
  return paperId === undefined || paperId.length === 0 || selectionRevision === undefined
    || selectionRevision < 1 || profileRefs === undefined || profileRefs.length === 0
    || profileRefs.length > 3 || new Set(refKeys).size !== refKeys.length
    || fieldKinds === undefined || fieldKinds.length === 0
    || new Set(fieldKinds).size !== fieldKinds.length
    ? undefined : { paperId, selectionRevision, profileRefs, fieldKinds };
}

function decodeMatrixSynthesisContext(
  value: unknown,
): AiMatrixSynthesisContext | null | undefined {
  if (value === undefined) return null;
  const data = record(value); if (data === undefined) return undefined;
  const projectId = string(data.projectId); const matrixId = string(data.matrixId);
  const matrixRevision = integer(data.matrixRevision);
  const viewKind = oneOf(data.viewKind, [
    "taxonomy", "historical_context", "gap_hypothesis", "innovation_hypothesis",
  ] as const);
  const title = string(data.title); const scopeNote = string(data.scopeNote);
  const inputs = decodeList(data.inputs, (value) => {
    const input = record(value); if (input === undefined) return undefined;
    const objectKind = oneOf(input.objectKind, ["matrix_cell"] as const);
    const objectId = string(input.objectId); const objectRevision = integer(input.objectRevision);
    const selectionReason = string(input.selectionReason);
    return objectKind === undefined || objectId === undefined || objectId.length === 0
      || objectRevision === undefined || objectRevision < 1 || selectionReason === undefined
      || selectionReason.length > 500 ? undefined
      : { objectKind, objectId, objectRevision, selectionReason };
  });
  const evidenceAnchorIds = decodeList(data.evidenceAnchorIds, string);
  const inputIds = inputs?.map((input) => input.objectId) ?? [];
  return projectId === undefined || projectId.length === 0 || matrixId === undefined
    || matrixId.length === 0 || matrixRevision === undefined || matrixRevision < 1
    || viewKind === undefined || title === undefined || title.length === 0 || title.length > 240
    || scopeNote === undefined || scopeNote.length === 0 || scopeNote.length > 4_000
    || inputs === undefined || inputs.length < 2 || inputs.length > 20
    || new Set(inputIds).size !== inputIds.length || evidenceAnchorIds === undefined
    || evidenceAnchorIds.length < 2 || evidenceAnchorIds.length > 32
    || evidenceAnchorIds.some((id) => id.length === 0)
    || new Set(evidenceAnchorIds).size !== evidenceAnchorIds.length
    ? undefined
    : { projectId, matrixId, matrixRevision, viewKind, title, scopeNote, inputs,
      evidenceAnchorIds };
}

export function decodeAiRun(value: unknown): AiRun | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const taskKind = oneOf(data.taskKind, AI_TASK_KINDS); const status = oneOf(data.status, AI_RUN_STATUSES);
  const providerId = string(data.providerId); const modelId = string(data.modelId); const pipelineVersion = string(data.pipelineVersion);
  const reasoningEffort = data.reasoningEffort === undefined ? null : oneOf(data.reasoningEffort, LLM_REASONING_EFFORTS);
  const promptVersion = string(data.promptVersion); const outputSchemaVersion = integer(data.outputSchemaVersion); const userGoal = string(data.userGoal);
  const scopeHash = string(data.scopeHash); const inputItemCount = integer(data.inputItemCount); const maxInputTokens = integer(data.maxInputTokens);
  const maxOutputTokens = integer(data.maxOutputTokens); const timeoutMs = integer(data.timeoutMs); const cancelRequested = boolean(data.cancelRequested);
  const createdAt = string(data.createdAt); const updatedAt = string(data.updatedAt); const scopeItems = decodeList(data.scopeItems, decodeScopeItem);
  const stages = decodeList(data.stages, decodeStage);
  const cloudConsentVersion = optionalString(data.cloudConsentVersion); const cloudConsentedAt = optionalString(data.cloudConsentedAt);
  const inputTokens = optionalInteger(data.inputTokens); const outputTokens = optionalInteger(data.outputTokens);
  const cacheHitTokens = optionalInteger(data.cacheHitTokens); const cacheMissTokens = optionalInteger(data.cacheMissTokens);
  const errorCode = optionalString(data.errorCode); const errorMessage = optionalString(data.errorMessage);
  const startedAt = optionalString(data.startedAt); const finishedAt = optionalString(data.finishedAt);
  const overviewRetry = decodeOverviewRetry(data.overviewRetry);
  const overviewProfile = decodeOverviewProfile(data.overviewProfile);
  const suggestionCandidateKind = optionalSuggestionKind(data.suggestionCandidateKind);
  const matrixSynthesisContext = decodeMatrixSynthesisContext(data.matrixSynthesisContext);
  const sharedConversationId = optionalString(data.sharedConversationId);
  const sharedRequestId = optionalString(data.sharedRequestId);
  const sharedTurnId = optionalString(data.sharedTurnId);
  const sharedTurnOutcome = data.sharedTurnOutcome === undefined
    ? null
    : oneOf(data.sharedTurnOutcome, ["completed", "failed", "canceled", "interrupted"] as const);
  const sharedCandidateId = optionalString(data.sharedCandidateId);
  const sharedTerminalStatus = optionalString(data.sharedTerminalStatus);
  if (id === undefined || taskKind === undefined || status === undefined || providerId === undefined
    || modelId === undefined || reasoningEffort === undefined || pipelineVersion === undefined || promptVersion === undefined
    || outputSchemaVersion === undefined || userGoal === undefined || scopeHash === undefined
    || inputItemCount === undefined || maxInputTokens === undefined || maxOutputTokens === undefined
    || timeoutMs === undefined || cancelRequested === undefined || createdAt === undefined
    || updatedAt === undefined || scopeItems === undefined || stages === undefined
    || cloudConsentVersion === undefined || cloudConsentedAt === undefined || inputTokens === undefined
    || outputTokens === undefined || cacheHitTokens === undefined || cacheMissTokens === undefined
    || errorCode === undefined || errorMessage === undefined || startedAt === undefined
    || finishedAt === undefined || overviewRetry === undefined || overviewProfile === undefined
    || suggestionCandidateKind === undefined || matrixSynthesisContext === undefined
    || sharedConversationId === undefined || sharedRequestId === undefined
    || sharedTurnId === undefined || sharedTurnOutcome === undefined
    || sharedCandidateId === undefined || sharedTerminalStatus === undefined
    || overviewProfile !== null && taskKind !== "paper_overview"
    || (taskKind === "paper_suggestions") !== (suggestionCandidateKind !== null)
    || (taskKind === "matrix_synthesis") !== (matrixSynthesisContext !== null)) return undefined;
  return { id, taskKind, status, providerId, modelId, pipelineVersion, promptVersion,
    ...(reasoningEffort === null ? {} : { reasoningEffort }),
    outputSchemaVersion, userGoal, scopeHash, inputItemCount, maxInputTokens, maxOutputTokens,
    timeoutMs, cancelRequested, createdAt, updatedAt, scopeItems, stages,
    ...(cloudConsentVersion === null ? {} : { cloudConsentVersion }),
    ...(cloudConsentedAt === null ? {} : { cloudConsentedAt }),
    ...(inputTokens === null ? {} : { inputTokens }), ...(outputTokens === null ? {} : { outputTokens }),
    ...(cacheHitTokens === null ? {} : { cacheHitTokens }), ...(cacheMissTokens === null ? {} : { cacheMissTokens }),
    ...(errorCode === null ? {} : { errorCode }), ...(errorMessage === null ? {} : { errorMessage }),
    ...(startedAt === null ? {} : { startedAt }), ...(finishedAt === null ? {} : { finishedAt }),
    ...(overviewRetry === null ? {} : { overviewRetry }),
    ...(overviewProfile === null ? {} : { overviewProfile }),
    ...(suggestionCandidateKind === null ? {} : { suggestionCandidateKind }),
    ...(matrixSynthesisContext === null ? {} : { matrixSynthesisContext }),
    ...(sharedConversationId === null ? {} : { sharedConversationId }),
    ...(sharedRequestId === null ? {} : { sharedRequestId }),
    ...(sharedTurnId === null ? {} : { sharedTurnId }),
    ...(sharedTurnOutcome === null ? {} : { sharedTurnOutcome }),
    ...(sharedCandidateId === null ? {} : { sharedCandidateId }),
    ...(sharedTerminalStatus === null ? {} : { sharedTerminalStatus }) };
}
