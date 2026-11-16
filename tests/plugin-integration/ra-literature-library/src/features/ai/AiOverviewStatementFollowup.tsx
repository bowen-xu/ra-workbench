import { notify } from "@research-assistant/workbench-sdk/feedback";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { AiAtomicStatementV1, AiEvidenceItem } from "@research-assistant/core/ai";
import type { ClaimDraft, EvidenceStance } from "@research-assistant/core/evidence";
import type { ReadingNoteCreationStorage } from "../../../../../../apps/web/src/core/public/noteStorage";

import type { EvidenceGateway } from "../../../../../../../plugins/ra-literature-library/src/services/evidenceGateway";
import {
  researchKnowledgeErrorMessage,
  type ResearchKnowledgeGateway,
} from "../../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import {
  ConceptCapturePanel,
  type ConceptCaptureValue,
} from "../../../../../../../plugins/ra-literature-library/src/features/knowledge/ConceptCapturePanel";

type FollowupMode = "closed" | "claim" | "note" | "concept";

interface AiOverviewStatementFollowupProps {
  readonly statement: AiAtomicStatementV1;
  readonly evidenceItems: readonly AiEvidenceItem[];
  readonly projectId?: string;
  readonly evidenceGateway: EvidenceGateway;
  readonly knowledgeGateway: ResearchKnowledgeGateway;
  readonly notesGateway: ReadingNoteCreationStorage;
  readonly onKnowledgeChanged: () => void;
}

const sourceStatusLabels = {
  exact: "精确定位",
  page_level: "页级定位",
  stale: "附件版本已变化",
  missing: "来源缺失",
} as const;

const claimRoles: readonly ClaimDraft["epistemicRole"][] = [
  "source_paraphrase", "assertion", "synthesis", "inference", "hypothesis", "question", "working_note",
];

function isUsableSource(evidence: AiEvidenceItem): boolean {
  return evidence.validationStatus === "valid"
    && (evidence.currentAnchorStatus === "exact" || evidence.currentAnchorStatus === "page_level");
}

