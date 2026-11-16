import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { useEffect, useRef, useState } from "react";
import type {
  AiCandidate,
  AiDerivedResearchViewCandidateV1,
  AiDerivedResearchViewItemV1,
  AiEvidenceItem,
  AiReviewAction,
} from "@research-assistant/core/ai";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { aiAssistanceErrorMessage } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { AiCitationReviewList, isCandidateCitationReady } from "./AiCitationReviewList";
import { useEvidenceInspectionState } from "../../../../../../../plugins/ra-literature-library/src/features/ai/useEvidenceInspectionState";

interface AiDerivedResearchViewReviewCardProps {
  readonly candidate: AiCandidate;
  readonly gateway: AiAssistanceGateway;
  readonly projectId?: string;
  readonly onChanged: (candidate: AiCandidate, didCreateKnowledge: boolean) => void;
  readonly onOpenEvidence: (evidence: AiEvidenceItem) => Promise<void>;
}

const viewLabels = {
  taxonomy: "Taxonomy 候选",
  historical_context: "历史脉络",
  gap_hypothesis: "领域空白假设",
  innovation_hypothesis: "创新建议假设",
} as const;

function currentPayload(candidate: AiCandidate): AiDerivedResearchViewCandidateV1 | undefined {
  const revision = candidate.revisions.find((item) => item.revision === candidate.currentRevision);
  return revision?.payload.kind === "derived_research_view_v1" ? revision.payload : undefined;
}

function copyItems(items: readonly AiDerivedResearchViewItemV1[]) {
  return items.map((item) => ({ ...item, evidenceItemIds: [...item.evidenceItemIds] }));
}

function revisionSnapshot(payload: AiDerivedResearchViewCandidateV1): React.JSX.Element {
  return <section>
    <b>{payload.title}</b>
    <p>{payload.scopeNote}</p>
    <ol>{payload.items.map((item) => <li key={item.id}>{item.text}<small>{item.epistemicRole} · {item.evidenceItemIds.join("、")}{item.hasConflict ? " · 含未解决冲突" : ""}</small></li>)}</ol>
  </section>;
}

