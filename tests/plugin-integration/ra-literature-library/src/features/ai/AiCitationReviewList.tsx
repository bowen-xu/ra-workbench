import "../../../../../../../plugins/ra-literature-library/src/features/ai/AiCandidateReview.css";
import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  AiCandidate,
  AiCitationAssessment,
  AiCitationSemanticRelation,
  AiEvidenceItem,
} from "@research-assistant/core/ai";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { aiAssistanceErrorMessage } from "../../../../../../apps/web/src/core/public/aiAssistance";

interface AiCitationReviewListProps {
  readonly candidate: AiCandidate;
  readonly evidenceItemIds: readonly string[];
  readonly gateway: AiAssistanceGateway;
  readonly onChanged: (candidate: AiCandidate, didCreateKnowledge: boolean) => void;
  readonly onOpenEvidence: (evidence: AiEvidenceItem) => Promise<void>;
  readonly statementId: string;
}

const anchorLabels = {
  exact: "精确锚点",
  page_level: "页级定位（已降级）",
  stale: "失效（原文已变化）",
  missing: "来源缺失",
} as const;

const relationLabels: Readonly<Record<AiCitationSemanticRelation, string>> = {
  supports: "支持",
  partially_supports: "部分支持",
  context_only: "仅提供背景",
  opposes: "反对",
  unsupported: "不支持",
  uncertain: "无法判断",
};

const terminalStates = new Set(["accepted_unchanged", "accepted_after_edit", "rejected"]);

function latestAssessment(
  candidate: AiCandidate,
  statementId: string,
  evidenceItemId: string,
): AiCitationAssessment | undefined {
  return candidate.citationAssessments
    .filter((item) => item.candidateRevision === candidate.currentRevision
      && item.statementId === statementId && item.evidenceItemId === evidenceItemId)
    .sort((left, right) => right.assessmentRevision - left.assessmentRevision)[0];
}

export function isCitationTargetReady(
  candidate: AiCandidate,
  statementId: string,
  evidenceItemIds: readonly string[],
): boolean {
  if (evidenceItemIds.length === 0) return true;
  let hasPositive = false;
  for (const citationKey of evidenceItemIds) {
    const evidence = candidate.evidenceBundle.items.find((item) => item.citationKey === citationKey);
    if (evidence === undefined || evidence.currentAnchorStatus === "stale"
      || evidence.currentAnchorStatus === "missing") return false;
    const relation = latestAssessment(candidate, statementId, evidence.id)?.semanticRelation;
    if (relation === "opposes" || relation === "unsupported") return false;
    hasPositive ||= relation === "supports" || relation === "partially_supports";
  }
  return hasPositive;
}

export function isCandidateCitationReady(candidate: AiCandidate): boolean {
  const revision = candidate.revisions.find((item) => item.revision === candidate.currentRevision);
  const payload = revision?.payload;
  if (payload === undefined) return false;
  // Portrait adoption is an optional choice, not a claim of human source verification.
  if (payload.kind === "paper_portrait_v1") return true;
  if (payload.kind === "selection_explanation_v1") {
    return payload.statements.every((item) =>
      isCitationTargetReady(candidate, item.id, item.evidenceItemIds));
  }
  if (payload.kind === "paper_overview_unit_v1") {
    return payload.fields.flatMap((field) => field.statements).every((item) =>
      isCitationTargetReady(candidate, item.id, item.evidenceItemIds));
  }
  if (payload.kind === "paper_concept_map_v1") {
    return payload.concepts.every((concept) =>
      isCitationTargetReady(candidate, concept.id, concept.evidenceItemIds)
      && concept.definitionClaims.every((definition) =>
        isCitationTargetReady(candidate, definition.id, definition.evidenceItemIds)))
      && payload.relations.every((relation) =>
        isCitationTargetReady(candidate, relation.id, relation.evidenceItemIds));
  }
  if (payload.kind === "reference_record_candidate_v1") {
    return isCitationTargetReady(candidate, "reference-record", payload.evidenceItemIds)
      && payload.fields.every((field) => isCitationTargetReady(
        candidate, `reference-field:${field.name}`, field.evidenceItemIds,
      ));
  }
  if (payload.kind === "derived_research_view_v1") {
    return payload.items.every((item) =>
      isCitationTargetReady(candidate, item.id, item.evidenceItemIds));
  }
  return payload.suggestions.every((item) =>
    isCitationTargetReady(candidate, item.id, item.evidenceItemIds));
}

