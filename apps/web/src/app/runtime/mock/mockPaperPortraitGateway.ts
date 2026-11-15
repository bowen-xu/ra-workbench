import { isVisionByLogicAttachment, visionByLogicEvidence, visionByLogicPortrait } from "./visionByLogicFixture";
import { mockPaperTemplateGateway } from "./mockPaperTemplateGateway";
import {
  DEFAULT_PORTRAIT_TEMPLATE, DEFAULT_PORTRAIT_CATALOG, PORTRAIT_DIMENSIONS, portraitTagDefinition,
  decodeAiCandidatePayload, decodePortraitCatalog, validatePortraitQuotes, type AiCandidate, type AiRun,
  type AiTaskPreview, type AiTaskSpecDraft, type PaperPortraitCandidateV1,
} from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";

/** Static fixtures behind the production gateway; no provider or private PDF is contacted. */
export function mockPaperPortraitGateway(storage: Storage): Partial<AiAssistanceGateway> {
  let candidates: AiCandidate[] = JSON.parse(storage.getItem("portrait-candidates") ?? "[]");
  const runs = new Map<string, AiRun>();
  const previews = new Map<string, AiTaskPreview>();
  const persist = (next: AiCandidate[]): void => {
    storage.setItem("portrait-candidates", JSON.stringify(next));
    candidates = next;
  };
  const add = (candidate: AiCandidate): void => persist([candidate, ...candidates]);
  const find = (id: string): AiCandidate => { const value = candidates.find(candidate => candidate.id === id); if (value === undefined) throw new Error("找不到模拟画像"); return value; };
  const update = (candidate: AiCandidate): AiCandidate => {
    persist(candidates.map(value => value.id === candidate.id ? candidate : value));
    return candidate;
  };
  let seedPromise: Promise<void> | undefined;
  function ensureSourceFixture(): Promise<void> {
    return seedPromise ??= (async () => {
      if (storage.getItem("vision-portrait-fixture-version") === "1") return;
      const papers = await getLiteratureGateway().list();
      const paper = papers.find(paper => paper.attachments.some(isVisionByLogicAttachment));
      const attachment = paper?.attachments.find(isVisionByLogicAttachment);
      if (paper === undefined || attachment === undefined) return;
      add(visionByLogicPortrait(paper, attachment));
      storage.setItem("vision-portrait-fixture-version", "1");
    })().catch((reason: unknown) => { seedPromise = undefined; throw reason; });
  }
  async function previewTask(draft: AiTaskSpecDraft): Promise<AiTaskPreview> {
    if (draft.taskKind !== "paper_profile_classification") throw new Error("此模拟场景仅支持文献画像");
    if (draft.scopeItems.length < 1 || draft.scopeItems.length > 8 || new Set(draft.scopeItems.map(value => value.objectId)).size !== draft.scopeItems.length) throw new Error("请选择 1–8 个不同附件");
    const papers = await getLiteratureGateway().list();
    const paper = papers.find(item => item.id === draft.scopeItems[0]?.parentObjectId);
    if (paper === undefined || draft.scopeItems.some(item => item.parentObjectId !== paper.id)) throw new Error("请选择同一文献的附件");
    const attachments = draft.scopeItems.map(item => {
      const attachment = paper.attachments.find(value => value.id === item.objectId && value.mimeType === "application/pdf" && (item.contentSha256 === undefined || value.contentSha256 === item.contentSha256));
      if (attachment === undefined) throw new Error("找不到模拟附件或附件版本已变化");
      return attachment;
    });
    const bytes = new TextEncoder().encode(JSON.stringify(attachments.map(value => [value.id, value.contentSha256])));
    const scopeHash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), value => value.toString(16).padStart(2, "0")).join("");
    const sources = attachments.flatMap(attachment => {
      const quote = attachment.displayName === "sample.pdf" ? "Synthetic Research Preview" : paper.title.split(/[ :]/u)[0] ?? paper.title;
      return isVisionByLogicAttachment(attachment) ? visionByLogicEvidence(paper, attachment) : [{ citationKey: "E1", anchorId: `mock-portrait:${attachment.id}`, paperTitle: paper.title, attachmentId: attachment.id, contentSha256: attachment.contentSha256, pageIndex: 0, quoteExact: quote, contextText: quote, relationKind: "direct_quote" as const }];
    }).map((source, index) => ({ ...source, citationKey: `E${index + 1}` }));
    const scope: AiTaskPreview = { taskKind: draft.taskKind, providerId: "fixture-generation", modelId: attachments.some(isVisionByLogicAttachment) ? "原文整理样例 · 非实时生成" : "mock-portrait", isCloud: false,
      scopeHash, inputItemCount: attachments.length, requiresCloudConsent: false, scopeItems: [],
      evidencePreview: [] };
    // Synthetic parsed bodies belong to the mock tool/data boundary, not the task preview.
    previews.set(scopeHash, {...scope, evidencePreview: sources}); return scope;
  }
  return {
    ...mockPaperTemplateGateway(storage, { list: () => candidates, update, add }),
    previewTask,
    listCandidates: async (_projectId, scope) => {
      await ensureSourceFixture();
      return candidates.filter(candidate => scope === undefined || candidate.evidenceBundle.items.length > 0 &&
        candidate.evidenceBundle.items.every(source => source.paperId === scope.paperId));
    },
    listRuns: async () => [...runs.values()],
    startRun: async draft => {
      const scope = previews.get(draft.confirmedScopeHash ?? "");
      if (scope === undefined) throw new Error("请先预览画像范围");
      const evidence = scope.evidencePreview[0]; const paperId = draft.scopeItems[0]?.parentObjectId;
      if (evidence === undefined || paperId === undefined) throw new Error("模拟范围不完整");
      const id = crypto.randomUUID(); const now = new Date().toISOString();
      const payload: PaperPortraitCandidateV1 = { kind: "paper_portrait_v1", schemaVersion: 1,
        coverage: "模拟数据：仅展示第 1 页标题，未调用 AI，也未判断真实论文。",
        claims: PORTRAIT_DIMENSIONS.map((dimension, index) => ({ id: `claim_${index}`, dimension,
          label: "不确定", description: portraitTagDefinition(DEFAULT_PORTRAIT_CATALOG.tags, dimension, "不确定"),
          certainty: "uncertain", evidence: [{ evidenceItemId: "E1", quote: evidence.quoteExact }] })), template: DEFAULT_PORTRAIT_TEMPLATE };
      const paper = await getLiteratureGateway().get(paperId);
      const attachment = paper.attachments.find(value => isVisionByLogicAttachment(value) && scope.evidencePreview.some(source => source.attachmentId === value.id));
      let candidate: AiCandidate = attachment !== undefined && isVisionByLogicAttachment(attachment) ? visionByLogicPortrait(paper, attachment, id) : { id: crypto.randomUUID(), runId: id, providerId: scope.providerId, modelId: scope.modelId,
        candidateKind: "paper_portrait_v1", schemaVersion: 1, currentRevision: 1, reviewState: "unreviewed", validationStatus: "valid",
        originalPayloadHash: "b".repeat(64), createdAt: now, updatedAt: now,
        evidenceBundle: { id: crypto.randomUUID(), runId: id, scopeHash: scope.scopeHash, retrievalConfigVersion: "mock-portrait-v1", validationStatus: "valid", createdAt: now,
          items: [{ ...evidence, id: crypto.randomUUID(), ordinal: 0, paperId, quotePrefix: "", quoteSuffix: "", currentAnchorStatus: "page_level", validationStatus: "valid", createdAt: now }] },
        revisions: [{ id: crypto.randomUUID(), revision: 1, payload, payloadHash: "b".repeat(64), editorKind: "ai", sourceRunId: id, changeReason: "mock_generation", createdAt: now }],
        decisions: [], attentionFeedback: [], suggestionItemDecisions: [], citationAssessments: [], overviewFieldConfirmations: [], inspectedEvidenceItemIds: [] };
      const original = candidate.revisions[0]?.payload;
      if (original?.kind !== "paper_portrait_v1") throw new Error("模拟画像不完整");
      const mappedClaims = original.claims.map(claim => ({ ...claim, evidence: claim.evidence.map(source => {
        const previous = candidate.evidenceBundle.items.find(item => item.citationKey === source.evidenceItemId);
        const next = scope.evidencePreview.find(item => item.attachmentId === previous?.attachmentId && item.quoteExact === previous?.quoteExact);
        if (next === undefined) throw new Error("模拟来源不完整");
        return { ...source, evidenceItemId: next.citationKey };
      }) }));
      const marker = "分类标签库及阅读模板：";
      const config = draft.userGoal.includes(marker) ? decodePortraitCatalog(JSON.parse(draft.userGoal.slice(draft.userGoal.indexOf(marker) + marker.length)) as unknown) : undefined;
      const template = config?.templates.find(value => value.id === (attachment === undefined ? "general" : "formal"))?.fields ?? original.template;
      const claims = mappedClaims.flatMap(claim => claim.dimension === "contribution" || claim.dimension === "structure" ? [] : [{ ...claim, description: portraitTagDefinition(config?.tags ?? DEFAULT_PORTRAIT_CATALOG.tags, claim.dimension, claim.label) }]);
      const customDimensions = config?.customDimensions;
      const generated: PaperPortraitCandidateV1 = { ...original, claims: [...claims, ...(customDimensions ?? []).map(dimension => ({ id: 'claim_' + dimension.id, dimension: dimension.id, label: "不确定", description: portraitTagDefinition(config?.tags ?? DEFAULT_PORTRAIT_CATALOG.tags, dimension.id, "不确定"), certainty: "uncertain" as const, evidence: [] }))], template, ...(customDimensions === undefined ? {} : { customDimensions }) };
      const bundle = { ...candidate.evidenceBundle, scopeHash: scope.scopeHash, items: scope.evidencePreview.map((source, ordinal) => ({ ...source, id: crypto.randomUUID(), ordinal, paperId, quotePrefix: "", quoteSuffix: "", currentAnchorStatus: "page_level" as const, validationStatus: "valid" as const, createdAt: now })) };
      validatePortraitQuotes(generated, bundle.items);
      candidate = { ...candidate, evidenceBundle: bundle, revisions: candidate.revisions.map((revision, index) => index === 0 ? { ...revision, payload: generated } : revision) };
      add(candidate);
      const run: AiRun = { id, taskKind: draft.taskKind, status: "needs_review", providerId: scope.providerId, modelId: scope.modelId,
        pipelineVersion: draft.pipelineVersion, promptVersion: draft.promptVersion, outputSchemaVersion: 1, userGoal: draft.userGoal,
        scopeHash: scope.scopeHash, inputItemCount: scope.inputItemCount, maxInputTokens: draft.maxInputTokens, maxOutputTokens: draft.maxOutputTokens, timeoutMs: draft.timeoutMs,
        cancelRequested: false, createdAt: now, updatedAt: now, scopeItems: [], stages: [] };
      runs.set(id, run); return run;
    },
    getRun: async id => { const run = runs.get(id); if (run === undefined) throw new Error("找不到模拟运行"); return run; },
    cancelRun: async id => { const run = runs.get(id); if (run === undefined) return false; runs.set(id, { ...run, status: "canceled" }); return true; },
    reviseCandidate: async draft => {
      const candidate = find(draft.candidateId);
      if (["accepted_unchanged", "accepted_after_edit", "rejected"].includes(candidate.reviewState)) throw new Error("历史画像只读");
      if (candidate.currentRevision !== draft.expectedRevision) throw new Error("画像版本冲突");
      const payload = decodeAiCandidatePayload(draft.payload, { allowedEvidenceItemIds: new Set(candidate.evidenceBundle.items.map(value => value.citationKey)) });
      if (payload.kind === "paper_portrait_v1") validatePortraitQuotes(payload, candidate.evidenceBundle.items);
      else if (payload.kind === "paper_overview_unit_v1") {
        const current = candidate.revisions.find(value => value.revision === candidate.currentRevision)?.payload;
        if (current?.kind !== payload.kind || current.profileId !== payload.profileId || JSON.stringify(current.profileRefs) !== JSON.stringify(payload.profileRefs) || JSON.stringify(current.paperTemplate) !== JSON.stringify(payload.paperTemplate)) throw new Error("修订不能改变冻结模板或阅读框架");
        for (const statement of payload.fields.flatMap(field => field.statements)) for (const quote of statement.evidenceQuotes ?? []) {
          if (!candidate.evidenceBundle.items.some(source => source.citationKey === quote.evidenceItemId && source.quoteExact.includes(quote.quote))) throw new Error("摘录不是连续原文");
        }
      } else throw new Error("模拟候选类型不兼容");
      return update({ ...candidate, currentRevision: candidate.currentRevision + 1, reviewState: "edited", inspectedEvidenceItemIds: [], revisions: [...candidate.revisions,
        { id: crypto.randomUUID(), revision: candidate.currentRevision + 1, parentRevision: candidate.currentRevision, editorKind: "human", payload,
          payloadHash: "c".repeat(64), changeReason: draft.changeReason, createdAt: new Date().toISOString() }] });
    },
    recordEvidenceInspection: async (id, revision, evidenceId) => {
      const candidate = find(id);
      if (candidate.currentRevision !== revision || !candidate.evidenceBundle.items.some(value => value.id === evidenceId)) throw new Error("来源版本冲突");
      update({ ...candidate, inspectedEvidenceItemIds: [...new Set([...(candidate.inspectedEvidenceItemIds ?? []), evidenceId])] }); return true;
    },
    recordCitationAssessment: async draft => {
      const candidate = find(draft.candidateId);
      if (["accepted_unchanged", "accepted_after_edit", "rejected"].includes(candidate.reviewState)) throw new Error("历史画像只读");
      if (candidate.currentRevision !== draft.candidateRevision) throw new Error("画像版本冲突");
      const prior = candidate.citationAssessments.filter(value => value.candidateRevision === draft.candidateRevision
        && value.statementId === draft.statementId && value.evidenceItemId === draft.evidenceItemId);
      if (prior.some(value => value.idempotencyKey === draft.idempotencyKey)) return candidate;
      if (Math.max(0, ...prior.map(value => value.assessmentRevision)) !== draft.expectedAssessmentRevision) throw new Error("引用审查版本冲突");
      const { expectedAssessmentRevision, ...assessment } = draft;
      return update({ ...candidate, citationAssessments: [...candidate.citationAssessments, { ...assessment, id: crypto.randomUUID(), assessmentRevision: expectedAssessmentRevision + 1, assessedAt: new Date().toISOString() }] });
    },
    decideCandidate: async draft => {
      const candidate = find(draft.candidateId);
      const repeated = candidate.decisions.find(value => value.idempotencyKey === draft.idempotencyKey);
      if (repeated !== undefined) {
        if (repeated.candidateRevision !== draft.candidateRevision || repeated.action !== draft.action || repeated.rationale !== draft.rationale) throw new Error("审查幂等键冲突");
        return candidate;
      }
      if (candidate.currentRevision !== draft.candidateRevision) throw new Error("画像版本冲突");
      if (["accepted_unchanged", "accepted_after_edit", "rejected"].includes(candidate.reviewState)) throw new Error("历史画像只读");
      if (draft.action.startsWith("accept_")) {
        const revision = candidate.revisions.find(value => value.revision === candidate.currentRevision);
        if (revision?.payload.kind !== "paper_portrait_v1") throw new Error("画像结构错误");
        if (draft.action === "accept_after_edit" && revision.editorKind !== "human"
          || draft.action === "accept_unchanged" && revision.editorKind === "human") throw new Error("采纳方式与当前版本不匹配");
        validatePortraitQuotes(revision.payload, candidate.evidenceBundle.items);
      }
      return update({ ...candidate, reviewState: draft.action === "reject" ? "rejected" : draft.action === "accept_after_edit" ? "accepted_after_edit" : draft.action === "defer" ? "deferred" : draft.action === "request_more_evidence" ? "needs_more_evidence" : "accepted_unchanged",
        decisions: [...candidate.decisions, { id: crypto.randomUUID(), candidateId: candidate.id, candidateRevision: draft.candidateRevision, action: draft.action, rationale: draft.rationale, idempotencyKey: draft.idempotencyKey, createdAt: new Date().toISOString(),
          ...(draft.action.startsWith("accept_") ? { targetObjectKind: "paper_portrait" as const, targetObjectId: candidate.id, targetObjectRevision: candidate.currentRevision } : {}) }] });
    },
  };
}