export function AiDerivedResearchViewReviewCard({
  candidate,
  gateway,
  projectId,
  onChanged,
  onOpenEvidence,
}: AiDerivedResearchViewReviewCardProps): React.JSX.Element {
  const payload = currentPayload(candidate);
  const [title, setTitle] = useState(payload?.title ?? "");
  const [scopeNote, setScopeNote] = useState(payload?.scopeNote ?? "");
  const [items, setItems] = useState<readonly AiDerivedResearchViewItemV1[]>(() =>
    payload === undefined ? [] : copyItems(payload.items));
  const [rationale, setRationale] = useState("");
  const [busy, setBusy] = useState(false);
  const [openedEvidenceIds, markEvidenceInspected] = useEvidenceInspectionState(candidate);
  const idempotencyKeys = useRef(new Map<string, string>());

  useEffect(() => {
    const next = currentPayload(candidate);
    setTitle(next?.title ?? "");
    setScopeNote(next?.scopeNote ?? "");
    setItems(next === undefined ? [] : copyItems(next.items));
  }, [candidate.id, candidate.currentRevision]);

  if (payload === undefined) {
    return <article className="ai-review-card"><p>派生视图候选结构不兼容，未提供审查操作。</p></article>;
  }
  const reviewedPayload = payload;
  const originalPayload = candidate.revisions[0]?.payload.kind === "derived_research_view_v1"
    ? candidate.revisions[0].payload
    : undefined;
  const currentRevision = candidate.revisions.find((item) =>
    item.revision === candidate.currentRevision);
  const acceptsAsHumanEdit = currentRevision?.editorKind === "human";
  const isTerminal = ["accepted_unchanged", "accepted_after_edit", "rejected"]
    .includes(candidate.reviewState);
  const isHypothesis = reviewedPayload.viewKind === "gap_hypothesis"
    || reviewedPayload.viewKind === "innovation_hypothesis";
  const hasChanges = title !== reviewedPayload.title || scopeNote !== reviewedPayload.scopeNote
    || JSON.stringify(items) !== JSON.stringify(reviewedPayload.items);
  const itemsValid = items.length > 0 && items.length <= 24
    && items.every((item) => item.text.trim() !== ""
      && item.evidenceItemIds.length >= (item.epistemicRole === "synthesis" ? 2 : 1)
      && item.evidenceItemIds.length <= 16
      && (!isHypothesis || item.epistemicRole === "hypothesis"));

  function updateItem(id: string, patch: Partial<AiDerivedResearchViewItemV1>): void {
    setItems((current) => current.map((item) => item.id === id ? { ...item, ...patch } : item));
  }

  function toggleEvidence(itemId: string, citationKey: string): void {
    const item = items.find((value) => value.id === itemId);
    if (item === undefined) return;
    const selected = item.evidenceItemIds.includes(citationKey);
    if (!selected && item.evidenceItemIds.length >= 16) return;
    updateItem(itemId, {
      evidenceItemIds: selected
        ? item.evidenceItemIds.filter((id) => id !== citationKey)
        : [...item.evidenceItemIds, citationKey],
    });
  }

  function addItem(): void {
    setItems((current) => [...current, {
      id: `item_${globalThis.crypto.randomUUID().replaceAll("-", "")}`,
      text: "",
      epistemicRole: isHypothesis ? "hypothesis" : "synthesis",
      evidenceItemIds: [],
      hasConflict: false,
    }]);
  }

  async function saveRevision(): Promise<void> {
    if (!hasChanges || !itemsValid) return;
    setBusy(true);
    try {
      const nextPayload: AiDerivedResearchViewCandidateV1 = {
        ...reviewedPayload,
        title: title.trim(),
        scopeNote: scopeNote.trim(),
        items: items.map((item) => ({ ...item, text: item.text.trim() })),
      };
      const revised = await gateway.reviseCandidate({
        candidateId: candidate.id,
        expectedRevision: candidate.currentRevision,
        payload: nextPayload,
        changeReason: "human_derived_view_review",
      });
      onChanged(revised, false);
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
    finally { setBusy(false); }
  }

  async function openEvidence(evidence: AiEvidenceItem): Promise<void> {

    try {
      await onOpenEvidence(evidence);
      await gateway.recordEvidenceInspection(candidate.id, candidate.currentRevision, evidence.id);
      markEvidenceInspected(evidence.id);
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
  }

  async function decide(action: AiReviewAction): Promise<void> {
    setBusy(true);
    try {
      const keyId = `${candidate.currentRevision}:${action}`;
      let idempotencyKey = idempotencyKeys.current.get(keyId);
      if (idempotencyKey === undefined) {
        idempotencyKey = `${candidate.id}:${keyId}:${globalThis.crypto.randomUUID()}`;
        idempotencyKeys.current.set(keyId, idempotencyKey);
      }
      const updated = await gateway.decideCandidate({
        candidateId: candidate.id,
        candidateRevision: candidate.currentRevision,
        action,
        rationale,
        idempotencyKey,
        ...(projectId === undefined ? {} : { projectId }),
      });
      onChanged(updated, action === "accept_unchanged" || action === "accept_after_edit");
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
    finally { setBusy(false); }
  }

  const acceptDisabled = busy || hasChanges || projectId === undefined
    || !isCandidateCitationReady(candidate)
    || !acceptsAsHumanEdit && openedEvidenceIds.size === 0;
  return <article className={`ai-review-card ai-derived-review ${isTerminal ? "terminal" : ""}`}>
    <header><div><span className="eyebrow">AI DERIVED VIEW</span><strong>{viewLabels[reviewedPayload.viewKind]} · {candidate.providerId}</strong></div><small>{candidate.reviewState} · 候选 v{candidate.currentRevision}</small></header>
    <p>输入矩阵单元与 revision 已冻结。接受只创建独立派生视图，不会回写矩阵；空白与创新即使接受也仍是工作假设。</p>
    {candidate.currentRevision > 1 && originalPayload !== undefined && <details className="ai-edit-diff">
      <summary>查看 AI 原稿与当前人工 revision 差异</summary>
      <div className="ai-revision-columns">
        <div><strong>初始 AI 原稿</strong>{revisionSnapshot(originalPayload)}</div>
        <div><strong>当前人工 revision</strong>{revisionSnapshot(reviewedPayload)}</div>
      </div>
    </details>}
    <label>标题<input disabled={busy || isTerminal} onChange={(event) => setTitle(event.target.value)} value={title} /></label>
    <label>范围与限制<textarea disabled={busy || isTerminal} onChange={(event) => setScopeNote(event.target.value)} rows={3} value={scopeNote} /></label>
    <div className="ai-derived-items">{items.map((item, index) => <section key={item.id}>
      <header><b>综合条目 {index + 1}</b>{!isTerminal && <button aria-label={`删除综合条目 ${index + 1}`} disabled={busy} onClick={() => setItems((current) => current.filter((value) => value.id !== item.id))} type="button">删除</button>}</header>
      <textarea aria-label={`综合条目 ${index + 1}`} disabled={busy || isTerminal} onChange={(event) => updateItem(item.id, { text: event.target.value })} rows={3} value={item.text} />
      {!isHypothesis && <label>认识角色<select disabled={busy || isTerminal} onChange={(event) => updateItem(item.id, { epistemicRole: event.target.value as AiDerivedResearchViewItemV1["epistemicRole"] })} value={item.epistemicRole}><option value="synthesis">综合（至少两条来源）</option><option value="inference">推论</option><option value="hypothesis">假设</option></select></label>}
      <label><input checked={item.hasConflict} disabled={busy || isTerminal} onChange={(event) => updateItem(item.id, { hasConflict: event.target.checked })} type="checkbox" />包含未解决冲突</label>
      <div className="ai-derived-evidence">{candidate.evidenceBundle.items.map((evidence) => <div key={evidence.id}><label><input checked={item.evidenceItemIds.includes(evidence.citationKey)} disabled={busy || isTerminal} onChange={() => toggleEvidence(item.id, evidence.citationKey)} type="checkbox" /><q>{evidence.quoteExact}</q></label></div>)}</div>
      <AiCitationReviewList candidate={candidate} evidenceItemIds={item.evidenceItemIds}
        gateway={gateway} onChanged={onChanged} onOpenEvidence={openEvidence}
        statementId={item.id} />
    </section>)}</div>
    {!isTerminal && <button disabled={busy || items.length >= 24} onClick={addItem} type="button">添加人工综合条目</button>}
    {!isTerminal && <>
      <label>审查说明（可选）<input disabled={busy} onChange={(event) => setRationale(event.target.value)} value={rationale} /></label>
      <div className="ai-review-actions"><button disabled={busy || !hasChanges || title.trim() === "" || scopeNote.trim() === "" || !itemsValid} onClick={() => void saveRevision()} type="button">保存人工修订</button><button disabled={acceptDisabled} onClick={() => void decide(acceptsAsHumanEdit ? "accept_after_edit" : "accept_unchanged")} type="button">{acceptsAsHumanEdit ? "接受人工修订" : "原样接受为派生视图"}</button><button disabled={busy || hasChanges} onClick={() => void decide("request_more_evidence")} type="button">需要更多证据</button><button disabled={busy || hasChanges} onClick={() => void decide("defer")} type="button">暂缓</button><button disabled={busy || hasChanges} onClick={() => void decide("reject")} type="button">拒绝</button></div>
      {projectId === undefined && <small>请进入该矩阵所属项目后再接受；拒绝、暂缓和继续编辑不受影响。</small>}
      {hasChanges && <small>先保存人工 revision，再作审查决定；引用只能从本候选冻结的证据中选择。</small>}
      {!hasChanges && !acceptsAsHumanEdit && openedEvidenceIds.size === 0 && <small>原样接受前，请至少打开一条实际来源进行核对。</small>}
      {!hasChanges && !isCandidateCitationReady(candidate) && <small>每个综合条目至少需要一条人工确认支持的引用；反对、不支持或失效来源会阻止接受。</small>}
    </>}
    {isTerminal && candidate.decisions.at(-1)?.targetObjectKind === "derived_research_view" && <p className="ai-review-result">已创建独立、可继续人工编辑的派生视图；冻结输入、AI 原稿和审查决定继续保留。</p>}
  </article>;
}
