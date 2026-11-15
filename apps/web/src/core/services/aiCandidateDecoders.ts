import { validatePortraitQuotes } from "@research-assistant/core/ai";
import {
  decodeAiCandidatePayload,
  type AiCandidate,
  type AiAttentionFeedback,
  type AiCandidateRevision,
  type AiCandidateSummaryVersion,
  type AiCitationAssessment,
  type AiEvidenceBundle,
  type AiEvidenceItem,
  type AiOverviewConfirmationBatch,
  type AiOverviewConfirmationResult,
  type AiOverviewFieldConfirmation,
  type AiPaperReviewConfirmationBatch,
  type AiPaperReviewConfirmationResult,
  type AiPaperReviewMetadataConfirmation,
  type AiReviewDecision,
  type AiSuggestionItemDecision,
} from "@research-assistant/core/ai";

type JsonRecord = Readonly<Record<string, unknown>>;

function record(value: unknown): JsonRecord | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as JsonRecord : undefined;
}
function string(value: unknown): string | undefined { return typeof value === "string" ? value : undefined; }
function integer(value: unknown): number | undefined { return Number.isSafeInteger(value) ? value as number : undefined; }
function boolean(value: unknown): boolean | undefined { return typeof value === "boolean" ? value : undefined; }
function optionalString(value: unknown): string | null | undefined { return value === undefined ? null : string(value); }
function optionalInteger(value: unknown): number | null | undefined { return value === undefined ? null : integer(value); }
function oneOf<const Values extends readonly string[]>(value: unknown, values: Values): Values[number] | undefined {
  return typeof value === "string" && values.includes(value) ? value as Values[number] : undefined;
}
function list<T>(value: unknown, decode: (item: unknown) => T | undefined): readonly T[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const result: T[] = [];
  for (const item of value) { const decoded = decode(item); if (decoded === undefined) return undefined; result.push(decoded); }
  return result;
}

export function decodeEvidenceItem(value: unknown): AiEvidenceItem | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const citationKey = string(data.citationKey); const ordinal = integer(data.ordinal);
  const anchorId = string(data.anchorId); const paperId = string(data.paperId); const paperTitle = string(data.paperTitle);
  const attachmentId = string(data.attachmentId); const contentSha256 = string(data.contentSha256);
  const pageIndex = integer(data.pageIndex); const pageLabel = optionalString(data.pageLabel);
  const quoteExact = string(data.quoteExact); const quotePrefix = string(data.quotePrefix); const quoteSuffix = string(data.quoteSuffix);
  const contextText = string(data.contextText); const relationKind = oneOf(data.relationKind, ["direct_quote", "context"] as const);
  const validationStatus = oneOf(data.validationStatus, ["valid", "invalid"] as const); const createdAt = string(data.createdAt);
  const currentAnchorStatus = oneOf(data.currentAnchorStatus, [
    "exact", "page_level", "stale", "missing",
  ] as const);
  if (id === undefined || citationKey === undefined || ordinal === undefined || anchorId === undefined
    || paperId === undefined || paperTitle === undefined || attachmentId === undefined
    || contentSha256 === undefined || pageIndex === undefined || pageLabel === undefined
    || quoteExact === undefined || quotePrefix === undefined || quoteSuffix === undefined
    || contextText === undefined || relationKind === undefined || validationStatus === undefined
    || currentAnchorStatus === undefined
    || createdAt === undefined) return undefined;
  return { id, citationKey, ordinal, anchorId, paperId, paperTitle, attachmentId,
    contentSha256, pageIndex, quoteExact, quotePrefix, quoteSuffix, contextText,
    relationKind, validationStatus, currentAnchorStatus, createdAt,
    ...(pageLabel === null ? {} : { pageLabel }) };
}

