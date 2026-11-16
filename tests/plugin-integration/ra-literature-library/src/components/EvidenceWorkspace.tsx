import { notify } from "@research-assistant/workbench-sdk/feedback";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";
import type {
  Claim,
  ClaimDraft,
  EvidenceLink,
  EvidenceStance,
  SourceAnchorSummary,
} from "@research-assistant/core/evidence";
import type { LiteratureItem } from "@research-assistant/core/literature";
import type { ResearchProject } from "@research-assistant/core/project";

import { AiRunHistoryPanel } from "../features/ai/AiRunHistoryPanel";
import { ResearchKnowledgePanel } from "../../../../../../plugins/ra-literature-library/src/features/knowledge/ResearchKnowledgePanel";
import { ResearchObjectHistory } from "../../../../../../plugins/ra-literature-library/src/features/knowledge/ResearchObjectHistory";
import { exportAllResearchKnowledge } from "../../../../../../plugins/ra-literature-library/src/features/knowledge/researchExportActions";
import { EvidenceMatrixPanel } from "../../../../../../plugins/ra-literature-library/src/features/synthesis/EvidenceMatrixPanel";
import { DerivedResearchViewPanel } from "../../../../../../plugins/ra-literature-library/src/features/synthesis/DerivedResearchViewPanel";
import { epistemicRoleLabels, provenanceLabels } from "../../../../../../plugins/ra-literature-library/src/features/knowledge/knowledgeLabels";
import { getEvidenceGateway } from "../../../../../../plugins/ra-literature-library/src/services/evidenceGateway";
import { getResearchKnowledgeGateway, researchKnowledgeErrorMessage } from "../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
import { getLiteratureGateway } from "../../../../../../plugins/ra-literature-library/src/services/literatureGateway";
import {
  getResearchExportGateway,
  ResearchExportGatewayError,
} from "../../../../../../plugins/ra-literature-library/src/services/researchExportGateway";
import "../../../../../../plugins/ra-literature-library/src/components/EvidenceWorkspace.css";

const PdfReader = lazy(async () => {
  const module = await import("../../../../../../plugins/ra-literature-library/src/components/PdfReader");
  return { default: module.PdfReader };
});

interface EvidenceWorkspaceProps {
  readonly project: ResearchProject | undefined;
  readonly openClaimId: string | undefined;
  readonly onClaimOpened: () => void;
}

function stanceLabel(stance: EvidenceStance): string {
  const labels: Record<EvidenceStance, string> = {
    supports: "支持",
    opposes: "反对",
    context: "背景",
  };
  return labels[stance];
}

