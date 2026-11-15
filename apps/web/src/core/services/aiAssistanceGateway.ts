import { getAppRuntime, runtimeAgentGateway } from "../runtime/appRuntime";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import type {
  AiCandidate,
  AiCitationAssessmentDraft,
  AiAttentionFeedback,
  AiAttentionFeedbackDraft,
  AiCandidateRevisionDraft,
  AiCandidateSummaryDiscardDraft,
  AiCandidateSummarySelectionDraft,
  AiCandidateSummaryVersionDraft,
  AiCredentialSummary,
  AiCredentialUpdate,
  AiOverviewConfirmationResult,
  AiOverviewConfirmationUndoDraft,
  AiOverviewFieldConfirmationDraft,
  AiPaperReviewConfirmationDraft,
  AiPaperReviewConfirmationResult,
  AiPaperReviewConfirmationUndoDraft,
  AiProviderOverview,
  AiProviderSettings,
  AiProviderSettingsDraft,
  AiProviderTestResult,
  AiRun,
  AiReviewDecisionDraft,
  AiSuggestionItemDecisionDraft,
  AiTaskPreview,
  AiTaskSpecDraft,
  ChatGptAccountStatus,
  ChatGptLoginStart,
  ConceptRetrievalPreview,
  ConceptRetrievalPreviewDraft,
  LlmConfiguration,
  LlmModelRouteDraft,
  LlmModelKind,
  LlmProviderConfigDraft,
  LlmProviderId,
  LlmProviderModels,
  PaperAiReadingCancelResult,
  PaperAiReadingPrepareResult,
  PaperAiReadingSessionDetail,
  PaperAiReadingSessionSummary,
  PaperAiReadingSteerResult,
  PaperAiReadingTurnDraft,
  PaperAiReadingTurnDebugData,
  PaperAnalysisStartDraft,
  PaperAnalysisWorkflow,
  PaperReadingBootstrapOptions,
} from "@research-assistant/core/ai";
import providerDefaults from "../../../../../packages/core/src/ai/llm-provider-defaults.json";

import {
  decodeAiCredentialSummary,
  decodeAiProviderOverview,
  decodeAiProviderSettings,
  decodeAiProviderTestResult,
  decodeAiRun,
  decodeAiTaskPreview,
  decodeChatGptAccountStatus,
  decodeChatGptLoginStart,
  decodeList,
  decodeLlmConfiguration,
  decodeLlmProviderModels,
  decodePaperAiReadingPrepareResult,
  decodePaperAiReadingSessionDetail,
  decodePaperAiReadingSessionList,
  decodePaperAiReadingTurnDebugData,
  decodePaperAnalysisWorkflow,
} from "./aiDecoders";
import {
  decodeAiAttentionFeedback,
  decodeAiCandidate,
  decodeAiCandidateListBestEffort,
  decodeAiOverviewConfirmationResult,
  decodeAiPaperReviewConfirmationResult,
} from "./aiCandidateDecoders";
import { decodePaperReadingBootstrapOptions } from "./paperReadingInteractionDecoders";
import { decodeConceptRetrievalPreview } from "./conceptRetrievalDecoders";

