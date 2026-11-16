import { useEffect, useRef, useState } from "react";
import type {
  AiCandidate,
  AiEvidenceItem,
  AiReviewAction,
  AiSuggestionResolutionDraft,
  PaperConceptCandidateV1,
  PaperConceptMapCandidateV1,
  PaperConceptRelationCandidateV1,
} from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { aiAssistanceErrorMessage } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { useEvidenceInspectionState } from "../../../../../../../plugins/ra-literature-library/src/features/ai/useEvidenceInspectionState";
import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { AiCitationReviewList, isCitationTargetReady } from "./AiCitationReviewList";
import { ConceptTargetPicker } from "../../../../../../../plugins/ra-literature-library/src/features/ai/ConceptTargetPicker";
import { ReviewConfirmationDialog } from "../../../../../../../plugins/ra-literature-library/src/features/ai/ReviewConfirmationDialog";

interface Props {
  readonly candidates: readonly AiCandidate[];
  readonly gateway: AiAssistanceGateway;
  readonly item: LiteratureItem;
  readonly onCandidatesChanged: (candidates: readonly AiCandidate[]) => void;
  readonly onOpenAttachment: (attachment: LiteratureItem["attachments"][number]) => void;
}

interface ConceptTarget {
  readonly mode?: "create_concept" | "link_concept" | "unresolved";
  readonly conceptId?: string;
}

type BatchAction = "accept_ready" | "reject_all";

export interface PaperConceptCandidateStats {
  readonly conceptCount: number;
  readonly pendingCount: number;
}

const groupOrder = ["focal", "foundational", "prerequisite"] as const;
const groupLabels = {
  focal: "核心概念",
  foundational: "基础概念",
  prerequisite: "前置／依赖概念",
} as const;
const noveltyLabels = {
  pre_existing: "已有概念",
  adapted_by_paper: "本文调整",
  introduced_by_paper: "本文提出",
  unclear: "新颖性不明",
} as const;
const explanationLabels = {
  explained_in_paper: "文中有解释",
  mentioned_only: "仅提及",
  insufficient_evidence: "证据不足",
} as const;
const relationLabels = {
  depends_on: "依赖",
  builds_on: "基于",
  extends: "扩展",
  redefines: "重新定义",
  contrasts_with: "对照",
  part_of: "属于",
} as const;

function payload(candidate: AiCandidate): PaperConceptMapCandidateV1 | undefined {
  const current = candidate.revisions.find((revision) =>
    revision.revision === candidate.currentRevision)?.payload;
  return current?.kind === "paper_concept_map_v1" ? current : undefined;
}

export function isPaperConceptCandidate(candidate: AiCandidate, paperId: string): boolean {
  return payload(candidate) !== undefined && candidate.evidenceBundle.items.length > 0
    && candidate.evidenceBundle.items.every((evidence) => evidence.paperId === paperId);
}

function latestPaperConceptCandidate(
  candidates: readonly AiCandidate[],
  paperId: string,
): AiCandidate | undefined {
  return [...candidates]
    .filter((value) => isPaperConceptCandidate(value, paperId))
    .sort((left, right) => Date.parse(right.updatedAt) - Date.parse(left.updatedAt))[0];
}

export function paperConceptCandidateStats(
  candidates: readonly AiCandidate[],
  paperId: string,
): PaperConceptCandidateStats | undefined {
  const candidate = latestPaperConceptCandidate(candidates, paperId);
  const conceptMap = candidate === undefined ? undefined : payload(candidate);
  if (candidate === undefined || conceptMap === undefined) return undefined;
  const decisions = candidate.suggestionItemDecisions.filter((decision) =>
    decision.candidateRevision === candidate.currentRevision);
  const terminalIds = new Set(decisions.filter((decision) =>
    ["accept_unchanged", "accept_after_edit", "reject"].includes(decision.action))
    .map((decision) => decision.suggestionId));
  return {
    conceptCount: conceptMap.concepts.length,
    pendingCount: conceptMap.concepts.filter((concept) => !terminalIds.has(concept.id)).length,
  };
}

export function PaperConceptMapCandidatePanel({
  candidates,
  gateway,
  item,
  onCandidatesChanged,
  onOpenAttachment,
}: Props): React.JSX.Element | null {
  const candidate = latestPaperConceptCandidate(candidates, item.id);
  if (candidate === undefined) return null;
  return <PaperConceptCandidateCard candidate={candidate} candidates={candidates}
    gateway={gateway} item={item} onCandidatesChanged={onCandidatesChanged}
    onOpenAttachment={onOpenAttachment} />;
}