function decodeCitationAssessment(value: unknown): AiCitationAssessment | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const candidateId = string(data.candidateId);
  const candidateRevision = integer(data.candidateRevision); const statementId = string(data.statementId);
  const evidenceItemId = string(data.evidenceItemId); const assessmentRevision = integer(data.assessmentRevision);
  const semanticRelation = oneOf(data.semanticRelation, [
    "supports", "partially_supports", "context_only", "opposes", "unsupported", "uncertain",
  ] as const);
  const rationale = string(data.rationale); const idempotencyKey = string(data.idempotencyKey);
  const assessedAt = string(data.assessedAt);
  if (id === undefined || candidateId === undefined || candidateRevision === undefined || candidateRevision < 1
    || statementId === undefined || statementId.length < 1 || statementId.length > 160
    || evidenceItemId === undefined || assessmentRevision === undefined || assessmentRevision < 1
    || semanticRelation === undefined || rationale === undefined || rationale.length > 2_000
    || idempotencyKey === undefined || idempotencyKey.length < 1 || idempotencyKey.length > 200
    || assessedAt === undefined) return undefined;
  return { id, candidateId, candidateRevision, statementId, evidenceItemId, assessmentRevision,
    semanticRelation, rationale, idempotencyKey, assessedAt };
}

function decodeEvidenceBundle(value: unknown): AiEvidenceBundle | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const runId = string(data.runId); const scopeHash = string(data.scopeHash);
  const retrievalConfigVersion = string(data.retrievalConfigVersion);
  const validationStatus = oneOf(data.validationStatus, ["valid", "partial", "invalid"] as const);
  const createdAt = string(data.createdAt); const items = list(data.items, decodeEvidenceItem);
  return id === undefined || runId === undefined || scopeHash === undefined || retrievalConfigVersion === undefined
    || validationStatus === undefined || createdAt === undefined || items === undefined ? undefined
    : { id, runId, scopeHash, retrievalConfigVersion, validationStatus, createdAt, items };
}

function decodeOverviewFieldConfirmation(
  value: unknown,
): AiOverviewFieldConfirmation | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const batchId = string(data.batchId);
  const candidateId = string(data.candidateId);
  const candidateRevision = integer(data.candidateRevision);
  const fieldKind = string(data.fieldKind); const fieldPayloadHash = string(data.fieldPayloadHash);
  const decisionKind = oneOf(data.decisionKind, ["ai_unchanged", "human_edited"] as const);
  const targetReadingUnitId = string(data.targetReadingUnitId);
  const targetAppliedRevision = integer(data.targetAppliedRevision);
  const active = boolean(data.active); const createdAt = string(data.createdAt);
  const undoneAt = optionalString(data.undoneAt);
  if (id === undefined || batchId === undefined || candidateId === undefined
    || candidateRevision === undefined || candidateRevision < 1
    || fieldKind === undefined || fieldKind.length === 0
    || fieldPayloadHash === undefined || fieldPayloadHash.length !== 64
    || decisionKind === undefined || targetReadingUnitId === undefined
    || targetAppliedRevision === undefined || targetAppliedRevision < 1
    || active === undefined || createdAt === undefined || undoneAt === undefined
    || active === (undoneAt !== null)) return undefined;
  return { id, batchId, candidateId, candidateRevision, fieldKind, fieldPayloadHash,
    decisionKind, targetReadingUnitId, targetAppliedRevision, active, createdAt,
    ...(undoneAt === null ? {} : { undoneAt }) };
}

function decodeOverviewConfirmationBatch(value: unknown): AiOverviewConfirmationBatch | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const candidateId = string(data.candidateId);
  const candidateRevision = integer(data.candidateRevision);
  const scopeKind = oneOf(data.scopeKind, ["field", "block", "paper"] as const);
  const paperId = string(data.paperId); const projectId = optionalString(data.projectId);
  const idempotencyKey = string(data.idempotencyKey); const createdAt = string(data.createdAt);
  const fieldConfirmations = list(data.fieldConfirmations, decodeOverviewFieldConfirmation);
  const active = boolean(data.active); const undoneAt = optionalString(data.undoneAt);
  if (id === undefined || candidateId === undefined || candidateRevision === undefined
    || candidateRevision < 1 || scopeKind === undefined || paperId === undefined
    || projectId === undefined || idempotencyKey === undefined || createdAt === undefined
    || fieldConfirmations === undefined || fieldConfirmations.length === 0
    || active === undefined || undoneAt === undefined || active === (undoneAt !== null)
    || fieldConfirmations.some((field) => field.batchId !== id
      || field.candidateId !== candidateId || field.candidateRevision !== candidateRevision)) {
    return undefined;
  }
  return { id, candidateId, candidateRevision, scopeKind, paperId, idempotencyKey,
    createdAt, fieldConfirmations, active,
    ...(projectId === null ? {} : { projectId }),
    ...(undoneAt === null ? {} : { undoneAt }) };
}