export interface AiAssistanceGateway {
  getLlmConfiguration(): Promise<LlmConfiguration>;
  saveLlmProviderConfig(draft: LlmProviderConfigDraft): Promise<LlmConfiguration>;
  saveLlmModelRoutes(drafts: readonly LlmModelRouteDraft[]): Promise<LlmConfiguration>;
  fetchLlmProviderModels(providerId: LlmProviderId, refresh?: boolean): Promise<LlmProviderModels>;
  getChatGptAccountStatus(): Promise<ChatGptAccountStatus>;
  startChatGptLogin(): Promise<ChatGptLoginStart>;
  cancelChatGptLogin(loginId: string): Promise<boolean>;
  logoutChatGpt(): Promise<boolean>;
  listProviders(): Promise<readonly AiProviderOverview[]>;
  saveProviderSettings(draft: AiProviderSettingsDraft): Promise<AiProviderSettings>;
  setCredential(update: AiCredentialUpdate): Promise<AiCredentialSummary>;
  deleteCredential(providerId: AiCredentialUpdate["providerId"]): Promise<AiCredentialSummary>;
  testProvider(providerId: string): Promise<AiProviderTestResult>;
  previewTask(draft: AiTaskSpecDraft): Promise<AiTaskPreview>;
  startRun(draft: AiTaskSpecDraft): Promise<AiRun>;
  startPaperAnalysis(draft: PaperAnalysisStartDraft): Promise<PaperAnalysisWorkflow>;
  cancelPaperAnalysis(clientRequestId: string): Promise<boolean>;
  getPaperAnalysisWorkflow(workflowId: string): Promise<PaperAnalysisWorkflow>;
  getLatestPaperAnalysisWorkflow(paperId: string): Promise<PaperAnalysisWorkflow | undefined>;
  getRun(runId: string): Promise<AiRun>;
  getLatestPaperPortraitRun(paperId: string): Promise<AiRun | undefined>;
  listRuns(limit?: number): Promise<readonly AiRun[]>;
  cancelRun(runId: string): Promise<boolean>;
  listCandidates(projectId?: string, scope?: { readonly paperId: string }): Promise<readonly AiCandidate[]>;
  reviseCandidate(draft: AiCandidateRevisionDraft): Promise<AiCandidate>;
  confirmOverviewFields(
    draft: AiOverviewFieldConfirmationDraft,
  ): Promise<AiOverviewConfirmationResult>;
  undoOverviewConfirmation(
    draft: AiOverviewConfirmationUndoDraft,
  ): Promise<AiOverviewConfirmationResult>;
  confirmPaperReview(
    draft: AiPaperReviewConfirmationDraft,
  ): Promise<AiPaperReviewConfirmationResult>;
  undoPaperReviewConfirmation(
    draft: AiPaperReviewConfirmationUndoDraft,
  ): Promise<AiPaperReviewConfirmationResult>;
  decideCandidate(draft: AiReviewDecisionDraft): Promise<AiCandidate>;
  decideSuggestionItem(draft: AiSuggestionItemDecisionDraft): Promise<AiCandidate>;
  decidePaperConceptItem(draft: AiSuggestionItemDecisionDraft): Promise<AiCandidate>;
  recordCitationAssessment(draft: AiCitationAssessmentDraft): Promise<AiCandidate>;
  recordEvidenceInspection(candidateId: string, candidateRevision: number, evidenceItemId: string): Promise<boolean>;
  recordAttentionFeedback(draft: AiAttentionFeedbackDraft): Promise<AiAttentionFeedback>;
  createCandidateSummaryVersion(draft: AiCandidateSummaryVersionDraft): Promise<AiCandidate>;
  selectCandidateSummaryVersion(draft: AiCandidateSummarySelectionDraft): Promise<AiCandidate>;
  discardOtherCandidateSummaryVersions(draft: AiCandidateSummaryDiscardDraft): Promise<AiCandidate>;
  preparePaperReadingContext(paperId: string, attachmentIds: readonly string[]): Promise<PaperAiReadingPrepareResult>;
  preparePaperReadingBootstrap(
    paperId: string,
    attachmentIds: readonly string[],
  ): Promise<PaperReadingBootstrapOptions>;
  previewConceptRetrieval(draft: ConceptRetrievalPreviewDraft): Promise<ConceptRetrievalPreview>;
  cancelConceptRetrievalPreview(clientRequestId: string): Promise<boolean>;
  getConceptRetrievalTrace(traceId: string): Promise<ConceptRetrievalPreview>;
  startPaperReadingTurn(draft: PaperAiReadingTurnDraft): Promise<PaperAiReadingSessionDetail>;
  steerPaperReadingTurn(clientRequestId: string, userMessage: string): Promise<PaperAiReadingSteerResult>;
  cancelPaperReadingTurn(clientRequestId: string): Promise<PaperAiReadingCancelResult>;
  listPaperReadingSessions(paperId: string): Promise<readonly PaperAiReadingSessionSummary[]>;
  getPaperReadingSession(sessionId: string): Promise<PaperAiReadingSessionDetail>;
  getPaperReadingTurnDebugData(turnId: string): Promise<PaperAiReadingTurnDebugData>;
  deletePaperReadingSession(sessionId: string): Promise<boolean>;
}

