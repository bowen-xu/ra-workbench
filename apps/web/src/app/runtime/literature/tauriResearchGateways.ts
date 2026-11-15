import {invoke} from "../../../core/runtime/platformCore";
import type {ResearchKnowledgeGateway} from "@research-assistant/literature-library-plugin/contracts/knowledgeGateway";
import type {EvidenceGateway} from "@research-assistant/literature-library-plugin/contracts/evidenceGateway";
import type {EvidenceMatrixGateway} from "@research-assistant/literature-library-plugin/contracts/evidenceMatrixGateway";
import type {DerivedResearchViewGateway} from "@research-assistant/literature-library-plugin/contracts/derivedViewGateway";
import type {ReferenceRecordGateway} from "@research-assistant/literature-library-plugin/contracts/referenceRecordGateway";
import {decodeConcept, decodeConceptCaptureResult, decodeConceptIdentityEvent, decodeConceptMergeImpact, decodeDefinitionClaim, decodeList, decodePaperReadingProfileSelection, decodeReadingProfile, decodeRelationAssertion, decodeRelationNode, decodeReadingUnit, decodeResearchObjectRevision, decodeConceptAlgorithmRun, decodeClaim, decodeEvidenceLink, decodeEvidenceList, decodeSourceAnchorSummary, decodeEvidenceMatrices, decodeEvidenceMatrix, decodeEvidenceMatrixCell, decodeEvidenceMatrixCells, decodeDerivedResearchView, decodeDerivedResearchViews, decodeReferenceRecord, decodeReferenceRecords} from "@research-assistant/literature-library-plugin/transportDecoders";

async function invokeDecoded<T>(
  command: string,
  args: Record<string, unknown>,
  decode: (value: unknown) => T | undefined,
): Promise<T> {
  const value = await invoke<unknown>(command, args);
  const decoded = decode(value);
  if (decoded === undefined) {
    throw new Error("桌面核心返回了不兼容的研究知识数据；本次结果未被采用。");
  }
  return decoded;
}

export const tauriKnowledgeGateway: ResearchKnowledgeGateway = {
  async listReadingProfiles() {
    return invokeDecoded(
      "list_reading_profiles", {},
      (value) => decodeList(value, decodeReadingProfile),
    );
  },
  async getPaperReadingProfileSelection(paperId) {
    return invokeDecoded(
      "get_paper_reading_profile_selection", { paperId },
      decodePaperReadingProfileSelection,
    );
  },
  async savePaperReadingProfileSelection(draft) {
    return invokeDecoded(
      "save_paper_reading_profile_selection", { draft },
      decodePaperReadingProfileSelection,
    );
  },
  async listReadingUnits(projectId, paperId) {
    return invokeDecoded(
      "list_structured_reading_units",
      { projectId, paperId },
      (value) => decodeList(value, decodeReadingUnit),
    );
  },
  async saveReadingUnit(draft) {
    return invokeDecoded("save_structured_reading_unit", { draft }, decodeReadingUnit);
  },
  async searchConcepts(query) {
    return invokeDecoded("search_concepts", { query }, (value) => decodeList(value, decodeConcept));
  },
  async previewConceptMerge(draft) {
    return invokeDecoded("preview_concept_merge", { draft }, decodeConceptMergeImpact);
  },
  async mergeConcepts(draft) {
    return invokeDecoded("merge_concepts", { draft }, decodeConceptIdentityEvent);
  },
  async splitConcepts(draft) {
    return invokeDecoded("split_concepts", { draft }, decodeConceptIdentityEvent);
  },
  async listConceptIdentityEvents() {
    return invokeDecoded(
      "list_concept_identity_events", {},
      (value) => decodeList(value, decodeConceptIdentityEvent),
    );
  },
  async saveConcept(draft) {
    return invokeDecoded("save_concept", { draft }, decodeConcept);
  },
  async captureConceptUsage(draft) {
    return invokeDecoded("capture_concept_usage", { draft }, decodeConceptCaptureResult);
  },
  async listDefinitionClaims(conceptId) {
    return invokeDecoded(
      "list_definition_claims",
      { conceptId },
      (value) => decodeList(value, decodeDefinitionClaim),
    );
  },
  async saveDefinitionClaim(draft) {
    return invokeDecoded("save_definition_claim", { draft }, decodeDefinitionClaim);
  },
  async searchRelationNodes(query) {
    return invokeDecoded(
      "search_relation_nodes",
      { query },
      (value) => decodeList(value, decodeRelationNode),
    );
  },
  async saveRelationAssertion(draft) {
    return invokeDecoded("save_relation_assertion", { draft }, decodeRelationAssertion);
  },
  async listRelationAssertions(projectId) {
    return invokeDecoded(
      "list_relation_assertions",
      { projectId },
      (value) => decodeList(value, decodeRelationAssertion),
    );
  },
  async listRevisions(objectKind, objectId) {
    return invokeDecoded(
      "list_research_object_revisions",
      { objectKind, objectId },
      (value) => decodeList(value, decodeResearchObjectRevision),
    );
  },
  async analyzeConceptEvolution(conceptId) {
    return invokeDecoded(
      "analyze_concept_evolution",
      { conceptId },
      decodeConceptAlgorithmRun,
    );
  },
  async analyzeSharedFoundations(draft) {
    return invokeDecoded(
      "analyze_shared_foundations",
      { draft },
      decodeConceptAlgorithmRun,
    );
  },
  async getConceptAlgorithmRun(runId) {
    return invokeDecoded(
      "get_concept_algorithm_run",
      { runId },
      decodeConceptAlgorithmRun,
    );
  },
};