function decodeDecision(value: unknown): AiReviewDecision | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const candidateId = string(data.candidateId); const candidateRevision = integer(data.candidateRevision);
  const action = oneOf(data.action, ["accept_unchanged", "accept_after_edit", "reject", "defer", "request_more_evidence"] as const);
  const rationale = string(data.rationale); const idempotencyKey = string(data.idempotencyKey);
  const targetObjectKind = data.targetObjectKind === undefined
    ? null
    : oneOf(data.targetObjectKind, [
      "reading_unit", "reference_record", "derived_research_view", "paper_portrait",
    ] as const);
  const targetObjectId = optionalString(data.targetObjectId);
  const targetObjectRevision = optionalInteger(data.targetObjectRevision); const createdAt = string(data.createdAt);
  if (id === undefined || candidateId === undefined || candidateRevision === undefined || action === undefined
    || rationale === undefined || idempotencyKey === undefined || targetObjectKind === undefined
    || targetObjectId === undefined || targetObjectRevision === undefined || createdAt === undefined)
    return undefined;
  return { id, candidateId, candidateRevision, action, rationale, idempotencyKey, createdAt,
    ...(targetObjectKind === null ? {} : { targetObjectKind }),
    ...(targetObjectId === null ? {} : { targetObjectId }),
    ...(targetObjectRevision === null ? {} : { targetObjectRevision }) };
}

export function decodeAiAttentionFeedback(value: unknown): AiAttentionFeedback | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const candidateId = string(data.candidateId);
  const candidateRevision = integer(data.candidateRevision); const suggestionId = string(data.suggestionId);
  const feedbackKind = oneOf(data.feedbackKind, ["useful", "not_useful", "dismissed"] as const);
  const note = string(data.note); const idempotencyKey = string(data.idempotencyKey);
  const createdAt = string(data.createdAt);
  return id === undefined || candidateId === undefined || candidateRevision === undefined
    || candidateRevision < 1 || suggestionId === undefined
    || Array.from(suggestionId).length < 1 || Array.from(suggestionId).length > 80
    || feedbackKind === undefined || note === undefined || Array.from(note).length > 1_000
    || idempotencyKey === undefined || idempotencyKey.length < 1 || idempotencyKey.length > 200
    || createdAt === undefined ? undefined : {
      id, candidateId, candidateRevision, suggestionId, feedbackKind, note, idempotencyKey, createdAt,
    };
}

