import {
  getAiAssistanceGateway as getCoreAiAssistanceGateway,
  type AiAssistanceGateway as CoreAiAssistanceGateway,
} from "./aiAssistance";

export type AiAssistanceGateway = Pick<CoreAiAssistanceGateway,
  "getLlmConfiguration" | "fetchLlmProviderModels" | "saveLlmModelRoutes" |
  "previewTask" | "startRun" | "getRun" | "cancelRun" | "getLatestPaperPortraitRun" |
  "startPaperAnalysis" | "cancelPaperAnalysis" | "getLatestPaperAnalysisWorkflow" |
  "listCandidates" | "reviseCandidate" | "confirmOverviewFields" |
  "undoOverviewConfirmation" | "recordEvidenceInspection" | "preparePaperReadingContext" |
  "previewConceptRetrieval" | "cancelConceptRetrievalPreview"
>;

const projections = new WeakMap<CoreAiAssistanceGateway, AiAssistanceGateway>();

/** Keep the original live gateway's read-time selection and already-read
 * function binding. Constructing this projection never reads its methods. */
export function getAiAssistanceGateway(): AiAssistanceGateway {
  const wide = getCoreAiAssistanceGateway();
  const existing = projections.get(wide);
  if (existing) return existing;
  const gateway = Object.freeze({
    get getLlmConfiguration() { return wide.getLlmConfiguration; },
    get fetchLlmProviderModels() { return wide.fetchLlmProviderModels; },
    get saveLlmModelRoutes() { return wide.saveLlmModelRoutes; },
    get previewTask() { return wide.previewTask; },
    get startRun() { return wide.startRun; },
    get getRun() { return wide.getRun; },
    get cancelRun() { return wide.cancelRun; },
    get getLatestPaperPortraitRun() { return wide.getLatestPaperPortraitRun; },
    get startPaperAnalysis() { return wide.startPaperAnalysis; },
    get cancelPaperAnalysis() { return wide.cancelPaperAnalysis; },
    get getLatestPaperAnalysisWorkflow() { return wide.getLatestPaperAnalysisWorkflow; },
    get listCandidates() { return wide.listCandidates; },
    get reviseCandidate() { return wide.reviseCandidate; },
    get confirmOverviewFields() { return wide.confirmOverviewFields; },
    get undoOverviewConfirmation() { return wide.undoOverviewConfirmation; },
    get recordEvidenceInspection() { return wide.recordEvidenceInspection; },
    get preparePaperReadingContext() { return wide.preparePaperReadingContext; },
    get previewConceptRetrieval() { return wide.previewConceptRetrieval; },
    get cancelConceptRetrievalPreview() { return wide.cancelConceptRetrievalPreview; },
  });
  projections.set(wide, gateway);
  return gateway;
}

export {aiAssistanceErrorMessage} from "./aiAssistance";