async function invokeEvidence<T>(
  command: string,
  args: Record<string, unknown>,
  decode: (value: unknown) => T | undefined,
): Promise<T> {
  const value = await invoke<unknown>(command, args);
  const decoded = decode(value);
  if (decoded === undefined) {
    throw new Error("桌面核心返回了不兼容的观点或证据数据；本次结果未被采用。");
  }
  return decoded;
}

export const tauriEvidenceGateway: EvidenceGateway = {
  async listClaims(projectId) {
    return invokeEvidence("list_claims", { projectId }, (value) => decodeEvidenceList(value, decodeClaim));
  },
  async saveClaim(draft) {
    return invokeEvidence("save_claim", { draft }, decodeClaim);
  },
  async listSources(projectId) {
    return invokeEvidence(
      "list_source_anchors",
      { projectId },
      (value) => decodeEvidenceList(value, decodeSourceAnchorSummary),
    );
  },
  async linkEvidence(draft) {
    return invokeEvidence("link_claim_to_anchor", { draft }, decodeEvidenceLink);
  },
  async createClaimWithEvidence(claim, evidence) {
    return invokeEvidence("create_claim_with_evidence", { claim, evidence }, decodeClaim);
  },
  async listClaimEvidence(claimId) {
    return invokeEvidence(
      "list_claim_evidence",
      { claimId },
      (value) => decodeEvidenceList(value, decodeEvidenceLink),
    );
  },
};

async function invokeMatrixDecoded<T>(
  command: string,
  args: Record<string, unknown>,
  decode: (value: unknown) => T | undefined,
): Promise<T> {
  const decoded = decode(await invoke<unknown>(command, args));
  if (decoded === undefined) throw new Error("桌面核心返回了不兼容的证据矩阵数据");
  return decoded;
}

export const tauriEvidenceMatrixGateway: EvidenceMatrixGateway = {
  async list(projectId) {
    return invokeMatrixDecoded("list_evidence_matrices", { projectId }, decodeEvidenceMatrices);
  },
  async save(draft) {
    return invokeMatrixDecoded("save_evidence_matrix", { draft }, decodeEvidenceMatrix);
  },
  async listCells(matrixId) {
    return invokeMatrixDecoded("list_evidence_matrix_cells", { matrixId }, decodeEvidenceMatrixCells);
  },
  async saveCell(draft) {
    return invokeMatrixDecoded("save_evidence_matrix_cell", { draft }, decodeEvidenceMatrixCell);
  },
};

export const tauriDerivedViewGateway: DerivedResearchViewGateway = {
  async list(projectId) {
    const decoded = decodeDerivedResearchViews(
      await invoke<unknown>("list_derived_research_views", { projectId }),
    );
    if (decoded === undefined) throw new Error("桌面核心返回了不兼容的派生视图列表");
    return decoded;
  },
  async save(draft) {
    const decoded = decodeDerivedResearchView(
      await invoke<unknown>("save_derived_research_view", { draft }),
    );
    if (decoded === undefined) throw new Error("桌面核心返回了不兼容的派生视图");
    return decoded;
  },
};

export const tauriReferenceRecordGateway: ReferenceRecordGateway = {
  async list(paperId) {
    const decoded = decodeReferenceRecords(
      await invoke<unknown>("list_reference_records", { paperId }),
    );
    if (decoded === undefined) throw new Error("桌面核心返回了不兼容的参考条目列表");
    return decoded;
  },
  async save(draft) {
    const decoded = decodeReferenceRecord(
      await invoke<unknown>("save_reference_record", { draft }),
    );
    if (decoded === undefined) throw new Error("桌面核心返回了不兼容的参考条目");
    return decoded;
  },
};
