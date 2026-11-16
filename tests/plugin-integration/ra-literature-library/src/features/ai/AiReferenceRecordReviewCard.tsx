import { notify } from "../../../../../../apps/web/src/core/public/feedback";
import { useEffect, useRef, useState } from "react";
import type {
  AiCandidate,
  AiEvidenceItem,
  AiReferenceFieldName,
  AiReferenceRecordCandidateV1,
  AiReviewAction,
} from "@research-assistant/core/ai";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { aiAssistanceErrorMessage } from "../../../../../../apps/web/src/core/public/aiAssistance";
import { AiCitationReviewList, isCandidateCitationReady } from "./AiCitationReviewList";
import { useEvidenceInspectionState } from "../../../../../../../plugins/ra-literature-library/src/features/ai/useEvidenceInspectionState";

interface AiReferenceRecordReviewCardProps {
  readonly candidate: AiCandidate;
  readonly gateway: AiAssistanceGateway;
  readonly projectId?: string;
  readonly onChanged: (candidate: AiCandidate, didCreateKnowledge: boolean) => void;
  readonly onOpenEvidence: (evidence: AiEvidenceItem) => Promise<void>;
}

const fieldLabels: Readonly<Record<AiReferenceFieldName, string>> = {
  title: "标题",
  authors: "作者",
  year: "年份",
  doi: "DOI",
};
const fieldNames = Object.keys(fieldLabels) as readonly AiReferenceFieldName[];

function currentPayload(candidate: AiCandidate): AiReferenceRecordCandidateV1 | undefined {
  const revision = candidate.revisions.find((item) => item.revision === candidate.currentRevision);
  return revision?.payload.kind === "reference_record_candidate_v1"
    ? revision.payload
    : undefined;
}

function fieldValues(payload: AiReferenceRecordCandidateV1) {
  return Object.fromEntries(payload.fields.map((field) => [field.name, field.value])) as
    Readonly<Partial<Record<AiReferenceFieldName, string>>>;
}

