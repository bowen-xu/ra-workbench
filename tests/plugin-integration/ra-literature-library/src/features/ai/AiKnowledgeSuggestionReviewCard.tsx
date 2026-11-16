import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { useEffect, useRef, useState } from "react";
import {
  type AiCandidate,
  type AiEvidenceItem,
  type AiKnowledgeCandidatePayloadV1,
  type AiReviewAction,
  type AiSuggestionResolutionDraft,
} from "@research-assistant/core/ai";
import { RELATION_PREDICATE_KINDS } from "@research-assistant/core/knowledge";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { aiAssistanceErrorMessage } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { useEvidenceInspectionState } from "../../../../../../../plugins/ra-literature-library/src/features/ai/useEvidenceInspectionState";
import { AiCitationReviewList, isCitationTargetReady } from "./AiCitationReviewList";
import { AiAttentionFeedbackControls } from "./AiAttentionFeedbackControls";
import { ConceptTargetPicker } from "../../../../../../../plugins/ra-literature-library/src/features/ai/ConceptTargetPicker";

interface AiKnowledgeSuggestionReviewCardProps {
  readonly candidate: AiCandidate;
  readonly gateway: AiAssistanceGateway;
  readonly projectId?: string;
  readonly onChanged: (candidate: AiCandidate, didCreateKnowledge: boolean) => void;
  readonly onOpenEvidence: (evidence: AiEvidenceItem) => Promise<void>;
}

interface TargetDraft {
  readonly mode?: "create_concept" | "link_concept" | "unresolved";
  readonly conceptId?: string;
  readonly subjectConceptId?: string;
  readonly objectConceptId?: string;
}

const roleLabels = {
  source_paraphrase: "来源转述",
  synthesis: "综合",
  inference: "推论",
  uncertain: "不确定",
} as const;

const kindLabels = {
  concept_suggestion_v1: "AI 概念候选",
  definition_suggestion_v1: "AI 定义候选",
  relation_suggestion_v1: "AI 关系候选",
  attention_suggestion_v1: "AI 阅读注意力建议",
} as const;

function currentPayload(candidate: AiCandidate): AiKnowledgeCandidatePayloadV1 | undefined {
  const payload = candidate.revisions.find((item) =>
    item.revision === candidate.currentRevision)?.payload;
  return payload?.kind === "concept_suggestion_v1"
    || payload?.kind === "definition_suggestion_v1"
    || payload?.kind === "relation_suggestion_v1"
    || payload?.kind === "attention_suggestion_v1" ? payload : undefined;
}

function optionalText(value: string | undefined): string | undefined {
  return value?.trim() === "" || value === undefined ? undefined : value.trim();
}

function cleanPayload(payload: AiKnowledgeCandidatePayloadV1): AiKnowledgeCandidatePayloadV1 {
  if (payload.kind === "concept_suggestion_v1") {
    return { ...payload, suggestions: payload.suggestions.map((suggestion) => {
      const { language, disambiguation, domain, tradition, period, ...required } = suggestion;
      const nextLanguage = optionalText(language); const nextDisambiguation = optionalText(disambiguation);
      const nextDomain = optionalText(domain); const nextTradition = optionalText(tradition);
      const nextPeriod = optionalText(period);
      return { ...required, ...(nextLanguage === undefined ? {} : { language: nextLanguage }),
        ...(nextDisambiguation === undefined ? {} : { disambiguation: nextDisambiguation }),
        ...(nextDomain === undefined ? {} : { domain: nextDomain }),
        ...(nextTradition === undefined ? {} : { tradition: nextTradition }),
        ...(nextPeriod === undefined ? {} : { period: nextPeriod }) };
    }) };
  }
  if (payload.kind === "definition_suggestion_v1") {
    return { ...payload, suggestions: payload.suggestions.map(({ scopeNote, ...suggestion }) => ({
      ...suggestion,
      ...(scopeNote?.trim() === "" || scopeNote === undefined ? {} : { scopeNote: scopeNote.trim() }),
    })) };
  }
  if (payload.kind === "relation_suggestion_v1") {
    return { ...payload, suggestions: payload.suggestions.map(({ scopeNote, ...suggestion }) => ({
      ...suggestion,
      ...(scopeNote?.trim() === "" || scopeNote === undefined ? {} : { scopeNote: scopeNote.trim() }),
    })) };
  }
  return payload;
}