export function decodeAiSuggestionItemDecision(
  value: unknown,
): AiSuggestionItemDecision | undefined {
  const data = record(value);
  if (data === undefined) return undefined;
  const id = string(data.id);
  const candidateId = string(data.candidateId);
  const candidateRevision = integer(data.candidateRevision);
  const suggestionId = string(data.suggestionId);
  const action = oneOf(data.action, [
    "accept_unchanged",
    "accept_after_edit",
    "reject",
    "defer",
    "request_more_evidence",
  ] as const);
  const resolutionKind = oneOf(data.resolutionKind, [
    "none",
    "create_concept",
    "link_concept",
    "unresolved",
    "attach_definition",
    "assert_relation",
  ] as const);
  const rationale = string(data.rationale);
  const idempotencyKey = string(data.idempotencyKey);
  const targetObjectKind = data.targetObjectKind === undefined
    ? null
    : oneOf(data.targetObjectKind, [
        "concept",
        "term_usage",
        "definition_claim",
        "relation_assertion",
      ] as const);
  const targetObjectId = optionalString(data.targetObjectId);
  const targetObjectRevision = optionalInteger(data.targetObjectRevision);
  const createdAt = string(data.createdAt);
  const hasTarget = targetObjectKind !== null && targetObjectId !== null && targetObjectRevision !== null;
  const accepts = action === "accept_unchanged" || action === "accept_after_edit";
  const resolutionTargetMatches =
    resolutionKind === "create_concept" || resolutionKind === "link_concept"
      ? targetObjectKind === "concept"
      : resolutionKind === "unresolved"
        ? targetObjectKind === "term_usage"
        : resolutionKind === "attach_definition"
          ? targetObjectKind === "definition_claim"
          : resolutionKind === "assert_relation"
            ? targetObjectKind === "relation_assertion"
            : !hasTarget;
  if (
    id === undefined
    || candidateId === undefined
    || candidateRevision === undefined
    || candidateRevision < 1
    || suggestionId === undefined
    || suggestionId.length < 1
    || suggestionId.length > 80
    || action === undefined
    || resolutionKind === undefined
    || rationale === undefined
    || idempotencyKey === undefined
    || targetObjectKind === undefined
    || targetObjectId === undefined
    || targetObjectRevision === undefined
    || createdAt === undefined
    || accepts !== hasTarget
    || accepts === (resolutionKind === "none")
    || !resolutionTargetMatches
  ) {
    return undefined;
  }
  return {
    id,
    candidateId,
    candidateRevision,
    suggestionId,
    action,
    resolutionKind,
    rationale,
    idempotencyKey,
    createdAt,
    ...(targetObjectKind === null ? {} : { targetObjectKind }),
    ...(targetObjectId === null ? {} : { targetObjectId }),
    ...(targetObjectRevision === null ? {} : { targetObjectRevision }),
  };
}

function decodeCandidateSummaryVersion(value: unknown): AiCandidateSummaryVersion | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const candidateId = string(data.candidateId);
  const candidateRevision = integer(data.candidateRevision); const version = integer(data.version);
  const summary = string(data.summary);
  const sourceKind = oneOf(data.sourceKind, ["local_extractive", "ai_generated", "human"] as const);
  const isSelected = boolean(data.isSelected); const createdAt = string(data.createdAt);
  const discardedAt = optionalString(data.discardedAt);
  if (id === undefined || candidateId === undefined || candidateRevision === undefined
    || candidateRevision < 1 || version === undefined || version < 1
    || summary === undefined || Array.from(summary.trim()).length < 1
    || Array.from(summary).length > 1_200 || sourceKind === undefined
    || isSelected === undefined || createdAt === undefined || discardedAt === undefined
    || (discardedAt !== null && isSelected)) return undefined;
  return {
    id,
    candidateId,
    candidateRevision,
    version,
    summary,
    sourceKind,
    isSelected,
    createdAt,
    ...(discardedAt === null ? {} : { discardedAt }),
  };
}

function payloadSuggestionIds(revision: AiCandidateRevision): ReadonlySet<string> {
  const payload = revision.payload;
  if (payload.kind === "paper_portrait_v1") return new Set(payload.claims.map(claim => claim.id));
  if (payload.kind === "selection_explanation_v1") {
    return new Set(payload.statements.map((statement) => statement.id));
  }
  if (payload.kind === "paper_overview_unit_v1") {
    return new Set(payload.fields.flatMap((field) =>
      field.statements.map((statement) => statement.id)));
  }
  if (payload.kind === "paper_concept_map_v1") {
    return new Set([
      ...payload.concepts.map((concept) => concept.id),
      ...payload.relations.map((relation) => relation.id),
    ]);
  }
  if (payload.kind === "reference_record_candidate_v1") return new Set();
  if (payload.kind === "derived_research_view_v1") {
    return new Set(payload.items.map((item) => item.id));
  }
  return new Set(payload.suggestions.map((suggestion) => suggestion.id));
}

