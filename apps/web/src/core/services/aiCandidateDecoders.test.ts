import { describe, expect, it } from "vitest";

import {
  decodeAiAttentionFeedback,
  decodeAiCandidate,
  decodeAiCandidateListBestEffort,
  decodeAiOverviewConfirmationResult,
  decodeAiPaperReviewConfirmationResult,
  decodeAiSuggestionItemDecision,
} from "./aiCandidateDecoders";

function candidate(evidenceItemIds: readonly string[] = ["ev-quote"]): unknown {
  return {
    id: "candidate", runId: "run", providerId: "fixture-generation", modelId: "fixture-v1",
    candidateKind: "selection_explanation_v1", schemaVersion: 1, currentRevision: 1,
    reviewState: "unreviewed", validationStatus: "valid", originalPayloadHash: "a".repeat(64),
    createdAt: "now", updatedAt: "now",
    evidenceBundle: {
      id: "bundle", runId: "run", scopeHash: "b".repeat(64), retrievalConfigVersion: "v1",
      validationStatus: "valid", createdAt: "now", items: [{
        id: "evidence", citationKey: "ev-quote", ordinal: 0, anchorId: "anchor",
        paperId: "paper", paperTitle: "Paper", attachmentId: "attachment",
        contentSha256: "c".repeat(64), pageIndex: 1, quoteExact: "Exact quote",
        quotePrefix: "", quoteSuffix: "", contextText: "Exact quote",
        relationKind: "direct_quote", validationStatus: "valid",
        currentAnchorStatus: "exact", createdAt: "now",
      }],
    },
    revisions: [{
      id: "revision", revision: 1, payloadHash: "a".repeat(64), editorKind: "ai",
      sourceRunId: "run",
      changeReason: "provider_generation", createdAt: "now", payload: {
        kind: "selection_explanation_v1", schemaVersion: 1,
        statements: [{
          id: "s1", text: "Explanation", epistemicRole: "source_paraphrase", evidenceItemIds,
        }],
        uncertainties: [],
      },
    }],
    decisions: [],
    attentionFeedback: [],
    suggestionItemDecisions: [],
    citationAssessments: [],
    overviewFieldConfirmations: [],
  };
}