export function EvidenceWorkspace({ project, openClaimId, onClaimOpened }: EvidenceWorkspaceProps): React.JSX.Element {
  const gateway = useMemo(() => getEvidenceGateway(), []);
  const knowledgeGateway = useMemo(() => getResearchKnowledgeGateway(), []);
  const literatureGateway = useMemo(() => getLiteratureGateway(), []);
  const exportGateway = useMemo(() => getResearchExportGateway(), []);
  const [claims, setClaims] = useState<readonly Claim[]>([]);
  const [sources, setSources] = useState<readonly SourceAnchorSummary[]>([]);
  const [evidenceByClaim, setEvidenceByClaim] = useState<ReadonlyMap<string, readonly EvidenceLink[]>>(new Map());
  const [selectedAnchorId, setSelectedAnchorId] = useState<string>();
  const [statement, setStatement] = useState("");
  const [epistemicRole, setEpistemicRole] = useState<ClaimDraft["epistemicRole"]>("assertion");
  const [editingClaim, setEditingClaim] = useState<Claim>();
  const [historyClaimId, setHistoryClaimId] = useState<string>();
  const [stance, setStance] = useState<EvidenceStance>("supports");
  const [rationale, setRationale] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [reading, setReading] = useState<{
    readonly item: LiteratureItem;
    readonly attachment: LiteratureItem["attachments"][number];
    readonly pageIndex: number;
    readonly anchorId: string;
  }>();
  const refreshSequence = useRef(0);

  async function refresh(): Promise<void> {
    const sequence = refreshSequence.current + 1;
    refreshSequence.current = sequence;
    const [nextClaims, nextSources] = await Promise.all([
      gateway.listClaims(openClaimId === undefined ? project?.id : undefined),
      gateway.listSources(project?.id),
    ]);
    const evidencePairs = await Promise.all(nextClaims.map(async (claim) => [
      claim.id,
      await gateway.listClaimEvidence(claim.id),
    ] as const));
    if (refreshSequence.current !== sequence) return;
    setClaims(nextClaims);
    setSources(nextSources);
    setEvidenceByClaim(new Map(evidencePairs));
    setSelectedAnchorId((current) => nextSources.some((source) => source.anchorId === current) ? current : undefined);
  }

  useEffect(() => {
    let isActive = true;
    setLoading(true);

    void refresh()
      .catch((reason: unknown) => {
        if (isActive) notify.error(researchKnowledgeErrorMessage(reason));
      })
      .finally(() => {
        if (isActive) setLoading(false);
      });
    return () => {
      isActive = false;
      refreshSequence.current += 1;
    };
  }, [gateway, openClaimId, project?.id]);

  useEffect(() => {
    if (openClaimId === undefined || !claims.some((claim) => claim.id === openClaimId)) return;
    document.getElementById(`claim-${openClaimId}`)?.scrollIntoView({ block: "center" });
    onClaimOpened();
  }, [claims, onClaimOpened, openClaimId]);

  function selectSource(source: SourceAnchorSummary): void {
    setSelectedAnchorId(source.anchorId);
    if (statement.trim().length === 0 && source.annotationBody.trim().length > 0) {
      setStatement(source.annotationBody);
    }
  }

  async function saveClaim(): Promise<void> {
    setSaving(true);

    const draft: ClaimDraft = {
      ...(editingClaim === undefined ? {} : {
        id: editingClaim.id,
        expectedRevision: editingClaim.revision,
        changeReason: "manual_edit",
      }),
      ...(project !== undefined
        ? { projectId: project.id }
        : editingClaim?.projectId === undefined ? {} : { projectId: editingClaim.projectId }),
      statement,
      epistemicRole,
    };
    try {
      if (editingClaim !== undefined) {
        if (selectedAnchorId === undefined) await gateway.saveClaim(draft);
        else await gateway.createClaimWithEvidence(draft, { anchorId: selectedAnchorId, stance, rationale });
        notify.info(selectedAnchorId === undefined ? "观点的新版本已保存" : "观点的新版本与所选来源已保存");
      } else if (selectedAnchorId === undefined) {
        await gateway.saveClaim({ ...draft, changeReason: "manual_create" });
        notify.info("观点已保存；当前尚未关联原文证据");
      } else {
        await gateway.createClaimWithEvidence(draft, { anchorId: selectedAnchorId, stance, rationale });
        notify.info("观点与原文证据已建立可追溯关联");
      }
      setStatement("");
      setRationale("");
      setSelectedAnchorId(undefined);
      setEpistemicRole("assertion");
      setEditingClaim(undefined);
      await refresh();
    } finally {
      setSaving(false);
    }
  }

  function startClaimEdit(claim: Claim): void {
    setEditingClaim(claim);
    setStatement(claim.statement);
    setEpistemicRole(claim.epistemicRole);
    setSelectedAnchorId(undefined);
    setRationale("");

  }

  function cancelClaimEdit(): void {
    setEditingClaim(undefined);
    setStatement("");
    setEpistemicRole("assertion");
    setSelectedAnchorId(undefined);
    setRationale("");
  }

  async function openSource(source: SourceAnchorSummary): Promise<void> {
    const item = await literatureGateway.get(source.itemId);
    const attachment = item.attachments.find((value) => value.id === source.attachmentId);
    if (attachment === undefined) throw new Error("原文附件已不可用；证据记录仍保留");
    setReading({ item, attachment, pageIndex: source.pageIndex, anchorId: source.anchorId });
  }

  async function exportResearchKnowledge(): Promise<void> {


    setExporting(true);
    try {
      const report = await exportAllResearchKnowledge(exportGateway);
      if (report === undefined) return;
      notify.info(`已导出 ${report.objectCount} 个研究对象、${report.sourceCount} 条证据来源和 ${report.fileCount} 个文件到“${report.folderName}”`);
    } catch (reason: unknown) {
      if (reason instanceof ResearchExportGatewayError && reason.code === "operation_cancelled") {
        notify.info("研究知识导出已取消；未留下不完整目录");
      } else {
        notify.error(reason instanceof Error ? reason.message : "研究知识导出失败；原始资料未改变");
      }
    } finally {
      setExporting(false);
    }
  }

  async function cancelResearchExport(): Promise<void> {
    try {
      if (await exportGateway.cancel()) notify.info("正在取消研究知识导出…");
    } catch (reason: unknown) {
      notify.error(reason instanceof Error ? reason.message : "无法取消研究知识导出");
    }
  }

  const selectedSource = sources.find((source) => source.anchorId === selectedAnchorId);

  return (
    <section className="evidence-workspace">
      <div className="evidence-heading">
        <div><span className="eyebrow">TRACEABLE CLAIMS</span><h2>{project?.title ?? "全部证据与观点"}</h2></div>
        <div className="evidence-heading-side">
          <p>观点与具体附件版本上的原文锚点分开保存；后续问答可直接消费同一条证据链。</p>
          <div className="evidence-heading-actions">
            <button className="ghost-button bordered" disabled={exporting} onClick={() => void exportResearchKnowledge()} type="button">{exporting ? "正在导出…" : "导出全部研究知识"}</button>
            {exporting && <button className="ghost-button" onClick={() => void cancelResearchExport()} type="button">取消导出</button>}
          </div>
        </div>
      </div>


      <div className="evidence-grid">
        <aside className="source-inbox" aria-busy={loading}>
          <div><span className="eyebrow">SOURCE ANCHORS</span><strong>{sources.length} 条阅读摘录</strong></div>
          {loading && <p role="status">正在读取原文锚点…</p>}
          {!loading && sources.length === 0 && <p>先在项目文献的 PDF 阅读器中创建高亮或页级笔记。</p>}
          {sources.map((source) => (
            <button className={source.anchorId === selectedAnchorId ? "source-card selected" : "source-card"} key={`${source.anchorId}-${source.annotationId ?? "anchor"}`} onClick={() => selectSource(source)} type="button">
              <span>{source.itemTitle} · 第 {source.pageLabel ?? source.pageIndex + 1} 页</span>
              <q>{source.quoteExact || source.annotationBody || "页级原文锚点"}</q>
              {source.annotationBody.length > 0 && source.quoteExact.length > 0 && <small>{source.annotationBody}</small>}
              <i>SHA-256 {source.contentSha256.slice(0, 10)}…</i>
            </button>
          ))}
        </aside>

        <div className="claim-column">
          <div className="claim-composer">
            <div><span className="eyebrow">{editingClaim === undefined ? "NEW CLAIM" : "EDIT CLAIM"}</span><strong>{editingClaim === undefined ? selectedSource === undefined ? "记录待验证观点" : "从选定原文形成观点" : `编辑观点 v${editingClaim.revision}`}</strong></div>
            {selectedSource !== undefined && <blockquote>{selectedSource.quoteExact || selectedSource.annotationBody || `第 ${selectedSource.pageIndex + 1} 页`}</blockquote>}
            <textarea aria-label="观点内容" onChange={(event) => setStatement(event.target.value)} placeholder="写下可以被证据支持、反对或补充背景的陈述" rows={4} value={statement} />
            <label className="claim-role-field">认识角色<select aria-label="观点认识角色" onChange={(event) => setEpistemicRole(event.target.value as ClaimDraft["epistemicRole"])} value={epistemicRole}><option value="assertion">观点陈述</option><option value="source_paraphrase">来源转述</option><option value="synthesis">人工综合</option><option value="inference">推论</option><option value="hypothesis">假设</option><option value="question">问题</option><option value="working_note">工作笔记</option></select></label>
            {selectedSource !== undefined && <div className="evidence-relation-fields"><select aria-label="证据关系" onChange={(event) => setStance(event.target.value as EvidenceStance)} value={stance}><option value="supports">支持观点</option><option value="opposes">反对观点</option><option value="context">提供背景</option></select><input onChange={(event) => setRationale(event.target.value)} placeholder="可选：说明这段原文为何相关" value={rationale} /></div>}
            <div className="form-actions"><button className="ghost-button" disabled={saving || (editingClaim === undefined && selectedSource === undefined)} onClick={editingClaim === undefined ? () => setSelectedAnchorId(undefined) : cancelClaimEdit} type="button">{editingClaim === undefined ? "取消来源选择" : "取消编辑"}</button><button className="primary-button" disabled={saving || statement.trim().length === 0} onClick={() => void saveClaim().catch((reason: unknown) => notify.error(`${researchKnowledgeErrorMessage(reason)}；你的观点草稿仍保留。`))} type="button">{saving ? "保存中…" : editingClaim === undefined ? selectedSource === undefined ? "保存观点" : "保存观点与证据" : "保存新版本"}</button></div>
          </div>

          <div className="claim-list">
            <div className="claim-list-heading"><span className="eyebrow">CLAIMS</span><strong>{claims.length} 条观点</strong></div>
            {loading && <p role="status">正在读取观点与证据…</p>}
            {!loading && claims.length === 0 && <div className="evidence-empty">选择左侧摘录，建立第一条可回查的证据关系。</div>}
            {claims.map((claim) => (
              <article className={claim.id === openClaimId ? "claim-card selected" : "claim-card"} id={`claim-${claim.id}`} key={claim.id}>
                <header><span>{provenanceLabels[claim.provenanceKind]}</span><small>{epistemicRoleLabels[claim.epistemicRole]} · v{claim.revision} · {claim.evidenceCount} 条证据</small></header>
                <h3>{claim.statement}</h3>
                <div className="claim-card-actions"><button onClick={() => startClaimEdit(claim)} type="button">编辑</button><button onClick={() => setHistoryClaimId(claim.id)} type="button">版本</button></div>
                <div className="claim-evidence-list">
                  {(evidenceByClaim.get(claim.id) ?? []).map((evidence) => (
                    <button key={evidence.id} onClick={() => void openSource(evidence.source).catch((reason: unknown) => notify.error(researchKnowledgeErrorMessage(reason)))} type="button">
                      <span className={`stance ${evidence.stance}`}>{stanceLabel(evidence.stance)}</span>
                      <q>{evidence.source.quoteExact || evidence.source.annotationBody || "页级原文锚点"}</q>
                      <small>{evidence.source.itemTitle} · 第 {evidence.source.pageLabel ?? evidence.source.pageIndex + 1} 页</small>
                      <b>回到原文 →</b>
                    </button>
                  ))}
                </div>
                {historyClaimId === claim.id && <ResearchObjectHistory gateway={knowledgeGateway} objectId={claim.id} objectKind="claim" onClose={() => setHistoryClaimId(undefined)} />}
              </article>
            ))}
          </div>
        </div>
      </div>


      <AiRunHistoryPanel {...(project === undefined ? {} : { projectId: project.id })} />

      {project !== undefined && (
        <EvidenceMatrixPanel
          onOpenSource={openSource}
          project={project}
          sources={sources}
        />
      )}

      {project !== undefined && (
        <DerivedResearchViewPanel
          onAiRunStarted={(run) => notify.info(`AI 多文献综合任务已启动（${run.id.slice(0, 8)}…）`)}
          onOpenSource={openSource}
          project={project}
          refreshVersion={0}
          sources={sources}
        />
      )}

      <ResearchKnowledgePanel {...(project === undefined ? {} : { project })} {...(selectedSource === undefined ? {} : { source: selectedSource })} />

      {reading !== undefined && (
        <Suspense fallback={<div className="reader-overlay"><div className="detail-empty">正在载入证据原文…</div></div>}>
          <PdfReader attachment={reading.attachment} focusAnchorId={reading.anchorId} gateway={literatureGateway} initialPageIndex={reading.pageIndex} item={reading.item} onClose={() => setReading(undefined)} />
        </Suspense>
      )}
    </section>
  );
}