function PaperConceptCandidateCard({
  candidate,
  candidates,
  gateway,
  item,
  onCandidatesChanged,
  onOpenAttachment,
}: Props & { readonly candidate: AiCandidate }): React.JSX.Element | null {
  const conceptMap = payload(candidate);
  const [targets, setTargets] = useState<Readonly<Record<string, ConceptTarget>>>({});
  const [rationales, setRationales] = useState<Readonly<Record<string, string>>>({});
  const [openedEvidenceIds, markEvidenceInspected] = useEvidenceInspectionState(candidate);
  const [busyItemId, setBusyItemId] = useState<string>();
  const [batchAction, setBatchAction] = useState<BatchAction>();
  const [batchBusy, setBatchBusy] = useState(false);
  const keys = useRef(new Map<string, string>());

  useEffect(() => {
    setTargets({});
    setRationales({});
  }, [candidate.id, candidate.currentRevision]);

  if (conceptMap === undefined) return null;
  const stableConceptMap = conceptMap;
  const terminal = ["accepted_unchanged", "accepted_after_edit", "rejected"]
    .includes(candidate.reviewState);

  function replace(updated: AiCandidate): void {
    onCandidatesChanged(candidates.map((value) => value.id === updated.id ? updated : value));
  }

  function decisionForCandidate(source: AiCandidate, id: string) {
    return [...source.suggestionItemDecisions].reverse().find((decision) =>
      decision.candidateRevision === source.currentRevision && decision.suggestionId === id);
  }

  function decisionFor(id: string) {
    return decisionForCandidate(candidate, id);
  }

  function isItemTerminalFor(source: AiCandidate, id: string): boolean {
    return ["accept_unchanged", "accept_after_edit", "reject"]
      .includes(decisionForCandidate(source, id)?.action ?? "");
  }

  function isItemTerminal(id: string): boolean {
    return isItemTerminalFor(candidate, id);
  }

  async function openEvidence(evidence: AiEvidenceItem): Promise<void> {
    try {
      const attachment = item.attachments.find((value) => value.id === evidence.attachmentId);
      if (attachment === undefined) throw new Error("找不到来源 PDF 附件");
      onOpenAttachment(attachment);
      await gateway.recordEvidenceInspection(candidate.id, candidate.currentRevision, evidence.id);
      markEvidenceInspected(evidence.id);
    } catch (reason) {
      notify.error(aiAssistanceErrorMessage(reason));
    }
  }

  function hasOpenedEvidence(ids: readonly string[]): boolean {
    return candidate.evidenceBundle.items.some((evidence) =>
      ids.includes(evidence.citationKey) && openedEvidenceIds.has(evidence.id));
  }

  async function submitDecision(
    source: AiCandidate,
    id: string,
    action: AiReviewAction,
    resolution?: AiSuggestionResolutionDraft,
  ): Promise<AiCandidate> {
    const identity = `${source.currentRevision}:${id}:${action}:${JSON.stringify(resolution)}`;
    let key = keys.current.get(identity);
    if (key === undefined) {
      key = `${source.id}:${identity}:${globalThis.crypto.randomUUID()}`;
      keys.current.set(identity, key);
    }
    return gateway.decidePaperConceptItem({
      candidateId: source.id,
      candidateRevision: source.currentRevision,
      suggestionId: id,
      action,
      rationale: rationales[id] ?? "",
      idempotencyKey: key,
      ...(resolution === undefined ? {} : { resolution }),
    });
  }

  async function decide(
    id: string,
    action: AiReviewAction,
    resolution?: AiSuggestionResolutionDraft,
  ): Promise<void> {
    setBusyItemId(id);
    try {
      replace(await submitDecision(candidate, id, action, resolution));
    } catch (reason) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setBusyItemId(undefined);
    }
  }

  function citationReview(
    statementId: string,
    evidenceItemIds: readonly string[],
  ): React.JSX.Element {
    return <AiCitationReviewList candidate={candidate} evidenceItemIds={evidenceItemIds}
      gateway={gateway} onChanged={(updated) => replace(updated)} onOpenEvidence={openEvidence}
      statementId={statementId} />;
  }

  function reviewActions(
    id: string,
    evidenceIds: readonly string[],
    resolution: AiSuggestionResolutionDraft | undefined,
    citationsReady: boolean,
  ): React.JSX.Element {
    const last = decisionFor(id);
    const itemTerminal = isItemTerminal(id);
    const busy = busyItemId !== undefined;
    const acceptDisabled = busy || itemTerminal || resolution === undefined || !citationsReady
      || !hasOpenedEvidence(evidenceIds);
    return <div className="paper-concept-review-actions">
      {last !== undefined && <small>最近处理：{last.action}{last.targetObjectId === undefined
        ? "" : ` · ${last.targetObjectKind} ${last.targetObjectId.slice(0, 8)}…`}</small>}
      {!itemTerminal && <>
        <label>审查说明（可选）<input disabled={busy} onChange={(event) =>
          setRationales((current) => ({ ...current, [id]: event.target.value }))}
          value={rationales[id] ?? ""} /></label>
        <div>
          <button disabled={acceptDisabled} onClick={() =>
            void decide(id, "accept_unchanged", resolution)} type="button">接受为工作知识</button>
          <button disabled={busy} onClick={() => void decide(id, "request_more_evidence")}
            type="button">需要更多证据</button>
          <button disabled={busy} onClick={() => void decide(id, "defer")} type="button">暂缓</button>
          <button disabled={busy} onClick={() => void decide(id, "reject")} type="button">拒绝</button>
        </div>
        {resolution !== undefined && !hasOpenedEvidence(evidenceIds)
          && <small>原样接受前，请先打开至少一条实际来源。</small>}
        {!citationsReady && <small>接受前需逐条确认引用支持关系。</small>}
      </>}
    </div>;
  }

  function conceptResolution(concept: PaperConceptCandidateV1): AiSuggestionResolutionDraft | undefined {
    const target = targets[concept.id];
    if (target?.mode === "create_concept") return { kind: "create_concept" };
    if (target?.mode === "unresolved") return { kind: "unresolved" };
    if (target?.mode === "link_concept" && target.conceptId !== undefined) {
      return { kind: "link_concept", conceptId: target.conceptId };
    }
    return undefined;
  }

  function conceptReady(concept: PaperConceptCandidateV1): boolean {
    return isCitationTargetReady(candidate, concept.id, concept.evidenceItemIds)
      && concept.definitionClaims.every((definition) =>
        isCitationTargetReady(candidate, definition.id, definition.evidenceItemIds));
  }

  function acceptedConceptIdFrom(source: AiCandidate, localId: string): string | undefined {
    const decision = decisionForCandidate(source, localId);
    return decision?.targetObjectKind === "concept" ? decision.targetObjectId : undefined;
  }

  function relationResolution(
    relation: PaperConceptRelationCandidateV1,
    source: AiCandidate = candidate,
  ): AiSuggestionResolutionDraft | undefined {
    const subjectConceptId = acceptedConceptIdFrom(source, relation.subjectConceptCandidateId);
    const objectConceptId = acceptedConceptIdFrom(source, relation.objectConceptCandidateId);
    return subjectConceptId === undefined || objectConceptId === undefined ? undefined : {
      kind: "assert_relation", subjectConceptId, objectConceptId,
    };
  }

  function conceptEvidenceIds(concept: PaperConceptCandidateV1): readonly string[] {
    return [...new Set([
      ...concept.evidenceItemIds,
      ...concept.surfaceForms.flatMap((form) => form.evidenceItemIds),
      ...concept.definitionClaims.flatMap((definition) => definition.evidenceItemIds),
    ])];
  }

  const readyConcepts = conceptMap.concepts.filter((concept) => !isItemTerminal(concept.id)
    && conceptResolution(concept) !== undefined && conceptReady(concept)
    && hasOpenedEvidence(conceptEvidenceIds(concept)));
  const readyRelations = conceptMap.relations.filter((relation) => !isItemTerminal(relation.id)
    && relationResolution(relation) !== undefined
    && isCitationTargetReady(candidate, relation.id, relation.evidenceItemIds)
    && hasOpenedEvidence(relation.evidenceItemIds));
  const pendingConceptCount = conceptMap.concepts.filter((concept) =>
    !isItemTerminal(concept.id)).length;
  const pendingRelationCount = conceptMap.relations.filter((relation) =>
    !isItemTerminal(relation.id)).length;

  async function performBatch(): Promise<void> {
    if (batchAction === undefined) return;
    setBatchBusy(true);
    let working = candidate;
    let succeeded = 0;
    let failed = 0;
    const failureMessages: string[] = [];
    try {
      if (batchAction === "reject_all") {
        const ids = [...stableConceptMap.concepts, ...stableConceptMap.relations]
          .map((value) => value.id)
          .filter((id) => !isItemTerminalFor(working, id));
        for (const id of ids) {
          try {
            working = await submitDecision(working, id, "reject");
            succeeded += 1;
          } catch (reason) {
            failed += 1;
            failureMessages.push(aiAssistanceErrorMessage(reason));
          }
        }
      } else {
        for (const concept of readyConcepts) {
          const resolution = conceptResolution(concept);
          if (resolution === undefined || isItemTerminalFor(working, concept.id)) continue;
          try {
            working = await submitDecision(working, concept.id, "accept_unchanged", resolution);
            succeeded += 1;
          } catch (reason) {
            failed += 1;
            failureMessages.push(aiAssistanceErrorMessage(reason));
          }
        }
        for (const relation of stableConceptMap.relations) {
          const resolution = relationResolution(relation, working);
          const ready = resolution !== undefined
            && isCitationTargetReady(working, relation.id, relation.evidenceItemIds)
            && hasOpenedEvidence(relation.evidenceItemIds);
          if (!ready || isItemTerminalFor(working, relation.id)) continue;
          try {
            working = await submitDecision(working, relation.id, "accept_unchanged", resolution);
            succeeded += 1;
          } catch (reason) {
            failed += 1;
            failureMessages.push(aiAssistanceErrorMessage(reason));
          }
        }
      }
      replace(working);
      if (failed === 0) notify.success(`已处理 ${succeeded} 条概念候选。`);
      else notify.warning(`已处理 ${succeeded} 条，${failed} 条失败；${failureMessages[0] ?? "失败项保持原状态"}`);
    } finally {
      setBatchBusy(false);
      setBatchAction(undefined);
    }
  }

  return <section className="paper-concept-candidates">
    <header>
      <div><b>关键概念候选</b><small>概念身份与词形分开；同名和向量相近都不会自动合并</small></div>
      <span>{candidate.reviewState} · {candidate.providerId} / {candidate.modelId}</span>
    </header>
    {!terminal && <div className="paper-concept-batch-actions">
      <div>
        <button disabled={batchBusy || readyConcepts.length + readyRelations.length === 0}
          onClick={() => setBatchAction("accept_ready")} type="button">接受全部已就绪</button>
        <button disabled={batchBusy || pendingConceptCount + pendingRelationCount === 0}
          onClick={() => setBatchAction("reject_all")} type="button">全部舍弃</button>
      </div>
      <small>{readyConcepts.length + readyRelations.length} 条已就绪 · {pendingConceptCount + pendingRelationCount} 条待处理</small>
    </div>}
    {groupOrder.map((group) => {
      const concepts = conceptMap.concepts.filter((concept) => concept.paperFunction === group);
      if (concepts.length === 0) return null;
      return <section className="paper-concept-group" key={group}>
        <header><h4>{groupLabels[group]}</h4><small>{concepts.length} 个候选</small></header>
        {concepts.map((concept) => {
          const target = targets[concept.id];
          const disabled = terminal || busyItemId !== undefined || isItemTerminal(concept.id);
          const allEvidenceIds = conceptEvidenceIds(concept);
          const last = decisionFor(concept.id);
          return <details className="paper-concept-card" key={concept.id}>
            <summary><div><h5>{concept.preferredTerm}</h5><small>{concept.surfaceForms
              .filter((form) => form.relationKind !== "preferred").map((form) => form.text)
              .join(" · ") || "无其他词形"}</small></div>
              <div><span>{noveltyLabels[concept.noveltyStatus]}</span>
                <span>{explanationLabels[concept.explanationStatus]}</span>
                {last !== undefined && <span>{last.action}</span>}</div></summary>
            <div className="paper-concept-card-body">
            <p>{concept.explanation || "本文未提供足够解释；保留引用线索供后续处理。"}</p>
            <details><summary>角色、新颖性与消歧依据</summary>
              <p>{concept.roleRationale}</p><p>{concept.noveltyRationale || "未判断"}</p>
              {concept.disambiguation !== "" && <p>{concept.disambiguation}</p>}</details>
            {citationReview(concept.id, concept.evidenceItemIds)}
            {concept.definitionClaims.map((definition) => <details key={definition.id}>
              <summary>定义候选：{definition.statement}</summary>
              {definition.scopeNote !== "" && <p>{definition.scopeNote}</p>}
              {citationReview(definition.id, definition.evidenceItemIds)}</details>)}
            {concept.referenceMentions.length > 0 && <details><summary>引用文献线索</summary>
              {concept.referenceMentions.map((reference, index) =>
                <p key={`${reference.citationText}:${index}`}>{reference.citationText}</p>)}</details>}
            {!isItemTerminal(concept.id) && <label>概念身份处理<select disabled={disabled}
              onChange={(event) => {
                const mode = event.target.value;
                setTargets((current) => ({ ...current, [concept.id]: mode === "" ? {} : {
                  mode: mode as NonNullable<ConceptTarget["mode"]>,
                } }));
              }} value={target?.mode ?? ""}>
              <option value="">请选择，不自动判断</option><option value="create_concept">创建新 Concept</option>
              <option value="link_concept">关联已有 Concept</option><option value="unresolved">保持未消歧</option>
            </select></label>}
            {target?.mode === "link_concept" && <ConceptTargetPicker disabled={disabled}
              label="选择已有 Concept" onSelect={(conceptId) => setTargets((current) => ({
                ...current, [concept.id]: { ...current[concept.id], conceptId },
              }))} selectedId={target.conceptId} suggestedTerm={concept.preferredTerm} />}
            {reviewActions(concept.id, allEvidenceIds, conceptResolution(concept), conceptReady(concept))}
            </div>
          </details>;
        })}
      </section>;
    })}
    {conceptMap.relations.length > 0 && <section className="paper-concept-relations">
      <header><h4>概念关系</h4><small>两端概念完成消歧后才可接受</small></header>
      {conceptMap.relations.map((relation) => {
        const subject = conceptMap.concepts.find((concept) => concept.id === relation.subjectConceptCandidateId);
        const object = conceptMap.concepts.find((concept) => concept.id === relation.objectConceptCandidateId);
        return <details className="paper-concept-relation-card" key={relation.id}>
          <summary><b>{subject?.preferredTerm} → {relationLabels[relation.predicateKind]} → {object?.preferredTerm}</b></summary>
          <div className="paper-concept-card-body">
          {relation.scopeNote !== "" && <p>{relation.scopeNote}</p>}
          {citationReview(relation.id, relation.evidenceItemIds)}
          {reviewActions(relation.id, relation.evidenceItemIds, relationResolution(relation),
            isCitationTargetReady(candidate, relation.id, relation.evidenceItemIds))}
          </div>
        </details>;
      })}
    </section>}
    {batchAction !== undefined && <ReviewConfirmationDialog
      busy={batchBusy}
      confirmLabel={batchAction === "accept_ready" ? "接受已就绪项" : "舍弃全部待处理项"}
      confirmTone={batchAction === "reject_all" ? "danger" : "primary"}
      description={batchAction === "accept_ready"
        ? "只处理已选择概念身份、已确认引用支持且打开过来源的条目；其余候选保持不变。"
        : "拒绝当前版本中全部尚未终态的概念和关系候选；已接受的工作知识不会被删除。"}
      groups={[
        { label: "概念", untrustedCount: batchAction === "reject_all" ? pendingConceptCount : 0,
          reviewableCount: batchAction === "accept_ready" ? readyConcepts.length : 0 },
        { label: "关系", untrustedCount: batchAction === "reject_all" ? pendingRelationCount : 0,
          reviewableCount: batchAction === "accept_ready" ? readyRelations.length : 0,
          ...(batchAction === "accept_ready"
            ? { note: "本批接受概念后，新满足两端身份条件的关系也会一并处理。" }
            : {}) },
      ]}
      onCancel={() => setBatchAction(undefined)}
      onConfirm={() => void performBatch()}
      title={batchAction === "accept_ready" ? "接受全部已就绪候选？" : "舍弃全部待处理候选？"}
      warning={batchAction === "accept_ready"
        ? "接受表示纳入当前工作认识，不表示概念或关系已成为客观事实。"
        : "舍弃会保留不可变审查记录，但这些候选不再进入当前工作知识。"}
    />}
  </section>;
}