export function AiReferenceRecordReviewCard({
  candidate,
  gateway,
  projectId,
  onChanged,
  onOpenEvidence,
}: AiReferenceRecordReviewCardProps): React.JSX.Element {
  const payload = currentPayload(candidate);
  const [rawText, setRawText] = useState(payload?.rawText ?? "");
  const [fields, setFields] = useState(() => payload === undefined ? {} : fieldValues(payload));
  const [uncertainties, setUncertainties] = useState(payload?.uncertainties.join("\n") ?? "");
  const [rationale, setRationale] = useState("");
  const [busy, setBusy] = useState(false);
  const [openedEvidenceIds, markEvidenceInspected] = useEvidenceInspectionState(candidate);
  const idempotencyKeys = useRef(new Map<string, string>());

  useEffect(() => {
    const next = currentPayload(candidate);
    setRawText(next?.rawText ?? "");
    setFields(next === undefined ? {} : fieldValues(next));
    setUncertainties(next?.uncertainties.join("\n") ?? "");
  }, [candidate.id, candidate.currentRevision]);

  if (payload === undefined) {
    return <article className="ai-review-card"><p>参考条目候选结构不兼容，未提供审查操作。</p></article>;
  }
  const reviewedPayload = payload;
  const originalPayload = candidate.revisions[0]?.payload.kind === "reference_record_candidate_v1"
    ? candidate.revisions[0].payload
    : undefined;
  const originalFields = originalPayload === undefined ? {} : fieldValues(originalPayload);
  const reviewedFields = fieldValues(reviewedPayload);
  const currentRevision = candidate.revisions.find((item) =>
    item.revision === candidate.currentRevision);
  const acceptsAsHumanEdit = currentRevision?.editorKind === "human";
  const isTerminal = ["accepted_unchanged", "accepted_after_edit", "rejected"]
    .includes(candidate.reviewState);
  const hasChanges = rawText !== reviewedPayload.rawText
    || uncertainties !== reviewedPayload.uncertainties.join("\n")
    || fieldNames.some((name) => (fields[name] ?? "")
      !== (reviewedPayload.fields.find((field) => field.name === name)?.value ?? ""));
  const directEvidence = candidate.evidenceBundle.items.find((item) =>
    item.relationKind === "direct_quote");

  async function saveRevision(): Promise<void> {
    if (!hasChanges || directEvidence === undefined) return;
    setBusy(true);
    try {
      const nextPayload: AiReferenceRecordCandidateV1 = {
        ...reviewedPayload,
        rawText: rawText.trim(),
        fields: fieldNames.flatMap((name) => {
          const value = fields[name]?.trim() ?? "";
          if (value === "") return [];
          const existing = reviewedPayload.fields.find((field) => field.name === name);
          return [{
            name,
            value,
            evidenceItemIds: existing?.evidenceItemIds ?? [directEvidence.citationKey],
          }];
        }),
        uncertainties: uncertainties.split("\n").map((item) => item.trim()).filter(Boolean),
      };
      const revised = await gateway.reviseCandidate({
        candidateId: candidate.id,
        expectedRevision: candidate.currentRevision,
        payload: nextPayload,
        changeReason: "human_reference_field_review",
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

  return <article className={`ai-review-card ${isTerminal ? "terminal" : ""}`}>
    <header><div><span className="eyebrow">AI REFERENCE CANDIDATE</span><strong>{candidate.providerId} · {candidate.modelId}</strong></div><small>{candidate.reviewState} · 候选 v{candidate.currentRevision}</small></header>
    <p>这是字段提取候选，不是文献身份匹配结果。即使接受，身份和 DOI 仍保持“未核验”。</p>
    {candidate.currentRevision > 1 && originalPayload !== undefined && <details className="ai-edit-diff">
      <summary>查看 AI 原稿与当前人工 revision 差异</summary>
      <div>
        {originalPayload.rawText !== reviewedPayload.rawText && <p><b>原始条目：</b> <del>{originalPayload.rawText}</del> <ins>{reviewedPayload.rawText}</ins></p>}
        {fieldNames.map((name) => originalFields[name] === reviewedFields[name] ? null : <p key={name}><b>{fieldLabels[name]}：</b> <del>{originalFields[name] ?? "（空）"}</del> <ins>{reviewedFields[name] ?? "（空）"}</ins></p>)}
        {originalPayload.uncertainties.join("；") !== reviewedPayload.uncertainties.join("；") && <p><b>不确定性：</b> <del>{originalPayload.uncertainties.join("；") || "（空）"}</del> <ins>{reviewedPayload.uncertainties.join("；") || "（空）"}</ins></p>}
      </div>
    </details>}
    <label>原始参考条目<textarea disabled={busy || isTerminal} onChange={(event) => setRawText(event.target.value)} rows={4} value={rawText} /></label>
    <div className="ai-reference-fields">{fieldNames.map((name) => <label key={name}>{fieldLabels[name]}<input disabled={busy || isTerminal} onChange={(event) => setFields((current) => ({ ...current, [name]: event.target.value }))} placeholder="原文未出现时请留空" value={fields[name] ?? ""} /></label>)}</div>
    <label>不确定性<textarea disabled={busy || isTerminal} onChange={(event) => setUncertainties(event.target.value)} rows={3} value={uncertainties} /></label>
    <section><strong>原始条目引用</strong><AiCitationReviewList candidate={candidate}
      evidenceItemIds={payload.evidenceItemIds} gateway={gateway} onChanged={onChanged}
      onOpenEvidence={openEvidence} statementId="reference-record" /></section>
    {payload.fields.map((field) => <section key={field.name}><strong>{fieldLabels[field.name]}字段引用</strong>
      <AiCitationReviewList candidate={candidate} evidenceItemIds={field.evidenceItemIds}
        gateway={gateway} onChanged={onChanged} onOpenEvidence={openEvidence}
        statementId={`reference-field:${field.name}`} /></section>)}
    {!isTerminal && <>
      <label>审查说明（可选）<input disabled={busy} onChange={(event) => setRationale(event.target.value)} value={rationale} /></label>
      <div className="ai-review-actions"><button disabled={busy || !hasChanges || rawText.trim() === ""} onClick={() => void saveRevision()} type="button">保存人工修订</button><button disabled={busy || hasChanges || !isCandidateCitationReady(candidate) || !acceptsAsHumanEdit && openedEvidenceIds.size === 0} onClick={() => void decide(acceptsAsHumanEdit ? "accept_after_edit" : "accept_unchanged")} type="button">{acceptsAsHumanEdit ? "接受人工修订" : "原样接受为待核验条目"}</button><button disabled={busy || hasChanges} onClick={() => void decide("request_more_evidence")} type="button">需要更多证据</button><button disabled={busy || hasChanges} onClick={() => void decide("defer")} type="button">暂缓</button><button disabled={busy || hasChanges} onClick={() => void decide("reject")} type="button">拒绝</button></div>
      {hasChanges && <small>先保存人工 revision，再作审查决定。新增字段应当能从所选原文直接核对；外部检索结果请在正式参考条目中另行核验。</small>}
      {!hasChanges && !acceptsAsHumanEdit && openedEvidenceIds.size === 0 && <small>原样接受前，请至少打开一条实际来源；查看来源不等于完成身份核验。</small>}
      {!hasChanges && !isCandidateCitationReady(candidate) && <small>原始条目及每个非空字段都需要人工确认支持引用；身份核验仍是独立步骤。</small>}
    </>}
    {isTerminal && candidate.decisions.at(-1)?.targetObjectKind === "reference_record" && <p className="ai-review-result">已创建可继续人工编辑的参考条目；身份仍未核验，AI 原稿和审查决定继续保留。</p>}
  </article>;
}