async function invokeDecoded<T>(
  command: string,
  args: Record<string, unknown>,
  decode: (value: unknown) => T | undefined,
): Promise<T> {
  const value = await invoke<unknown>(command, args);
  const decoded = decode(value);
  if (decoded === undefined) {
    throw new Error("桌面核心返回了不兼容的 AI 数据；本次结果未被采用。");
  }
  return decoded;
}

const desktopGateway: AiAssistanceGateway = {
  async getLlmConfiguration() {
    return invokeDecoded("get_llm_configuration", {}, decodeLlmConfiguration);
  },
  async saveLlmProviderConfig(draft) {
    return invokeDecoded("save_llm_provider_config", { draft }, decodeLlmConfiguration);
  },
  async saveLlmModelRoutes(drafts) {
    return invokeDecoded("save_llm_model_routes", { drafts }, decodeLlmConfiguration);
  },
  async fetchLlmProviderModels(providerId, refresh = false) {
    return invokeDecoded("fetch_llm_provider_models", { providerId, refresh }, decodeLlmProviderModels);
  },
  async getChatGptAccountStatus() {
    return invokeDecoded("get_chatgpt_account_status", {}, decodeChatGptAccountStatus);
  },
  async startChatGptLogin() {
    return invokeDecoded("start_chatgpt_login", {}, decodeChatGptLoginStart);
  },
  async cancelChatGptLogin(loginId) {
    return invokeDecoded("cancel_chatgpt_login", { loginId }, (value) =>
      typeof value === "boolean" ? value : undefined);
  },
  async logoutChatGpt() {
    return invokeDecoded("logout_chatgpt", {}, (value) =>
      typeof value === "boolean" ? value : undefined);
  },
  async listProviders() {
    return invokeDecoded("list_ai_provider_overviews", {}, (value) =>
      decodeList(value, decodeAiProviderOverview));
  },
  async saveProviderSettings(draft) {
    return invokeDecoded("save_ai_provider_settings", { draft }, decodeAiProviderSettings);
  },
  async setCredential(update) {
    return invokeDecoded(
      "set_ai_credential",
      { providerId: update.providerId, apiKey: update.apiKey },
      decodeAiCredentialSummary,
    );
  },
  async deleteCredential(providerId) {
    return invokeDecoded("delete_ai_credential", { providerId }, decodeAiCredentialSummary);
  },
  async testProvider(providerId) {
    return invokeDecoded("test_ai_provider", { providerId }, decodeAiProviderTestResult);
  },
  async previewTask(draft) {
    return invokeDecoded("preview_ai_task", { draft }, decodeAiTaskPreview);
  },
  async startRun(draft) {
    return invokeDecoded("start_ai_run", { draft }, decodeAiRun);
  },
  async startPaperAnalysis(draft) {
    return invokeDecoded("start_paper_analysis_workflow", { draft }, decodePaperAnalysisWorkflow);
  },
  async cancelPaperAnalysis(clientRequestId) {
    return invokeDecoded("cancel_paper_analysis_workflow", { clientRequestId }, (value) =>
      typeof value === "boolean" ? value : undefined);
  },
  async getPaperAnalysisWorkflow(workflowId) {
    return invokeDecoded("get_paper_analysis_workflow", { workflowId }, decodePaperAnalysisWorkflow);
  },
  async getLatestPaperAnalysisWorkflow(paperId) {
    const result = await invoke<unknown>("get_latest_paper_analysis_workflow", { paperId });
    if (result === null) return undefined;
    const decoded = decodePaperAnalysisWorkflow(result);
    if (decoded === undefined) throw new Error("论文分析工作流返回结构无效");
    return decoded;
  },
  async getRun(runId) {
    return invokeDecoded("get_ai_run", { runId }, decodeAiRun);
  },
  async listRuns(limit) {
    return invokeDecoded("list_ai_runs", { limit }, (value) => decodeList(value, decodeAiRun));
  },
  async getLatestPaperPortraitRun(paperId) {
    const result = await invoke<unknown>("get_latest_paper_portrait_run", { paperId });
    if (result === null) return undefined;
    const run = decodeAiRun(result);
    if (run === undefined) throw new Error("画像任务返回结构无效");
    return run;
  },
  async cancelRun(runId) {
    return invokeDecoded("cancel_ai_run", { runId }, (value) =>
      typeof value === "boolean" ? value : undefined);
  },
  async listCandidates(projectId, scope) {
    return invokeDecoded("list_ai_candidates", { projectId, ...(scope === undefined ? {} : { paperId: scope.paperId }) }, decodeAiCandidateListBestEffort);
  },
  async reviseCandidate(draft) {
    return invokeDecoded("revise_ai_candidate", { draft }, decodeAiCandidate);
  },
  async confirmOverviewFields(draft) {
    return invokeDecoded(
      "confirm_ai_overview_fields",
      { draft },
      decodeAiOverviewConfirmationResult,
    );
  },
  async undoOverviewConfirmation(draft) {
    return invokeDecoded(
      "undo_ai_overview_confirmation",
      { draft },
      decodeAiOverviewConfirmationResult,
    );
  },
  async confirmPaperReview(draft) {
    return invokeDecoded(
      "confirm_ai_paper_review",
      { draft },
      decodeAiPaperReviewConfirmationResult,
    );
  },
  async undoPaperReviewConfirmation(draft) {
    return invokeDecoded(
      "undo_ai_paper_review_confirmation",
      { draft },
      decodeAiPaperReviewConfirmationResult,
    );
  },
  async decideCandidate(draft) {
    return invokeDecoded("decide_ai_candidate", { draft }, decodeAiCandidate);
  },
  async decideSuggestionItem(draft) {
    return invokeDecoded("decide_ai_suggestion_item", { draft }, decodeAiCandidate);
  },
  async decidePaperConceptItem(draft) {
    return invokeDecoded("decide_ai_paper_concept_item", { draft }, decodeAiCandidate);
  },
  async recordCitationAssessment(draft) {
    return invokeDecoded("record_ai_citation_assessment", { draft }, decodeAiCandidate);
  },
  async recordEvidenceInspection(candidateId, candidateRevision, evidenceItemId) {
    return invokeDecoded(
      "record_ai_evidence_inspection",
      { candidateId, candidateRevision, evidenceItemId },
      (value) => typeof value === "boolean" ? value : undefined,
    );
  },
  async recordAttentionFeedback(draft) {
    return invokeDecoded(
      "record_ai_attention_feedback",
      { draft },
      decodeAiAttentionFeedback,
    );
  },
  async createCandidateSummaryVersion(draft) {
    return invokeDecoded("create_ai_candidate_summary_version", { draft }, decodeAiCandidate);
  },
  async selectCandidateSummaryVersion(draft) {
    return invokeDecoded("select_ai_candidate_summary_version", { draft }, decodeAiCandidate);
  },
  async discardOtherCandidateSummaryVersions(draft) {
    return invokeDecoded(
      "discard_other_ai_candidate_summary_versions",
      { draft },
      decodeAiCandidate,
    );
  },
  async preparePaperReadingContext(paperId, attachmentIds) {
    return invokeDecoded(
      "prepare_paper_ai_reading_context",
      { paperId, attachmentIds },
      decodePaperAiReadingPrepareResult,
    );
  },
  async preparePaperReadingBootstrap(paperId, attachmentIds) {
    return invokeDecoded(
      "prepare_paper_reading_bootstrap",
      { paperId, attachmentIds },
      decodePaperReadingBootstrapOptions,
    );
  },
  async previewConceptRetrieval(draft) {
    return invokeDecoded(
      "preview_concept_retrieval",
      { draft },
      decodeConceptRetrievalPreview,
    );
  },
  async cancelConceptRetrievalPreview(clientRequestId) {
    return invokeDecoded(
      "cancel_concept_retrieval_preview",
      { clientRequestId },
      (value) => typeof value === "boolean" ? value : undefined,
    );
  },
  async getConceptRetrievalTrace(traceId) {
    return invokeDecoded(
      "get_concept_retrieval_trace",
      { traceId },
      decodeConceptRetrievalPreview,
    );
  },
  async startPaperReadingTurn(draft) {
    void draft;
    return legacyPaperReadingExecutionUnavailable();
  },
  async steerPaperReadingTurn(clientRequestId, userMessage) {
    void clientRequestId;
    void userMessage;
    return legacyPaperReadingExecutionUnavailable();
  },
  async cancelPaperReadingTurn(clientRequestId) {
    void clientRequestId;
    return legacyPaperReadingExecutionUnavailable();
  },
  async listPaperReadingSessions(paperId) {
    return invokeDecoded(
      "list_paper_ai_reading_sessions",
      { paperId },
      decodePaperAiReadingSessionList,
    );
  },
  async getPaperReadingSession(sessionId) {
    return invokeDecoded(
      "get_paper_ai_reading_session",
      { sessionId },
      decodePaperAiReadingSessionDetail,
    );
  },
  async getPaperReadingTurnDebugData(turnId) {
    return invokeDecoded(
      "get_paper_ai_reading_turn_debug_data",
      { turnId },
      decodePaperAiReadingTurnDebugData,
    );
  },
  async deletePaperReadingSession(sessionId) {
    return invokeDecoded(
      "delete_paper_ai_reading_session",
      { sessionId },
      (value) => typeof value === "boolean" ? value : undefined,
    );
  },
};