function payloadCitationTargets(revision: AiCandidateRevision): ReadonlyMap<string, ReadonlySet<string>> {
  const targets = new Map<string, ReadonlySet<string>>();
  const payload = revision.payload;
  if (payload.kind === "paper_portrait_v1") {
    for (const claim of payload.claims) claim.evidence.forEach((source, index) => targets.set(`${claim.id}:e${index}`, new Set([source.evidenceItemId])));
    return targets;
  }
  if (payload.kind === "selection_explanation_v1") {
    for (const item of payload.statements) targets.set(item.id, new Set(item.evidenceItemIds));
  } else if (payload.kind === "paper_overview_unit_v1") {
    for (const field of payload.fields) {
      for (const item of field.statements) targets.set(item.id, new Set(item.evidenceItemIds));
    }
  } else if (payload.kind === "paper_concept_map_v1") {
    for (const concept of payload.concepts) {
      targets.set(concept.id, new Set(concept.evidenceItemIds));
      for (const definition of concept.definitionClaims) {
        targets.set(definition.id, new Set(definition.evidenceItemIds));
      }
    }
    for (const relation of payload.relations) {
      targets.set(relation.id, new Set(relation.evidenceItemIds));
    }
  } else if (payload.kind === "reference_record_candidate_v1") {
    targets.set("reference-record", new Set(payload.evidenceItemIds));
    for (const field of payload.fields) {
      targets.set(`reference-field:${field.name}`, new Set(field.evidenceItemIds));
    }
  } else if (payload.kind === "derived_research_view_v1") {
    for (const item of payload.items) targets.set(item.id, new Set(item.evidenceItemIds));
  } else {
    for (const item of payload.suggestions) targets.set(item.id, new Set(item.evidenceItemIds));
  }
  return targets;
}

function decodeRevision(value: unknown, allowedEvidenceIds: ReadonlySet<string>): AiCandidateRevision | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const revision = integer(data.revision); const payloadHash = string(data.payloadHash);
  const editorKind = oneOf(data.editorKind, ["ai", "human", "ai_retry"] as const);
  const sourceRunId = optionalString(data.sourceRunId); const parentRevision = optionalInteger(data.parentRevision);
  const changeReason = string(data.changeReason); const createdAt = string(data.createdAt);
  if (id === undefined || revision === undefined || payloadHash === undefined || editorKind === undefined
    || sourceRunId === undefined || parentRevision === undefined || changeReason === undefined
    || createdAt === undefined || (editorKind === "human" && sourceRunId !== null)
    || (editorKind !== "human" && sourceRunId === null)) return undefined;
  try {
    const payload = decodeAiCandidatePayload(data.payload, { allowedEvidenceItemIds: allowedEvidenceIds });
    return { id, revision, payload, payloadHash, editorKind, changeReason, createdAt,
      ...(sourceRunId === null ? {} : { sourceRunId }),
      ...(parentRevision === null ? {} : { parentRevision }) };
  } catch { return undefined; }
}