export function AiKnowledgeSuggestionReviewCard({
  candidate,
  gateway,
  projectId,
  onChanged,
  onOpenEvidence,
}: AiKnowledgeSuggestionReviewCardProps): React.JSX.Element {
  const payload = currentPayload(candidate);
  const [draft, setDraft] = useState<AiKnowledgeCandidatePayloadV1 | undefined>(payload);
  const [targets, setTargets] = useState<Readonly<Record<string, TargetDraft>>>({});
  const [rationales, setRationales] = useState<Readonly<Record<string, string>>>({});
  const [openedEvidenceIds, markEvidenceInspected] = useEvidenceInspectionState(candidate);
  const [busyItemId, setBusyItemId] = useState<string>();
  const idempotencyKeys = useRef(new Map<string, string>());

  useEffect(() => {
    setDraft(currentPayload(candidate));
    setTargets({}); setRationales({});
  }, [candidate.id, candidate.currentRevision]);

  if (payload === undefined || draft === undefined || payload.kind !== draft.kind) {
    return <article className="ai-review-card"><p>知识建议结构不兼容，未提供审查操作。</p></article>;
  }
  const currentRevision = candidate.revisions.find((item) =>
    item.revision === candidate.currentRevision);
  const acceptsAfterEdit = currentRevision?.editorKind === "human";
  const isTerminal = ["accepted_unchanged", "accepted_after_edit", "rejected"]
    .includes(candidate.reviewState);
  const decisions = candidate.suggestionItemDecisions.filter((decision) =>
    decision.candidateRevision === candidate.currentRevision);
  const hasTerminalItemDecision = decisions.some((decision) =>
    ["accept_unchanged", "accept_after_edit", "reject"].includes(decision.action));
  const hasChanges = JSON.stringify(draft) !== JSON.stringify(payload);

  function itemDecision(suggestionId: string) {
    return [...decisions].reverse().find((decision) => decision.suggestionId === suggestionId);
  }

  function isItemTerminal(suggestionId: string): boolean {
    const action = itemDecision(suggestionId)?.action;
    return action === "accept_unchanged" || action === "accept_after_edit" || action === "reject";
  }

  function updateTarget(suggestionId: string, patch: Partial<TargetDraft>): void {
    setTargets((current) => ({
      ...current,
      [suggestionId]: { ...current[suggestionId], ...patch },
    }));
  }

  async function saveRevision(): Promise<void> {
    if (!hasChanges || hasTerminalItemDecision || draft === undefined) return;
    const reviewedDraft = cleanPayload(draft);
    setBusyItemId("save");
    try {
      const revised = await gateway.reviseCandidate({
        candidateId: candidate.id,
        expectedRevision: candidate.currentRevision,
        payload: reviewedDraft,
        changeReason: "human_knowledge_suggestion_edit",
      });
      onChanged(revised, false);
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
    finally { setBusyItemId(undefined); }
  }

  async function openEvidence(evidence: AiEvidenceItem): Promise<void> {

    try {
      await onOpenEvidence(evidence);
      await gateway.recordEvidenceInspection(candidate.id, candidate.currentRevision, evidence.id);
      markEvidenceInspected(evidence.id);
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
  }

  function hasOpenedEvidence(evidenceItemIds: readonly string[]): boolean {
    return candidate.evidenceBundle.items.some((evidence) =>
      evidenceItemIds.includes(evidence.citationKey) && openedEvidenceIds.has(evidence.id));
  }

  async function decideItem(
    suggestionId: string,
    action: AiReviewAction,
    resolution?: AiSuggestionResolutionDraft,
  ): Promise<void> {
    setBusyItemId(suggestionId);
    try {
      const keyId = `${candidate.currentRevision}:${suggestionId}:${action}:${JSON.stringify(resolution)}`;
      let idempotencyKey = idempotencyKeys.current.get(keyId);
      if (idempotencyKey === undefined) {
        idempotencyKey = `${candidate.id}:${keyId}:${globalThis.crypto.randomUUID()}`;
        idempotencyKeys.current.set(keyId, idempotencyKey);
      }
      const updated = await gateway.decideSuggestionItem({
        candidateId: candidate.id,
        candidateRevision: candidate.currentRevision,
        suggestionId,
        action,
        rationale: rationales[suggestionId] ?? "",
        idempotencyKey,
        ...(resolution === undefined ? {} : { resolution }),
      });
      onChanged(updated, action === "accept_unchanged" || action === "accept_after_edit");
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
    finally { setBusyItemId(undefined); }
  }

  function evidenceButtons(
    statementIdOrEvidenceItemIds: string | readonly string[],
    explicitEvidenceItemIds?: readonly string[],
  ): React.JSX.Element {
    const evidenceItemIds = typeof statementIdOrEvidenceItemIds === "string"
      ? explicitEvidenceItemIds ?? [] : statementIdOrEvidenceItemIds;
    const statementId = typeof statementIdOrEvidenceItemIds === "string"
      ? statementIdOrEvidenceItemIds
      : draft?.suggestions.find((item) => item.evidenceItemIds === evidenceItemIds)?.id ?? "missing";
    return <AiCitationReviewList candidate={candidate} evidenceItemIds={evidenceItemIds}
      gateway={gateway} onChanged={onChanged} onOpenEvidence={openEvidence}
      statementId={statementId} />;
  }

  function commonActions(
    suggestionId: string,
    evidenceItemIds: readonly string[],
    resolution?: AiSuggestionResolutionDraft,
  ): React.JSX.Element {
    const terminal = isItemTerminal(suggestionId);
    const lastDecision = itemDecision(suggestionId);
    const busy = busyItemId !== undefined;
    const acceptDisabled = resolution === undefined || busy || terminal || hasChanges
      || !isCitationTargetReady(candidate, suggestionId, evidenceItemIds)
      || !acceptsAfterEdit && !hasOpenedEvidence(evidenceItemIds);
    return <div className="ai-suggestion-item-review">
      {lastDecision !== undefined && <p>本版本最近处理：{lastDecision.action}{lastDecision.targetObjectKind === undefined ? "" : ` · ${lastDecision.targetObjectKind} ${lastDecision.targetObjectId?.slice(0, 8)}…`}</p>}
      {!terminal && <><label>逐条审查说明（可选）<input disabled={busy} onChange={(event) => setRationales((current) => ({ ...current, [suggestionId]: event.target.value }))} value={rationales[suggestionId] ?? ""} /></label>
        <div className="ai-review-actions">
          {resolution !== undefined && <button disabled={acceptDisabled} onClick={() => void decideItem(suggestionId, acceptsAfterEdit ? "accept_after_edit" : "accept_unchanged", resolution)} type="button">{acceptsAfterEdit ? "接受人工修订" : "接受为工作知识"}</button>}
          <button disabled={busy || hasChanges} onClick={() => void decideItem(suggestionId, "request_more_evidence")} type="button">需要更多证据</button>
          <button disabled={busy || hasChanges} onClick={() => void decideItem(suggestionId, "defer")} type="button">暂缓</button>
          <button disabled={busy || hasChanges} onClick={() => void decideItem(suggestionId, "reject")} type="button">拒绝此条</button>
        </div>
        {!hasChanges && !acceptsAfterEdit && resolution !== undefined && !hasOpenedEvidence(evidenceItemIds) && <small>原样接受此条前，请先打开它实际引用的至少一条来源。</small>}
        {!hasChanges && resolution !== undefined && !isCitationTargetReady(candidate, suggestionId, evidenceItemIds) && <small>接受前至少确认一条支持引用；失效、反对或不支持会阻止接受。</small>}
      </>}
    </div>;
  }

  function conceptResolution(suggestionId: string): AiSuggestionResolutionDraft | undefined {
    const target = targets[suggestionId];
    if (target?.mode === "create_concept") return { kind: "create_concept" };
    if (target?.mode === "unresolved") return { kind: "unresolved" };
    if (target?.mode === "link_concept" && target.conceptId !== undefined) {
      return { kind: "link_concept", conceptId: target.conceptId };
    }
    return undefined;
  }

  function setConceptMode(suggestionId: string, value: string): void {
    setTargets((current) => ({
      ...current,
      [suggestionId]: value === "" ? {} : { mode: value as NonNullable<TargetDraft["mode"]> },
    }));
  }

  return <article className={`ai-review-card ai-knowledge-review ${isTerminal ? "terminal" : ""}`}>
    <header><div><span className="eyebrow">{kindLabels[draft.kind]}</span><strong>{candidate.providerId} · {candidate.modelId}</strong></div><small>{candidate.reviewState} · 候选 v{candidate.currentRevision} · 逐条审查</small></header>
    <p>AI 只提供带来源的启发式候选。编辑整批建议必须发生在第一条终态决定之前；接受后创建的对象仍是可修订的工作知识，不表示客观证实。</p>
    <div className="ai-knowledge-suggestions">
      {draft.kind === "concept_suggestion_v1" && draft.suggestions.map((suggestion, index) => {
        const target = targets[suggestion.id]; const disabled = busyItemId !== undefined || isTerminal || isItemTerminal(suggestion.id);
        return <section key={suggestion.id}><header><b>概念 {index + 1}</b><span>{roleLabels[suggestion.epistemicRole]}</span></header>
          <label>语词<input disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "concept_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, term: event.target.value } : item) })} value={suggestion.term} /></label>
          <label>详细解释<textarea disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "concept_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, explanation: event.target.value } : item) })} rows={3} value={suggestion.explanation} /></label>
          <div className="ai-suggestion-qualifiers"><label>消歧说明<input disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "concept_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, disambiguation: event.target.value } : item) })} value={suggestion.disambiguation ?? ""} /></label><label>领域<input disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "concept_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, domain: event.target.value } : item) })} value={suggestion.domain ?? ""} /></label><label>传统／流派<input disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "concept_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, tradition: event.target.value } : item) })} value={suggestion.tradition ?? ""} /></label><label>时期<input disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "concept_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, period: event.target.value } : item) })} value={suggestion.period ?? ""} /></label></div>
          {evidenceButtons(suggestion.id, suggestion.evidenceItemIds)}
          <label>概念身份处理<select disabled={disabled} onChange={(event) => setConceptMode(suggestion.id, event.target.value)} value={target?.mode ?? ""}><option value="">请选择，不自动判断</option><option value="create_concept">创建新的 Concept</option><option value="link_concept">关联已有 Concept</option><option value="unresolved">保留为未消歧 Term Usage</option></select></label>
          {target?.mode === "link_concept" && <ConceptTargetPicker disabled={disabled} label="选择已有 Concept" onSelect={(conceptId) => updateTarget(suggestion.id, { conceptId })} selectedId={target.conceptId} suggestedTerm={suggestion.term} />}
          <AiAttentionFeedbackControls candidate={candidate} gateway={gateway} suggestionId={suggestion.id} />
          {commonActions(suggestion.id, suggestion.evidenceItemIds, conceptResolution(suggestion.id))}
        </section>;
      })}
      {draft.kind === "definition_suggestion_v1" && draft.suggestions.map((suggestion, index) => {
        const target = targets[suggestion.id]; const disabled = busyItemId !== undefined || isTerminal || isItemTerminal(suggestion.id);
        const resolution = target?.conceptId === undefined ? undefined : { kind: "attach_definition", conceptId: target.conceptId } as const;
        return <section key={suggestion.id}><header><b>定义 {index + 1}</b><span>{roleLabels[suggestion.epistemicRole]}</span></header><label>语词<input disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "definition_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, term: event.target.value } : item) })} value={suggestion.term} /></label><label>定义陈述<textarea disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "definition_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, statement: event.target.value } : item) })} rows={3} value={suggestion.statement} /></label><label>适用范围<textarea disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "definition_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, scopeNote: event.target.value } : item) })} rows={2} value={suggestion.scopeNote ?? ""} /></label>{evidenceButtons(suggestion.evidenceItemIds)}<ConceptTargetPicker disabled={disabled} label="选择定义所属 Concept" onSelect={(conceptId) => updateTarget(suggestion.id, { conceptId })} selectedId={target?.conceptId} suggestedTerm={suggestion.term} /><AiAttentionFeedbackControls candidate={candidate} gateway={gateway} suggestionId={suggestion.id} />{commonActions(suggestion.id, suggestion.evidenceItemIds, resolution)}</section>;
      })}
      {draft.kind === "relation_suggestion_v1" && draft.suggestions.map((suggestion, index) => {
        const target = targets[suggestion.id]; const disabled = busyItemId !== undefined || isTerminal || isItemTerminal(suggestion.id);
        const resolution = target?.subjectConceptId === undefined || target.objectConceptId === undefined ? undefined : { kind: "assert_relation", subjectConceptId: target.subjectConceptId, objectConceptId: target.objectConceptId, ...(projectId === undefined ? {} : { projectId }) } as const;
        return <section key={suggestion.id}><header><b>关系 {index + 1}</b><span>{roleLabels[suggestion.epistemicRole]}</span></header><div className="ai-suggestion-relation"><input aria-label="建议关系起点语词" disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "relation_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, subjectTerm: event.target.value } : item) })} value={suggestion.subjectTerm} /><select aria-label="建议关系类型" disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "relation_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, predicateKind: event.target.value as typeof item.predicateKind } : item) })} value={suggestion.predicateKind}>{RELATION_PREDICATE_KINDS.map((kind) => <option key={kind} value={kind}>{kind}</option>)}</select><input aria-label="建议关系终点语词" disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "relation_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, objectTerm: event.target.value } : item) })} value={suggestion.objectTerm} /></div><label>适用范围<textarea disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "relation_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, scopeNote: event.target.value } : item) })} rows={2} value={suggestion.scopeNote ?? ""} /></label>{evidenceButtons(suggestion.evidenceItemIds)}<div className="ai-relation-targets"><ConceptTargetPicker disabled={disabled} label="选择起点 Concept" onSelect={(conceptId) => updateTarget(suggestion.id, { subjectConceptId: conceptId })} selectedId={target?.subjectConceptId} suggestedTerm={suggestion.subjectTerm} /><ConceptTargetPicker disabled={disabled} label="选择终点 Concept" onSelect={(conceptId) => updateTarget(suggestion.id, { objectConceptId: conceptId })} selectedId={target?.objectConceptId} suggestedTerm={suggestion.objectTerm} /></div><AiAttentionFeedbackControls candidate={candidate} gateway={gateway} suggestionId={suggestion.id} />{commonActions(suggestion.id, suggestion.evidenceItemIds, resolution)}</section>;
      })}
      {draft.kind === "attention_suggestion_v1" && draft.suggestions.map((suggestion, index) => {
        const disabled = busyItemId !== undefined || isTerminal || isItemTerminal(suggestion.id);
        return <section key={suggestion.id}><header><b>注意点 {index + 1}</b><span>优先级仅表示阅读顺序</span></header><label>标题<input disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "attention_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, title: event.target.value } : item) })} value={suggestion.title} /></label><label>为什么值得注意<textarea disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "attention_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, reason: event.target.value } : item) })} rows={3} value={suggestion.reason} /></label><label>阅读优先级<select disabled={disabled} onChange={(event) => setDraft((current) => current?.kind !== "attention_suggestion_v1" ? current : { ...current, suggestions: current.suggestions.map((item) => item.id === suggestion.id ? { ...item, priority: event.target.value as typeof item.priority } : item) })} value={suggestion.priority}><option value="high">高</option><option value="medium">中</option><option value="low">低</option></select></label>{evidenceButtons(suggestion.evidenceItemIds)}<AiAttentionFeedbackControls candidate={candidate} gateway={gateway} suggestionId={suggestion.id} />{commonActions(suggestion.id, suggestion.evidenceItemIds)}</section>;
      })}
    </div>
    {!isTerminal && <div className="ai-review-actions"><button disabled={busyItemId !== undefined || !hasChanges || hasTerminalItemDecision} onClick={() => void saveRevision()} type="button">保存整批人工修订</button></div>}
    {hasChanges && <small>先保存人工 revision，再逐条处理；未保存内容不会覆盖 AI 原稿。</small>}
    {hasTerminalItemDecision && <small>本 revision 已有条目完成审查，为避免已晋升知识与候选错位，整批编辑已锁定。</small>}
  </article>;
}