describe("AI candidate IPC decoder", () => {
  it("skips malformed candidates when decoding list responses", () => {
    const broken = candidate() as Record<string, unknown>;
    broken.id = "broken";
    broken.revisions = [];
    const decoded = decodeAiCandidateListBestEffort([
      candidate(),
      broken,
    ]);
    expect(decoded?.map((item) => item.id)).toEqual(["candidate"]);
  });

  it("validates anchor state and citation assessment history at the IPC boundary", () => {
    const value = candidate() as Record<string, unknown>;
    value.citationAssessments = [{
      id: "assessment", candidateId: "candidate", candidateRevision: 1,
      statementId: "s1", evidenceItemId: "evidence", assessmentRevision: 1,
      semanticRelation: "supports", rationale: "checked", idempotencyKey: "key",
      assessedAt: "now",
    }];
    expect(decodeAiCandidate(value)?.citationAssessments).toHaveLength(1);
    const assessments = value.citationAssessments as Array<Record<string, unknown>>;
    assessments[0]!.semanticRelation = "verified";
    expect(decodeAiCandidate(value)).toBeUndefined();
    assessments[0]!.semanticRelation = "supports";
    assessments[0]!.statementId = "unknown";
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("restores durable evidence inspections and rejects foreign evidence ids", () => {
    const value = candidate() as Record<string, unknown>;
    value.inspectedEvidenceItemIds = ["evidence"];
    expect(decodeAiCandidate(value)?.inspectedEvidenceItemIds).toEqual(["evidence"]);
    value.inspectedEvidenceItemIds = ["other"];
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("validates attention feedback at the IPC boundary", () => {
    const feedback = {
      id: "feedback", candidateId: "candidate", candidateRevision: 1,
      suggestionId: "s1", feedbackKind: "useful", note: "", idempotencyKey: "key",
      createdAt: "now",
    };
    expect(decodeAiAttentionFeedback(feedback)?.feedbackKind).toBe("useful");
    expect(decodeAiAttentionFeedback({ ...feedback, feedbackKind: "accepted" })).toBeUndefined();
  });

  it("rejects a candidate containing malformed attention feedback", () => {
    const value = candidate() as Record<string, unknown>;
    const feedback = {
      id: "feedback", candidateId: "candidate", candidateRevision: 1,
      suggestionId: "s1", feedbackKind: "useful", note: "", idempotencyKey: "key",
      createdAt: "now",
    };
    value.attentionFeedback = [feedback];
    expect(decodeAiCandidate(value)?.attentionFeedback).toHaveLength(1);
    value.attentionFeedback = [{ ...feedback, suggestionId: "not-in-revision" }];
    expect(decodeAiCandidate(value)).toBeUndefined();
    value.attentionFeedback = [{ ...feedback, candidateId: "another-candidate" }];
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("validates item-level suggestion decisions and their revision identity", () => {
    const decision = {
      id: "item-decision", candidateId: "candidate", candidateRevision: 1,
      suggestionId: "s1", action: "accept_unchanged", resolutionKind: "create_concept",
      rationale: "reviewed", idempotencyKey: "item-key", targetObjectKind: "concept",
      targetObjectId: "concept", targetObjectRevision: 1, createdAt: "now",
    };
    expect(decodeAiSuggestionItemDecision(decision)?.targetObjectKind).toBe("concept");
    expect(decodeAiSuggestionItemDecision({
      ...decision,
      resolutionKind: "none",
    })).toBeUndefined();
    const value = candidate() as Record<string, unknown>;
    value.suggestionItemDecisions = [decision];
    expect(decodeAiCandidate(value)?.suggestionItemDecisions).toHaveLength(1);
    value.suggestionItemDecisions = [{ ...decision, suggestionId: "unknown" }];
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("decodes a candidate only after validating its structured citations", () => {
    expect(decodeAiCandidate(candidate())?.revisions[0]?.payload.kind)
      .toBe("selection_explanation_v1");
  });

  it("validates candidate summary versions at the IPC boundary", () => {
    const value = candidate() as Record<string, unknown>;
    value.summaryVersions = [{
      id: "summary-1",
      candidateId: "candidate",
      candidateRevision: 1,
      version: 1,
      summary: "Concise explanation",
      sourceKind: "local_extractive",
      isSelected: true,
      createdAt: "now",
    }];
    expect(decodeAiCandidate(value)?.summaryVersions?.[0]?.summary).toBe("Concise explanation");
    const firstSummary = (value.summaryVersions as readonly Record<string, unknown>[])[0]!;
    value.summaryVersions = [
      firstSummary,
      { ...firstSummary, id: "summary-2", version: 2 },
    ];
    expect(decodeAiCandidate(value)).toBeUndefined();
    value.summaryVersions = [{
      id: "summary-1",
      candidateId: "other-candidate",
      candidateRevision: 1,
      version: 1,
      summary: "Concise explanation",
      sourceKind: "local_extractive",
      isSelected: false,
      createdAt: "now",
    }];
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("decodes grounded concept suggestions and their separate attention feedback", () => {
    const value = candidate() as Record<string, unknown>;
    value.candidateKind = "concept_suggestion_v1";
    const revisions = value.revisions as Array<Record<string, unknown>>;
    revisions[0]!.payload = {
      kind: "concept_suggestion_v1",
      schemaVersion: 1,
      suggestions: [{
        id: "concept_1", term: "Sparse gating", explanation: "Activates some experts.",
        epistemicRole: "source_paraphrase", evidenceItemIds: ["ev-quote"],
      }],
    };
    value.attentionFeedback = [{
      id: "feedback", candidateId: "candidate", candidateRevision: 1,
      suggestionId: "concept_1", feedbackKind: "useful", note: "", idempotencyKey: "key",
      createdAt: "now",
    }];
    expect(decodeAiCandidate(value)?.candidateKind).toBe("concept_suggestion_v1");
    (revisions[0]!.payload as Record<string, unknown>).conceptId = "invented-id";
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("decodes reference extraction candidates without treating identity as verified", () => {
    const value = candidate() as Record<string, unknown>;
    value.candidateKind = "reference_record_candidate_v1";
    const revisions = value.revisions as Array<Record<string, unknown>>;
    revisions[0]!.payload = {
      kind: "reference_record_candidate_v1",
      schemaVersion: 1,
      rawText: "Doe (2024). Grounded systems.",
      evidenceItemIds: ["ev-quote"],
      fields: [{ name: "title", value: "Grounded systems", evidenceItemIds: ["ev-quote"] }],
      uncertainties: ["Identity remains unverified."],
    };
    value.reviewState = "accepted_unchanged";
    value.decisions = [{
      id: "decision", candidateId: "candidate", candidateRevision: 1,
      action: "accept_unchanged", rationale: "reviewed", idempotencyKey: "decision-key",
      targetObjectKind: "reference_record", targetObjectId: "reference",
      targetObjectRevision: 1, createdAt: "now",
    }];
    const decoded = decodeAiCandidate(value);
    expect(decoded?.candidateKind).toBe("reference_record_candidate_v1");
    expect(decoded?.decisions[0]?.targetObjectKind).toBe("reference_record");
    (revisions[0]!.payload as Record<string, unknown>).identityState = "verified";
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("decodes multi-source synthesis candidates only within the frozen bundle", () => {
    const value = candidate() as Record<string, unknown>;
    value.candidateKind = "derived_research_view_v1";
    const bundle = value.evidenceBundle as Record<string, unknown>;
    const items = bundle.items as Array<Record<string, unknown>>;
    items.push({ ...items[0]!, id: "evidence-2", citationKey: "ev-second", ordinal: 1 });
    const revisions = value.revisions as Array<Record<string, unknown>>;
    revisions[0]!.payload = {
      kind: "derived_research_view_v1",
      schemaVersion: 1,
      viewKind: "taxonomy",
      title: "Routing taxonomy",
      scopeNote: "Selected matrix cells only.",
      items: [{
        id: "routing_1", text: "The sources use two routing families.",
        epistemicRole: "synthesis", evidenceItemIds: ["ev-quote", "ev-second"],
        hasConflict: false,
      }],
    };
    expect(decodeAiCandidate(value)?.candidateKind).toBe("derived_research_view_v1");
    const payload = revisions[0]!.payload as Record<string, unknown>;
    const viewItems = payload.items as Array<Record<string, unknown>>;
    viewItems[0]!.evidenceItemIds = ["ev-quote", "outside"];
    expect(decodeAiCandidate(value)).toBeUndefined();
  });

  it("rejects citations that are not present in the frozen evidence bundle", () => {
    expect(decodeAiCandidate(candidate(["unknown-evidence"]))).toBeUndefined();
  });

  it("decodes the frozen six-field paper overview candidate", () => {
    const overview = candidate() as Record<string, unknown>;
    overview.candidateKind = "paper_overview_unit_v1";
    const revisions = overview.revisions as Array<Record<string, unknown>>;
    revisions[0]!.payload = {
      kind: "paper_overview_unit_v1",
      schemaVersion: 1,
      profileId: "generic_overview_v1",
      fields: [
        { field: "research_question", status: "filled", statements: [{ id: "q1", text: "Question", epistemicRole: "source_paraphrase", evidenceItemIds: ["ev-quote"] }] },
        { field: "approach", status: "not_found", statements: [] },
        { field: "key_result", status: "not_found", statements: [] },
        { field: "conclusion", status: "not_found", statements: [] },
        { field: "limitation", status: "not_found", statements: [] },
        { field: "open_question", status: "not_found", statements: [] },
      ],
    };
    expect(decodeAiCandidate(overview)?.candidateKind).toBe("paper_overview_unit_v1");
  });

  it("decodes partial overview field confirmations and undo batches", () => {
    const overview = candidate() as Record<string, unknown>;
    overview.candidateKind = "paper_overview_unit_v1";
    overview.reviewState = "partially_accepted";
    const revisions = overview.revisions as Array<Record<string, unknown>>;
    revisions[0]!.payload = {
      kind: "paper_overview_unit_v1", schemaVersion: 1, profileId: "generic_overview_v1",
      fields: [
        { field: "research_question", status: "filled", statements: [{
          id: "q1", text: "Question", epistemicRole: "source_paraphrase",
          evidenceItemIds: ["ev-quote"],
        }] },
        { field: "approach", status: "not_found", statements: [] },
        { field: "key_result", status: "not_found", statements: [] },
        { field: "conclusion", status: "not_found", statements: [] },
        { field: "limitation", status: "not_found", statements: [] },
        { field: "open_question", status: "not_found", statements: [] },
      ],
    };
    const confirmation = {
      id: "field-confirmation", batchId: "batch", candidateId: "candidate",
      candidateRevision: 1, fieldKind: "research_question",
      fieldPayloadHash: "d".repeat(64), decisionKind: "ai_unchanged",
      targetReadingUnitId: "unit", targetAppliedRevision: 1,
      active: true, createdAt: "now",
    };
    overview.overviewFieldConfirmations = [confirmation];
    const result = decodeAiOverviewConfirmationResult({
      candidate: overview,
      batch: {
        id: "batch", candidateId: "candidate", candidateRevision: 1,
        scopeKind: "field", paperId: "paper", projectId: "project",
        idempotencyKey: "confirm-key", createdAt: "now",
        fieldConfirmations: [confirmation], active: true,
      },
    });
    expect(result?.candidate.reviewState).toBe("partially_accepted");
    expect(result?.batch.fieldConfirmations[0]?.decisionKind).toBe("ai_unchanged");
    expect(decodeAiOverviewConfirmationResult({
      candidate: overview,
      batch: {
        id: "batch", candidateId: "candidate", candidateRevision: 1,
        scopeKind: "field", paperId: "paper", idempotencyKey: "confirm-key",
        createdAt: "now", fieldConfirmations: [confirmation], active: false,
      },
    })).toBeUndefined();
  });

  it("decodes profile-driven fields and rejects duplicate profile references", () => {
    const overview = candidate() as Record<string, unknown>;
    overview.candidateKind = "paper_overview_unit_v1";
    const revisions = overview.revisions as Array<Record<string, unknown>>;
    revisions[0]!.payload = {
      kind: "paper_overview_unit_v1",
      schemaVersion: 1,
      profileId: "reviewed_profile_composition_v1",
      profileRefs: [{ profileId: "philosophical_conceptual", profileVersion: 1 }],
      fields: [{
        field: "research_question", status: "filled", statements: [{
          id: "argument_1", text: "论文给出一条论证。", epistemicRole: "source_paraphrase",
          evidenceItemIds: ["ev-quote"],
        }],
      },
      { field: "conclusion", status: "not_found", statements: [] },
      { field: "open_question", status: "not_found", statements: [] },
      { field: "argument", status: "not_found", statements: [] },
      { field: "objection", status: "not_found", statements: [] },
      { field: "response", status: "not_found", statements: [] }],
    };
    expect(decodeAiCandidate(overview)?.candidateKind).toBe("paper_overview_unit_v1");
    const payload = revisions[0]!.payload as Record<string, unknown>;
    payload.profileRefs = [
      { profileId: "philosophical_conceptual", profileVersion: 1 },
      { profileId: "philosophical_conceptual", profileVersion: 1 },
    ];
    expect(decodeAiCandidate(overview)).toBeUndefined();
  });

  it("decodes cross-module paper confirmation batches and partial failures", () => {
    const result = decodeAiPaperReviewConfirmationResult({
      batch: {
        id: "paper-batch", paperId: "paper", projectId: "project",
        idempotencyKey: "paper-confirm", active: true, createdAt: "now",
        metadataConfirmations: [{
          id: "metadata-confirmation", fieldName: "title", active: true, createdAt: "now",
        }],
        overviewBatchIds: ["overview-batch"],
      },
      candidates: [candidate()],
      moduleFailures: ["基础信息 title：版本冲突"],
    });
    expect(result?.batch.metadataConfirmations[0]?.fieldName).toBe("title");
    expect(result?.moduleFailures).toHaveLength(1);
    expect(decodeAiPaperReviewConfirmationResult({
      batch: {
        id: "paper-batch", paperId: "paper", idempotencyKey: "paper-confirm",
        active: false, createdAt: "now", metadataConfirmations: [], overviewBatchIds: [],
      },
      candidates: [], moduleFailures: [],
    })).toBeUndefined();
  });

  it("requires a source run for AI retry revisions", () => {
    const value = candidate() as Record<string, unknown>;
    const revisions = value.revisions as Array<Record<string, unknown>>;
    revisions.push({ ...revisions[0]!, id: "retry-revision", revision: 2,
      editorKind: "ai_retry", sourceRunId: "retry-run", parentRevision: 1 });
    value.currentRevision = 2;
    expect(decodeAiCandidate(value)?.revisions[1]?.sourceRunId).toBe("retry-run");
    delete revisions[1]!.sourceRunId;
    expect(decodeAiCandidate(value)).toBeUndefined();
  });
});