export function decodeAiCandidate(value: unknown): AiCandidate | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const evidenceBundle = decodeEvidenceBundle(data.evidenceBundle); if (evidenceBundle === undefined) return undefined;
  const allowedEvidenceIds = new Set(evidenceBundle.items.map((item) => item.citationKey));
  const revisions = list(data.revisions, (item) => decodeRevision(item, allowedEvidenceIds));
  try { for (const revision of revisions ?? []) {
    if (revision.payload.kind === "paper_portrait_v1") validatePortraitQuotes(revision.payload, evidenceBundle.items);
    if (revision.payload.kind === "paper_overview_unit_v1") for (const statement of revision.payload.fields.flatMap(field => field.statements)) for (const source of statement.evidenceQuotes ?? []) {
      if (!evidenceBundle.items.some(item => item.citationKey === source.evidenceItemId && item.quoteExact.includes(source.quote))) return undefined;
    }
  } } catch { return undefined; }
  const decisions = list(data.decisions, decodeDecision);
  const attentionFeedback = list(data.attentionFeedback, decodeAiAttentionFeedback);
  const suggestionItemDecisions = list(
    data.suggestionItemDecisions,
    decodeAiSuggestionItemDecision,
  );
  const citationAssessments = list(data.citationAssessments, decodeCitationAssessment);
  const inspectedEvidenceItemIds = data.inspectedEvidenceItemIds === undefined
    ? []
    : list(data.inspectedEvidenceItemIds, string);
  const overviewFieldConfirmations = list(
    data.overviewFieldConfirmations,
    decodeOverviewFieldConfirmation,
  );
  const summaryVersions = data.summaryVersions === undefined
    ? []
    : list(data.summaryVersions, decodeCandidateSummaryVersion);
  const id = string(data.id); const runId = string(data.runId); const providerId = string(data.providerId); const modelId = string(data.modelId);
  const candidateKind = oneOf(data.candidateKind, [
    "paper_portrait_v1", "selection_explanation_v1", "paper_overview_unit_v1", "concept_suggestion_v1",
    "paper_concept_map_v1",
    "definition_suggestion_v1", "relation_suggestion_v1", "attention_suggestion_v1",
    "reference_record_candidate_v1", "derived_research_view_v1",
  ] as const); const schemaVersion = integer(data.schemaVersion);
  const currentRevision = integer(data.currentRevision);
  const reviewState = oneOf(data.reviewState, ["unreviewed", "edited", "partially_accepted", "deferred", "needs_more_evidence", "accepted_unchanged", "accepted_after_edit", "rejected"] as const);
  const validationStatus = oneOf(data.validationStatus, ["valid"] as const); const originalPayloadHash = string(data.originalPayloadHash);
  const createdAt = string(data.createdAt); const updatedAt = string(data.updatedAt);
  const hasValidRevisionChain = revisions?.every((revision, index) =>
    revision.revision === index + 1
    && (revision.revision === 1
      ? revision.editorKind === "ai" && revision.parentRevision === undefined
      : revision.editorKind !== "ai" && revision.parentRevision === revision.revision - 1));
  const hasValidAttentionFeedback = attentionFeedback?.every((feedback) => {
    const revision = revisions?.find((item) => item.revision === feedback.candidateRevision);
    return feedback.candidateId === id && revision !== undefined
      && payloadSuggestionIds(revision).has(feedback.suggestionId);
  });
  const hasValidSuggestionItemDecisions = suggestionItemDecisions?.every((decision) => {
    const revision = revisions?.find((item) => item.revision === decision.candidateRevision);
    return decision.candidateId === id && revision !== undefined
      && payloadSuggestionIds(revision).has(decision.suggestionId);
  });
  const evidenceCitationKeys = new Map(
    evidenceBundle.items.map((item) => [item.id, item.citationKey] as const),
  );
  const hasValidCitationAssessments = citationAssessments?.every((assessment) => {
    const revision = revisions?.find((item) => item.revision === assessment.candidateRevision);
    const citationKey = evidenceCitationKeys.get(assessment.evidenceItemId);
    const target = revision === undefined ? undefined : payloadCitationTargets(revision).get(assessment.statementId);
    return assessment.candidateId === id && citationKey !== undefined && target?.has(citationKey) === true;
  });
  const hasValidSummaryVersions = summaryVersions?.every((summary) =>
    summary.candidateId === id && revisions?.some((revision) =>
      revision.revision === summary.candidateRevision) === true);
  const selectedSummaryKeys = new Set<string>();
  const hasAtMostOneSelectedSummary = summaryVersions?.every((summary) => {
    if (!summary.isSelected || summary.discardedAt !== undefined) return true;
    const key = `${summary.candidateId}:${summary.candidateRevision}`;
    if (selectedSummaryKeys.has(key)) return false;
    selectedSummaryKeys.add(key);
    return true;
  });
  if (id === undefined || runId === undefined || providerId === undefined || modelId === undefined
    || candidateKind === undefined || schemaVersion !== 1 || currentRevision === undefined
    || reviewState === undefined || validationStatus === undefined || originalPayloadHash === undefined
    || createdAt === undefined || updatedAt === undefined || revisions === undefined || decisions === undefined
    || attentionFeedback === undefined || suggestionItemDecisions === undefined || citationAssessments === undefined
    || overviewFieldConfirmations === undefined || summaryVersions === undefined
    || inspectedEvidenceItemIds === undefined
    || !hasValidRevisionChain || !hasValidAttentionFeedback || !hasValidSuggestionItemDecisions
    || !hasValidCitationAssessments || !hasValidSummaryVersions || !hasAtMostOneSelectedSummary
    || inspectedEvidenceItemIds.some((evidenceItemId) => !evidenceCitationKeys.has(evidenceItemId))
    || overviewFieldConfirmations.some((confirmation) => confirmation.candidateId !== id)
    || revisions.at(-1)?.revision !== currentRevision) return undefined;
  return { id, runId, providerId, modelId, candidateKind, schemaVersion: 1, currentRevision,
    reviewState, validationStatus, originalPayloadHash, createdAt, updatedAt,
    evidenceBundle, revisions, decisions, attentionFeedback, suggestionItemDecisions,
    citationAssessments, inspectedEvidenceItemIds, overviewFieldConfirmations, summaryVersions };
}