function CitationRow({
  candidate,
  evidence,
  gateway,
  onChanged,
  onOpenEvidence,
  statementId,
}: Omit<AiCitationReviewListProps, "evidenceItemIds"> & {
  readonly evidence: AiEvidenceItem;
}): React.JSX.Element {
  const current = useMemo(
    () => latestAssessment(candidate, statementId, evidence.id),
    [candidate, evidence.id, statementId],
  );
  const [relation, setRelation] = useState<AiCitationSemanticRelation | "">(
    current?.semanticRelation ?? "",
  );
  const [rationale, setRationale] = useState(current?.rationale ?? "");
  const [busy, setBusy] = useState(false);
  const idempotencyKey = useRef<string | undefined>(undefined);
  const isTerminal = terminalStates.has(candidate.reviewState);

  useEffect(() => {
    setRelation(current?.semanticRelation ?? "");
    setRationale(current?.rationale ?? "");
    idempotencyKey.current = undefined;
  }, [current?.assessmentRevision, current?.rationale, current?.semanticRelation]);

  async function save(): Promise<void> {
    if (relation === "" || isTerminal) return;
    setBusy(true);
    try {
      idempotencyKey.current ??=
        `${candidate.id}:${candidate.currentRevision}:${statementId}:${evidence.id}:${globalThis.crypto.randomUUID()}`;
      const updated = await gateway.recordCitationAssessment({
        candidateId: candidate.id,
        candidateRevision: candidate.currentRevision,
        statementId,
        evidenceItemId: evidence.id,
        expectedAssessmentRevision: current?.assessmentRevision ?? 0,
        semanticRelation: relation,
        rationale,
        idempotencyKey: idempotencyKey.current,
      });
      onChanged(updated, false);
    } catch (reason) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  const unchanged = current?.semanticRelation === relation && current.rationale === rationale.trim();
  return <div className={`ai-citation-row anchor-${evidence.currentAnchorStatus}`}>
    <button className="ai-citation-source" onClick={() => void onOpenEvidence(evidence)} type="button">
      <b>{evidence.citationKey}</b>
      <q>{evidence.relationKind === "direct_quote" ? evidence.quoteExact : evidence.contextText}</q>
      <small>第 {evidence.pageLabel ?? evidence.pageIndex + 1} 页 · 回到原文 →</small>
    </button>
    <div className="ai-citation-statuses">
      <span>{anchorLabels[evidence.currentAnchorStatus]}</span>
      <span>{current === undefined ? "人工语义：未审查" : `人工语义：${relationLabels[current.semanticRelation]} · v${current.assessmentRevision}`}</span>
    </div>
    {!isTerminal && <div className="ai-citation-assessment">
      <label>人工语义判断<select aria-label={`${evidence.citationKey} 人工语义判断`} disabled={busy} onChange={(event) => setRelation(event.target.value as AiCitationSemanticRelation | "")} value={relation}>
        <option value="">未审查</option>
        {Object.entries(relationLabels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
      </select></label>
      <label>判断说明（可选）<input disabled={busy} onChange={(event) => setRationale(event.target.value)} value={rationale} /></label>
      <button disabled={busy || relation === "" || unchanged} onClick={() => void save()} type="button">保存判断</button>
    </div>}
  </div>;
}

export function AiCitationReviewList(props: AiCitationReviewListProps): React.JSX.Element {
  if (props.evidenceItemIds.length === 0) {
    return <div className="ai-citation-review-list"><div className="ai-citation-empty">
      未附引用；此陈述保持“不确定”，没有被标记为有依据。
    </div></div>;
  }
  return <div className="ai-citation-review-list">
    {props.evidenceItemIds.map((citationKey) => {
      const evidence = props.candidate.evidenceBundle.items.find((item) =>
        item.citationKey === citationKey);
      return evidence === undefined
        ? <p key={citationKey}>缺失引用 {citationKey}</p>
        : <CitationRow {...props} evidence={evidence} key={evidence.id} />;
    })}
  </div>;
}