function unavailable(): never {
  throw new Error("AI provider 设置只在桌面应用中可用；浏览器预览不会保存凭据或调用外部服务。");
}

const browserLlmConfiguration: LlmConfiguration = {
  providers: providerDefaults.providers.map((provider) => ({
    providerId: provider.id as LlmProviderId,
    displayName: provider.displayName,
    authMode: provider.authMode as "api-key" | "chatgpt-login",
    protocol: provider.protocol as "openai-compatible" | "codex-cli",
    modelKinds: provider.modelKinds as readonly LlmModelKind[],
    defaultBaseUrls: provider.defaultBaseUrls,
    baseUrl: provider.defaultBaseUrls[0] ?? "",
    usesDefaultBaseUrl: true,
    enabled: false,
    timeoutMs: 60_000,
    credentialConfigured: false,
  })),
  defaultModels: [
    { routeKind: "default", routeId: "text", label: "文本模型", modelKind: "text" },
    { routeKind: "default", routeId: "embedding", label: "Embedding 模型", modelKind: "embedding" },
    { routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision" },
  ],
  useCases: [
    { routeKind: "use_case", routeId: "selection_explanation", label: "解释所选内容", modelKind: "text" },
    { routeKind: "use_case", routeId: "paper_profile_classification", label: "生成文献画像", modelKind: "text" },
    { routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision" },
    { routeKind: "use_case", routeId: "paper_reading_chat", label: "论文 AI 阅读对话", modelKind: "vision" },
    { routeKind: "use_case", routeId: "paper_suggestions", label: "生成阅读建议", modelKind: "text" },
    { routeKind: "use_case", routeId: "reference_extraction", label: "提取参考文献", modelKind: "text" },
    { routeKind: "use_case", routeId: "matrix_synthesis", label: "多文献综合", modelKind: "text" },
  ],
};

const browserGateway: AiAssistanceGateway = {
  async getLlmConfiguration() { return browserLlmConfiguration; },
  async saveLlmProviderConfig() { return unavailable(); },
  async saveLlmModelRoutes() { return unavailable(); },
  async fetchLlmProviderModels() { return unavailable(); },
  async getChatGptAccountStatus() { return unavailable(); },
  async startChatGptLogin() { return unavailable(); },
  async cancelChatGptLogin() { return unavailable(); },
  async logoutChatGpt() { return unavailable(); },
  async listProviders() { return []; },
  async saveProviderSettings() { return unavailable(); },
  async setCredential() { return unavailable(); },
  async deleteCredential() { return unavailable(); },
  async testProvider() { return unavailable(); },
  async previewTask() { return unavailable(); },
  async startRun() { return unavailable(); },
  async startPaperAnalysis() { return unavailable(); },
  async cancelPaperAnalysis() { return unavailable(); },
  async getPaperAnalysisWorkflow() { return unavailable(); },
  async getLatestPaperAnalysisWorkflow() { return undefined; },
  async getRun() { return unavailable(); },
  async listRuns() { return []; },
  async getLatestPaperPortraitRun() { return undefined; },
  async cancelRun() { return unavailable(); },
  async listCandidates() { return []; },
  async reviseCandidate() { return unavailable(); },
  async confirmOverviewFields() { return unavailable(); },
  async undoOverviewConfirmation() { return unavailable(); },
  async confirmPaperReview() { return unavailable(); },
  async undoPaperReviewConfirmation() { return unavailable(); },
  async decideCandidate() { return unavailable(); },
  async decideSuggestionItem() { return unavailable(); },
  async decidePaperConceptItem() { return unavailable(); },
  async recordCitationAssessment() { return unavailable(); },
  async recordEvidenceInspection() { return unavailable(); },
  async recordAttentionFeedback() { return unavailable(); },
  async createCandidateSummaryVersion() { return unavailable(); },
  async selectCandidateSummaryVersion() { return unavailable(); },
  async discardOtherCandidateSummaryVersions() { return unavailable(); },
  async preparePaperReadingContext() { return unavailable(); },
  async preparePaperReadingBootstrap() { return unavailable(); },
  async previewConceptRetrieval() { return unavailable(); },
  async cancelConceptRetrievalPreview() { return unavailable(); },
  async getConceptRetrievalTrace() { return unavailable(); },
  async startPaperReadingTurn() { return unavailable(); },
  async steerPaperReadingTurn() { return unavailable(); },
  async cancelPaperReadingTurn() { return unavailable(); },
  async listPaperReadingSessions() { return []; },
  async getPaperReadingSession() { return unavailable(); },
  async getPaperReadingTurnDebugData() { return unavailable(); },
  async deletePaperReadingSession() { return unavailable(); },
};

/**
 * The legacy paper-reading panel remains as a behavior fixture while its
 * production host uses the shared Agent submission gateway.  These commands
 * are intentionally no longer registered by the native shell: failing here
 * gives an actionable migration error instead of making the old panel look
 * like it owns a second Agent runtime.
 */
function legacyPaperReadingExecutionUnavailable(): never {
  throw new Error("论文阅读旧执行入口已迁移到共享 Agent；请使用共享阅读面板。 ");
}

const selectedGateway = new Proxy(desktopGateway, { get(_target, property) {
  const gateway = isDesktopRuntime() || getAppRuntime().nativeAgentPreview ? desktopGateway : browserGateway;
  const member: unknown = Reflect.get(gateway, property);
  return typeof member === "function" ? member.bind(gateway) : member;
} });
const liveGateway = runtimeAgentGateway("getAiAssistanceGateway", selectedGateway);
/** Application assembly can wrap this transport with domain-owned admission.
 * Plugin SDK consumers use getAiAssistanceGateway and its selected application port.
 */
export function getAiAssistanceTransport(): AiAssistanceGateway { return selectedGateway; }
export function getAiAssistanceGateway(): AiAssistanceGateway { return liveGateway; }

export { aiAssistanceErrorMessage } from "./aiAssistanceErrorMessage";