export function decodeAiCandidateListBestEffort(value: unknown): readonly AiCandidate[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const result: AiCandidate[] = [];
  for (const item of value) {
    const decoded = decodeAiCandidate(item);
    if (decoded !== undefined) result.push(decoded);
  }
  return result;
}

export function decodeAiOverviewConfirmationResult(
  value: unknown,
): AiOverviewConfirmationResult | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const candidate = decodeAiCandidate(data.candidate);
  const batch = decodeOverviewConfirmationBatch(data.batch);
  if (candidate === undefined || batch === undefined || batch.candidateId !== candidate.id) {
    return undefined;
  }
  return { candidate, batch };
}

function decodePaperReviewMetadataConfirmation(
  value: unknown,
): AiPaperReviewMetadataConfirmation | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const fieldName = string(data.fieldName);
  const active = boolean(data.active); const createdAt = string(data.createdAt);
  const undoneAt = optionalString(data.undoneAt);
  if (id === undefined || fieldName === undefined || active === undefined
    || createdAt === undefined || undoneAt === undefined || active !== (undoneAt === null)) {
    return undefined;
  }
  return { id, fieldName, active, createdAt, ...(undoneAt === null ? {} : { undoneAt }) };
}

function decodePaperReviewConfirmationBatch(
  value: unknown,
): AiPaperReviewConfirmationBatch | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const id = string(data.id); const paperId = string(data.paperId);
  const projectId = optionalString(data.projectId); const idempotencyKey = string(data.idempotencyKey);
  const metadataConfirmations = list(data.metadataConfirmations, decodePaperReviewMetadataConfirmation);
  const overviewBatchIds = list(data.overviewBatchIds, string);
  const active = boolean(data.active); const createdAt = string(data.createdAt);
  const undoneAt = optionalString(data.undoneAt);
  if (id === undefined || paperId === undefined || projectId === undefined
    || idempotencyKey === undefined || metadataConfirmations === undefined
    || overviewBatchIds === undefined || new Set(overviewBatchIds).size !== overviewBatchIds.length
    || active === undefined || createdAt === undefined || undoneAt === undefined
    || active !== (undoneAt === null)) return undefined;
  return {
    id, paperId, idempotencyKey, metadataConfirmations, overviewBatchIds, active, createdAt,
    ...(projectId === null ? {} : { projectId }),
    ...(undoneAt === null ? {} : { undoneAt }),
  };
}

export function decodeAiPaperReviewConfirmationResult(
  value: unknown,
): AiPaperReviewConfirmationResult | undefined {
  const data = record(value); if (data === undefined) return undefined;
  const batch = decodePaperReviewConfirmationBatch(data.batch);
  const candidates = list(data.candidates, decodeAiCandidate);
  const moduleFailures = list(data.moduleFailures, string);
  if (batch === undefined || candidates === undefined || moduleFailures === undefined
    || candidates.some((candidate) => candidate.evidenceBundle.items.some(
      (item) => item.paperId !== batch.paperId,
    ))) return undefined;
  return { batch, candidates, moduleFailures };
}