export function AiOverviewStatementFollowup({
  statement,
  evidenceItems,
  projectId,
  evidenceGateway,
  knowledgeGateway,
  notesGateway,
  onKnowledgeChanged,
}: AiOverviewStatementFollowupProps): React.JSX.Element {
  const usableEvidence = evidenceItems.filter(isUsableSource);
  const [mode, setMode] = useState<FollowupMode>("closed");
  const [evidenceId, setEvidenceId] = useState(usableEvidence[0]?.id ?? "");
  const [claimText, setClaimText] = useState("");
  const [claimRole, setClaimRole] = useState<ClaimDraft["epistemicRole"]>("source_paraphrase");
  const [stance, setStance] = useState<EvidenceStance>("context");
  const [rationale, setRationale] = useState("");
  const [noteBody, setNoteBody] = useState("");
  const [saving, setSaving] = useState(false);
  const claimTriggerRef = useRef<HTMLButtonElement>(null);
  const noteTriggerRef = useRef<HTMLButtonElement>(null);
  const conceptTriggerRef = useRef<HTMLButtonElement>(null);
  const returnFocusRef = useRef<HTMLButtonElement | null>(null);
  const shouldRestoreFocusRef = useRef(false);

  useEffect(() => {
    if (!usableEvidence.some((evidence) => evidence.id === evidenceId)) {
      setEvidenceId(usableEvidence[0]?.id ?? "");
    }
  }, [evidenceId, usableEvidence]);

  useLayoutEffect(() => {
    if (mode !== "closed" || !shouldRestoreFocusRef.current) return;
    shouldRestoreFocusRef.current = false;
    returnFocusRef.current?.focus();
  }, [mode]);

  const source = usableEvidence.find((evidence) => evidence.id === evidenceId);
  const sourceIsPageLevel = source?.currentAnchorStatus === "page_level";
  const statementLabel = statement.text.trim().slice(0, 80) || statement.id;

  function open(
    nextMode: Exclude<FollowupMode, "closed">,
    trigger: HTMLButtonElement | null,
  ): void {
    returnFocusRef.current = trigger;
    setMode(nextMode);


    if (sourceIsPageLevel) setStance("context");
  }

  function close(): void {
    shouldRestoreFocusRef.current = true;
    setMode("closed");
  }

  async function saveClaim(): Promise<void> {
    if (source === undefined || claimText.trim().length === 0) return;
    setSaving(true);
    try {
      await evidenceGateway.createClaimWithEvidence({
        ...(projectId === undefined ? {} : { projectId }),
        statement: claimText,
        epistemicRole: claimRole,
        changeReason: "manual_from_overview_source",
      }, {
        anchorId: source.anchorId,
        stance: sourceIsPageLevel ? "context" : stance,
        rationale: rationale.trim() || "由用户从概要提示回到原文后手工创建",
      });
      setClaimText(""); setRationale(""); close();
      notify.info("人工观点已保存，并关联所选原文；AI 陈述未被复制进知识库");
      onKnowledgeChanged();
    } catch (reason) {
      notify.error(`${researchKnowledgeErrorMessage(reason)}；你的观点草稿仍保留。`);
    } finally { setSaving(false); }
  }

  async function saveNote(): Promise<void> {
    if (source === undefined || noteBody.trim().length === 0) return;
    setSaving(true);
    try {
      await notesGateway.createReadingNote({
        sourceAnchorId: source.anchorId,
        noteType: source.quoteExact.trim().length === 0 ? "page_note" : "annotation",
        body: noteBody,
        itemId: source.paperId,
        attachmentId: source.attachmentId,
        pageIndex: source.pageIndex,
        ...(source.pageLabel === undefined ? {} : { pageLabel: source.pageLabel }),
        rects: [],
        quoteExact: source.quoteExact,
        quotePrefix: source.quotePrefix,
        quoteSuffix: source.quoteSuffix,
      });
      setNoteBody(""); close();
      notify.info("人工阅读笔记已保存，并复用所选原文锚点；AI 陈述未被复制进笔记");
      onKnowledgeChanged();
    } catch (reason) {
      notify.error(`${researchKnowledgeErrorMessage(reason)}；你的笔记草稿仍保留。`);
    } finally { setSaving(false); }
  }

  async function saveConcept(value: ConceptCaptureValue): Promise<void> {
    if (source === undefined) return;
    setSaving(true);
    try {
      await knowledgeGateway.captureConceptUsage({ ...value, anchorId: source.anchorId }, {
        paperId: source.paperId,
        paperTitle: source.paperTitle,
        attachmentId: source.attachmentId,
        contentSha256: source.contentSha256,
        pageIndex: source.pageIndex,
        ...(source.pageLabel === undefined ? {} : { pageLabel: source.pageLabel }),
        quoteExact: source.quoteExact,
      });
      close();
      notify.info("人工概念用法已保存；同名概念仍保持显式消歧");
      onKnowledgeChanged();
    } catch (reason) {
      notify.error(`${researchKnowledgeErrorMessage(reason)}；你的概念草稿仍保留。`);
    } finally { setSaving(false); }
  }

  return <section className="ai-overview-followup" aria-label="从概要提示继续人工阅读">
    <div className="ai-overview-followup-actions">
      <span>人工后续</span>
      <button aria-label={`为概要陈述写观点：${statementLabel}`} disabled={usableEvidence.length === 0 || saving} onClick={() => open("claim", claimTriggerRef.current)} ref={claimTriggerRef} type="button">写观点</button>
      <button aria-label={`为概要陈述写阅读笔记：${statementLabel}`} disabled={usableEvidence.length === 0 || saving} onClick={() => open("note", noteTriggerRef.current)} ref={noteTriggerRef} type="button">写阅读笔记</button>
      <button aria-label={`从概要陈述添加概念：${statementLabel}`} disabled={usableEvidence.length === 0 || saving} onClick={() => open("concept", conceptTriggerRef.current)} ref={conceptTriggerRef} type="button">添加概念</button>
    </div>
    {usableEvidence.length === 0 && <small>引用已失效或缺失；先恢复原文定位，不能用快照伪造来源。</small>}
    {mode !== "closed" && source !== undefined && <div className="ai-overview-followup-editor">
      <div className="ai-overview-followup-source">
        <label>作为人工内容来源<select disabled={saving} onChange={(event) => setEvidenceId(event.target.value)} value={evidenceId}>{usableEvidence.map((evidence) => <option key={evidence.id} value={evidence.id}>{evidence.citationKey} · 第 {evidence.pageLabel ?? evidence.pageIndex + 1} 页 · {sourceStatusLabels[evidence.currentAnchorStatus]}</option>)}</select></label>
        <blockquote><b>原文</b><q>{source.quoteExact || source.contextText || "页级来源"}</q></blockquote>
        <details><summary>查看 AI 提示（仅供注意，不会自动写入）</summary><p>{statement.text}</p></details>
      </div>
      {mode === "claim" && <div className="knowledge-editor-card">
        <strong>从空白撰写人工观点</strong>
        <label>观点内容<textarea autoFocus onChange={(event) => setClaimText(event.target.value)} rows={4} value={claimText} /></label>
        <div className="knowledge-field-row"><label>认识角色<select onChange={(event) => setClaimRole(event.target.value as ClaimDraft["epistemicRole"])} value={claimRole}>{claimRoles.map((role) => <option key={role} value={role}>{role}</option>)}</select></label><label>证据关系<select disabled={sourceIsPageLevel} onChange={(event) => setStance(event.target.value as EvidenceStance)} value={sourceIsPageLevel ? "context" : stance}><option value="supports">支持</option><option value="opposes">反对</option><option value="context">背景</option></select></label></div>
        {sourceIsPageLevel && <small>当前只有页级定位，因此只能记录为背景来源。</small>}
        <label>关联说明（可选）<input onChange={(event) => setRationale(event.target.value)} value={rationale} /></label>
        <div className="form-actions"><button className="ghost-button" disabled={saving} onClick={close} type="button">取消</button><button className="primary-button" disabled={saving || claimText.trim().length === 0} onClick={() => void saveClaim()} type="button">{saving ? "保存中…" : "保存人工观点"}</button></div>
      </div>}
      {mode === "note" && <div className="knowledge-editor-card">
        <strong>从空白撰写人工阅读笔记</strong>
        <label>笔记内容<textarea autoFocus onChange={(event) => setNoteBody(event.target.value)} rows={4} value={noteBody} /></label>
        <div className="form-actions"><button className="ghost-button" disabled={saving} onClick={close} type="button">取消</button><button className="primary-button" disabled={saving || noteBody.trim().length === 0} onClick={() => void saveNote()} type="button">{saving ? "保存中…" : "保存人工笔记"}</button></div>
      </div>}
      {mode === "concept" && <ConceptCapturePanel gateway={knowledgeGateway} heading="从所选原文添加概念" initialTerm="" onCancel={close} onSave={(value) => void saveConcept(value)} quoteExact={source.quoteExact || source.contextText} saving={saving} />}
    </div>}
  </section>;
}
