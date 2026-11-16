import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { overviewFieldTitle } from "@research-assistant/core/ai";
import { useEffect, useMemo, useRef, useState } from "react";
import type {
  AiCandidate,
  AiEvidenceItem,
  AiReviewAction,
  OverviewFieldStatus,
  PaperOverviewUnitCandidateV1,
} from "@research-assistant/core/ai";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { aiAssistanceErrorMessage } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { getEvidenceGateway } from "../../../../../../../plugins/ra-literature-library/src/services/evidenceGateway";
import { getResearchKnowledgeGateway } from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import { getReadingNoteCreationGateway } from "../../../../../../apps/web/src/core/public/noteStorage";
import { AiAttentionFeedbackControls } from "./AiAttentionFeedbackControls";
import { AiOverviewRetryPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/AiOverviewRetryPanel";
import { OverviewFieldSources } from "../../../../../../../plugins/ra-literature-library/src/features/ai/OverviewFieldSources";
import { AiOverviewStatementFollowup } from "./AiOverviewStatementFollowup";
import "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperOverviewPanel.css";

interface AiPaperOverviewReviewCardProps {
  readonly candidate: AiCandidate;
  readonly gateway: AiAssistanceGateway;
  readonly projectId?: string;
  readonly onChanged: (candidate: AiCandidate, didCreateKnowledge: boolean) => void;
  readonly onKnowledgeChanged: () => void;
  readonly onOpenEvidence: (evidence: AiEvidenceItem) => Promise<void>;
}



const roleLabels = {
  source_paraphrase: "来源转述",
  synthesis: "综合",
  inference: "推论",
  uncertain: "不确定",
} as const;

function currentPayload(candidate: AiCandidate): PaperOverviewUnitCandidateV1 | undefined {
  const revision = candidate.revisions.find((item) => item.revision === candidate.currentRevision);
  return revision?.payload.kind === "paper_overview_unit_v1" ? revision.payload : undefined;
}

function statementTexts(payload: PaperOverviewUnitCandidateV1): Readonly<Record<string, string>> {
  return Object.fromEntries(payload.fields.flatMap((field) =>
    field.statements.map((statement) => [statement.id, statement.text])));
}

function fieldStatuses(payload: PaperOverviewUnitCandidateV1): Readonly<Record<string, OverviewFieldStatus>> {
  return Object.fromEntries(payload.fields.map((field) => [field.field, field.status]));
}

export function AiPaperOverviewReviewCard({
  candidate,
  gateway,
  projectId,
  onChanged,
  onKnowledgeChanged,
  onOpenEvidence,
}: AiPaperOverviewReviewCardProps): React.JSX.Element {
  const evidenceGateway = useMemo(() => getEvidenceGateway(), []);
  const knowledgeGateway = useMemo(() => getResearchKnowledgeGateway(), []);
  const notesGateway = useMemo(() => getReadingNoteCreationGateway(), []);
  const payload = currentPayload(candidate);
  const original = candidate.revisions[0]?.payload.kind === "paper_overview_unit_v1"
    ? candidate.revisions[0].payload : undefined;
  const [texts, setTexts] = useState<Readonly<Record<string, string>>>(() =>
    payload === undefined ? {} : statementTexts(payload));
  const [statuses, setStatuses] = useState<Readonly<Record<string, OverviewFieldStatus>>>(() =>
    payload === undefined ? {} : fieldStatuses(payload));
  const [rationale, setRationale] = useState("");
  const [busy, setBusy] = useState(false);
  const idempotencyKeys = useRef(new Map<string, string>());

  useEffect(() => {
    const next = currentPayload(candidate);
    setTexts(next === undefined ? {} : statementTexts(next));
    setStatuses(next === undefined ? {} : fieldStatuses(next));
  }, [candidate.id, candidate.currentRevision]);

  if (payload === undefined) {
    return <article className="ai-review-card"><p>概要候选结构不兼容，未提供审查操作。</p></article>;
  }
  const reviewedPayload = payload;
  const isTerminal = ["accepted_unchanged", "accepted_after_edit", "rejected"].includes(candidate.reviewState);
  const hasConfirmedFields = candidate.overviewFieldConfirmations.some((confirmation) => confirmation.active);
  const isReviewLocked = isTerminal || hasConfirmedFields;
  const currentRevision = candidate.revisions.find((item) => item.revision === candidate.currentRevision);
  const hasChanges = payload.fields.some((field) => field.status !== statuses[field.field]
    || field.statements.some((statement) => statement.text !== texts[statement.id]));

  async function saveRevision(): Promise<void> {
    if (!hasChanges) return;
    setBusy(true);
    try {
      const nextPayload: PaperOverviewUnitCandidateV1 = {
        ...reviewedPayload,
        fields: reviewedPayload.fields.map((field) => ({
          ...field,
          status: statuses[field.field] ?? field.status,
          statements: field.statements.map((statement) => ({
            ...statement,
            text: texts[statement.id]?.trim() ?? "",
          })),
        })),
      };
      const revised = await gateway.reviseCandidate({
        candidateId: candidate.id,
        expectedRevision: candidate.currentRevision,
        payload: nextPayload,
        changeReason: "human_overview_field_review",
      });
      onChanged(revised, false);
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
    finally { setBusy(false); }
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
      onChanged(updated, false);
    } catch (reason) { notify.error(aiAssistanceErrorMessage(reason)); }
    finally { setBusy(false); }
  }

  return <article className={`ai-review-card ai-overview-review ${isTerminal ? "terminal" : ""}`}>
    <header><div><span className="eyebrow">AI PAPER OVERVIEW</span><strong>{candidate.providerId} · {candidate.modelId}</strong></div><small>{candidate.reviewState} · 候选 v{candidate.currentRevision} · {candidate.evidenceBundle.retrievalConfigVersion}</small></header>
    <p>范围 {candidate.evidenceBundle.scopeHash.slice(0, 12)}… · 附件 SHA {candidate.evidenceBundle.items[0]?.contentSha256.slice(0, 12)}… · {payload.profileId}{payload.profileRefs === undefined ? "" : ` · ${payload.profileRefs.map((profile) => `${profile.profileId}@${profile.profileVersion}`).join(" + ")}`}</p>
    {candidate.currentRevision > 1 && original !== undefined && <details className="ai-edit-diff"><summary>查看初始 AI 原稿与当前 revision</summary><p>初始 AI 原稿保持不可变；当前 v{candidate.currentRevision} 由 {currentRevision?.editorKind === "human" ? "人工修订" : "字段级 AI 重试"}产生，完整来源 Run 记录在版本历史中。</p></details>}
    {hasConfirmedFields && <p><span>已有字段被确认；为防止候选与绿色正式内容错位，本候选的继续编辑与重试已锁定。请先在文献详情的“文献画像”中撤销确认。</span></p>}
    <div className="ai-overview-fields">
      {payload.fields.map((field) => <section key={field.field}>
        <header><strong>{overviewFieldTitle(payload, field.field)}</strong><select aria-label={`${overviewFieldTitle(payload, field.field)}状态`} disabled={busy || isReviewLocked || field.statements.length === 0} onChange={(event) => setStatuses((current) => ({ ...current, [field.field]: event.target.value as OverviewFieldStatus }))} value={statuses[field.field] ?? field.status}>{field.statements.length === 0 ? <option value="not_found">未找到可靠来源</option> : <><option value="filled">已填写</option><option value="partial">部分／待补证</option></>}</select></header>
        {field.statements.length === 0 ? <p>本次有界证据中未找到可靠来源；没有生成占位事实。</p> : field.statements.map((statement) => <div className="ai-overview-statement" key={statement.id}>
          <label><span>{roleLabels[statement.epistemicRole]}</span><textarea disabled={busy || isReviewLocked} onChange={(event) => setTexts((current) => ({ ...current, [statement.id]: event.target.value }))} rows={3} value={texts[statement.id] ?? ""} /></label>
          <OverviewFieldSources candidate={candidate} field={{ ...field, statements: [statement] }} gateway={gateway}
            onChanged={updated => onChanged(updated, false)} onOpenEvidence={onOpenEvidence} />
          <AiAttentionFeedbackControls candidate={candidate} gateway={gateway}
            suggestionId={statement.id} />
          <AiOverviewStatementFollowup
            evidenceGateway={evidenceGateway}
            evidenceItems={candidate.evidenceBundle.items.filter((evidence) => statement.evidenceItemIds.includes(evidence.id))}
            key={`${candidate.currentRevision}:${statement.id}:followup`}
            knowledgeGateway={knowledgeGateway}
            notesGateway={notesGateway}
            onKnowledgeChanged={onKnowledgeChanged}
            statement={statement}
            {...(projectId === undefined ? {} : { projectId })}
          />
        </div>)}
      </section>)}
    </div>
    {!isTerminal && <AiOverviewRetryPanel candidate={candidate} disabled={busy || hasChanges || hasConfirmedFields}
      gateway={gateway} payload={payload} />}
    {!isTerminal && <>
      <label>审查说明（可选）<input disabled={busy} onChange={(event) => setRationale(event.target.value)} value={rationale} /></label>
      <div className="ai-review-actions"><button disabled={busy || !hasChanges || hasConfirmedFields} onClick={() => void saveRevision()} type="button">保存逐字段人工修订</button><button disabled={busy || hasChanges || hasConfirmedFields} onClick={() => void decide("request_more_evidence")} type="button">需要更多证据</button><button disabled={busy || hasChanges || hasConfirmedFields} onClick={() => void decide("defer")} type="button">暂缓</button><button disabled={busy || hasChanges || hasConfirmedFields} onClick={() => void decide("reject")} type="button">拒绝</button></div>
      {hasChanges && <small>先保存人工 revision，再作审查决定；未保存内容不会覆盖 AI 原稿。</small>}
      {!hasConfirmedFields && <small>字段确认、整块确认和撤销统一在当前文献详情的“文献画像”区域完成；确认需先审查原文引用。</small>}
    </>}
    {isTerminal && candidate.decisions.at(-1)?.targetObjectId !== undefined && <p className="ai-review-result">已按非空概要字段创建独立、可继续人工编辑的阅读单元；AI 原稿仍保留。</p>}
  </article>;
}
