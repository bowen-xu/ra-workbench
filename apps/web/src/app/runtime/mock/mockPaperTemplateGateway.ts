import { isVisionByLogicAttachment, visionByLogicReadingFields } from "./visionByLogicFixture";
import { decodeAiCandidatePayload, overviewFieldTitle, type AiCandidate, type PaperAnalysisWorkflow,
  type AiOverviewConfirmationBatch } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { STRUCTURED_READING_UNIT_KINDS, type StructuredReadingUnit } from "@research-assistant/core/knowledge";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { browserKnowledgeStorage } from "../../../../../../plugins/literature_library/src/services/browserKnowledgeStorage";
import { mockPaperExtractionInput } from "./mockPaperExtractionInput";
import { readMockPaperContext } from "./mockParsedPaperSources";

interface CandidateStore {
  readonly list: () => readonly AiCandidate[];
  readonly add: (candidate: AiCandidate) => void;
  readonly update: (candidate: AiCandidate) => AiCandidate;
}

/** Data-only adapter: extraction, review and formal records use the production UI. */
export function mockPaperTemplateGateway(storage: Storage, store: CandidateStore): Partial<AiAssistanceGateway> {
  const workflows: PaperAnalysisWorkflow[] = JSON.parse(storage.getItem("template-workflows") ?? "[]");
  const batches: AiOverviewConfirmationBatch[] = JSON.parse(storage.getItem("template-confirmations") ?? "[]");
  const priorUnits: Record<string, readonly StructuredReadingUnit[]> = JSON.parse(storage.getItem("template-confirmation-prior-units") ?? "{}");
  const reviewResets: { key: string; confirmationId: string }[] = JSON.parse(storage.getItem("template-review-resets") ?? "[]");
  const persist = (): void => {
    storage.setItem("template-workflows", JSON.stringify(workflows));
    storage.setItem("template-confirmations", JSON.stringify(batches));
    storage.setItem("template-confirmation-prior-units", JSON.stringify(priorUnits));
    storage.setItem("template-review-resets", JSON.stringify(reviewResets));
  };
  const find = (id: string): AiCandidate => {
    const candidate = store.list().find(value => value.id === id);
    if (candidate === undefined) throw new Error("模拟候选不存在");
    return candidate;
  };
  return {
    preparePaperReadingContext: (paperId, attachmentIds) => readMockPaperContext(storage, paperId, attachmentIds),
    getLatestPaperAnalysisWorkflow: async paperId => workflows.find(value => value.paperId === paperId),
    startPaperAnalysis: async draft => {
      const paper = await getLiteratureGateway().get(draft.paperId);
      const { definition, fieldIds, source, sources } = await mockPaperExtractionInput(draft, paper, store.list());
      const context = draft.attachmentIds.map(id => {
        const attachment = paper.attachments.find(value => value.id === id);
        if (attachment === undefined) throw new Error("模拟附件不存在");
        return { id, sha256: attachment.contentSha256 };
      }).sort((a, b) => a.id.localeCompare(b.id));
      const contextSnapshotId = [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(JSON.stringify(context))))]
        .map(value => value.toString(16).padStart(2, "0")).join("");
      const existing = workflows.find(value => value.paperId === draft.paperId && value.contextSnapshotId === contextSnapshotId);
      if (existing !== undefined && !existing.turns.some(turn => turn.kind === "paper_concept_map") && !["failed", "canceled"].includes(existing.status)) {
        const previous = store.list().find(value => value.id === existing.turns.find(turn => turn.kind === "paper_overview")?.candidateId)?.revisions.at(-1)?.payload;
        if (!draft.regenerate && previous?.kind === "paper_overview_unit_v1" && JSON.stringify({ profileId: previous.profileId, ...(previous.profileRefs === undefined ? {} : { profileRefs: previous.profileRefs }), ...(previous.paperTemplate === undefined ? {} : { paperTemplate: previous.paperTemplate }) }) === JSON.stringify(definition)) throw new Error("当前论文上下文已经存在分析工作流");
      }
      const evidence = sources.length === 0 ? [source] : sources;
      const groundedVision = paper.attachments.some(attachment => isVisionByLogicAttachment(attachment) && evidence.some(source => source.attachmentId === attachment.id));
      const payload = decodeAiCandidatePayload({ kind: "paper_overview_unit_v1", schemaVersion: 1, ...definition,
        fields: groundedVision ? visionByLogicReadingFields(fieldIds, evidence) : fieldIds.map((field, index) => ({ field, status: index === 0 ? "partial" : "not_found",
          statements: index === 0 ? [{ id: "mock_statement", text: `模拟候选：第 1 页标题为“${source.quoteExact}”。真实正文未进行分析，其他字段保留未找到。`,
            epistemicRole: "source_paraphrase", evidenceItemIds: [source.citationKey], evidenceQuotes: [{ evidenceItemId: source.citationKey, quote: source.quoteExact }] }] : [] })) },
        { allowedEvidenceItemIds: new Set(evidence.map(source => source.citationKey)) });
      const now = new Date().toISOString(); const runId = crypto.randomUUID(); const id = crypto.randomUUID();
      const candidate: AiCandidate = { id, runId, providerId: "fixture-generation", schemaVersion: 1, validationStatus: "valid", originalPayloadHash: "d".repeat(64), candidateKind: "paper_overview_unit_v1", reviewState: "unreviewed", currentRevision: 1,
        modelId: definition.paperTemplate === undefined ? "mock-profile-extraction" : "mock-template-extraction", createdAt: now, updatedAt: now,
        evidenceBundle: { id: crypto.randomUUID(), runId, scopeHash: contextSnapshotId, retrievalConfigVersion: "mock-template-v1", validationStatus: "valid", createdAt: now, items: evidence },
        revisions: [{ id: crypto.randomUUID(), revision: 1, payload, payloadHash: "d".repeat(64), editorKind: "ai", sourceRunId: runId, changeReason: "mock_template_extraction", createdAt: now }],
        decisions: [], attentionFeedback: [], suggestionItemDecisions: [], citationAssessments: [], overviewFieldConfirmations: [], inspectedEvidenceItemIds: [] };
      store.add(candidate);
      const workflow: PaperAnalysisWorkflow = { id: crypto.randomUUID(), paperId: draft.paperId, contextSnapshotId, status: "needs_review",
        providerId: "fixture-generation", modelId: candidate.modelId, promptSetVersion: "mock-template-v1", revision: 1, createdAt: now, updatedAt: now, completedAt: now,
        turns: [{ id: crypto.randomUUID(), workflowId: "mock", ordinal: 0, kind: "paper_overview", status: "succeeded", candidateId: id, createdAt: now, finishedAt: now, attempts: [] }] };
      workflows.unshift(workflow); persist(); return workflow;
    },
    confirmOverviewFields: async draft => {
      const repeated = batches.find(value => value.idempotencyKey === draft.idempotencyKey);
      if (repeated !== undefined) return { candidate: find(repeated.candidateId), batch: repeated };
      const candidate = find(draft.candidateId);
      if (candidate.currentRevision !== draft.candidateRevision || ["accepted_unchanged", "accepted_after_edit", "rejected"].includes(candidate.reviewState)) throw new Error("候选版本冲突");
      const payload = candidate.revisions.find(value => value.revision === draft.candidateRevision)?.payload;
      if (payload?.kind !== "paper_overview_unit_v1") throw new Error("候选结构错误");
      const template = payload.paperTemplate; const paperId = candidate.evidenceBundle.items[0]?.paperId;
      if (paperId === undefined || draft.fieldKinds.length === 0 || new Set(draft.fieldKinds).size !== draft.fieldKinds.length) throw new Error("确认字段无效");
      const fields = draft.fieldKinds.map(id => {
        const field = payload.fields.find(value => value.field === id);
        if (field === undefined || field.statements.length === 0 || candidate.overviewFieldConfirmations.some(value => value.active && value.fieldKind === id)) throw new Error("字段无内容或已确认");
        return field;
      });
      const now = new Date().toISOString(); const batchId = crypto.randomUUID();
      const units = browserKnowledgeStorage.readReadingUnits();
      const decisionFor = (fieldId: string): "human_edited" | "ai_unchanged" => {
        let previous = ""; let decision: "human_edited" | "ai_unchanged" = "ai_unchanged";
        for (const revision of [...candidate.revisions].sort((a, b) => a.revision - b.revision)) {
          const signature = JSON.stringify(revision.payload.kind === "paper_overview_unit_v1"
            ? revision.payload.fields.find(value => value.field === fieldId) : undefined);
          if (signature !== previous) { decision = revision.editorKind === "human" ? "human_edited" : "ai_unchanged"; previous = signature; }
        }
        return decision;
      };
      const previousFor = (fieldId: string): StructuredReadingUnit | undefined => units.find(unit => unit.paperId === paperId
        && (template === undefined ? unit.templateField === undefined && unit.unitKind === fieldId
          : "portraitCandidateId" in template && unit.templateField?.portraitCandidateId === template.portraitCandidateId && unit.templateField.fieldId === fieldId));
      priorUnits[batchId] = fields.flatMap(field => { const previous = previousFor(field.field); return previous === undefined ? [] : [previous]; });
      const created: StructuredReadingUnit[] = fields.map(field => ({ id: previousFor(field.field)?.id ?? crypto.randomUUID(), paperId, unitKind: template === undefined ? STRUCTURED_READING_UNIT_KINDS.find(kind => kind === field.field) ?? "other" : "other",
        title: `结构化阅读 · ${overviewFieldTitle(payload, field.field)}`,
        body: field.statements.map(value => value.text).join("\n\n"), epistemicRole: "source_paraphrase" as const,
        provenanceKind: decisionFor(field.field) === "human_edited" ? "ai_accepted_after_edit" as const : "ai_accepted_unchanged" as const,
        revision: (previousFor(field.field)?.revision ?? 0) + 1, evidenceCount: new Set(field.statements.flatMap(value => value.evidenceItemIds)).size,
        ...(template === undefined || !("portraitCandidateId" in template) ? {} : { templateField: { portraitCandidateId: template.portraitCandidateId, fieldId: field.field } }), createdAt: previousFor(field.field)?.createdAt ?? now, updatedAt: now }));
      const confirmations = created.map((unit, index) => ({ id: crypto.randomUUID(), batchId, candidateId: candidate.id, candidateRevision: draft.candidateRevision,
        fieldKind: fields[index]!.field, fieldPayloadHash: "e".repeat(64), decisionKind: decisionFor(fields[index]!.field), targetReadingUnitId: unit.id,
        targetAppliedRevision: unit.revision, active: true, createdAt: now }));
      const batch: AiOverviewConfirmationBatch = { id: batchId, ...draft, paperId, fieldConfirmations: confirmations, active: true, createdAt: now };
      browserKnowledgeStorage.writeReadingUnits([...created, ...units.filter(unit => !created.some(value => value.id === unit.id))]);
      batches.push(batch); persist();
      const complete = payload.fields.filter(field => field.statements.length > 0).every(field => [...candidate.overviewFieldConfirmations, ...confirmations].some(value => value.active && value.fieldKind === field.field));
      const hasHuman = [...candidate.overviewFieldConfirmations, ...confirmations].some(value => value.active && value.decisionKind === "human_edited");
      return { candidate: store.update({ ...candidate, reviewState: complete ? (hasHuman ? "accepted_after_edit" : "accepted_unchanged") : "partially_accepted",
        overviewFieldConfirmations: [...candidate.overviewFieldConfirmations, ...confirmations] }), batch };
    },
    undoOverviewConfirmation: async draft => {
      const index = batches.findIndex(value => value.id === draft.batchId); const batch = batches[index];
      if (batch === undefined) throw new Error("确认批次不存在");
      const candidate = find(batch.candidateId);
      if (draft.fieldKind !== undefined) {
        const field = batch.fieldConfirmations.find(field => field.fieldKind === draft.fieldKind);
        if (field === undefined) throw new Error("该批次没有此字段的审核记录");
        if (reviewResets.some(reset => reset.key === draft.idempotencyKey && reset.confirmationId !== field.id)) throw new Error("恢复幂等键已用于其他审核操作");
        if (!field.active) { if (field.reviewResetAt === undefined) throw new Error("该审核记录已撤销"); return { candidate, batch }; }
        const reviewResetAt = new Date().toISOString();
        const reset = { ...field, active: false, undoneAt: reviewResetAt, reviewResetAt };
        const confirmations = candidate.overviewFieldConfirmations.map(field => field.id === reset.id ? reset : field);
        const fields = batch.fieldConfirmations.map(field => field.id === reset.id ? reset : field);
        batches[index] = { ...batch, fieldConfirmations: fields, active: fields.some(field => field.active) };
        reviewResets.push({ key: draft.idempotencyKey, confirmationId: field.id }); persist();
        return { candidate: store.update({ ...candidate, overviewFieldConfirmations: confirmations,
          reviewState: confirmations.some(field => field.active) ? "edited" : candidate.revisions.some(revision => revision.editorKind === "human") ? "edited" : "unreviewed" }), batch: batches[index]! };
      }
      if (!batch.active) return { candidate, batch };
      const units = browserKnowledgeStorage.readReadingUnits();
      if (batch.fieldConfirmations.some(value => units.find(unit => unit.id === value.targetReadingUnitId)?.revision !== value.targetAppliedRevision)) throw new Error("正式内容已被编辑，不能覆盖后续修改");
      const undoneAt = new Date().toISOString();
      browserKnowledgeStorage.writeReadingUnits([...units.filter(unit => !batch.fieldConfirmations.some(value => value.targetReadingUnitId === unit.id)),
        ...(priorUnits[batch.id] ?? []).map(unit => ({ ...unit, revision: (units.find(value => value.id === unit.id)?.revision ?? unit.revision) + 1, updatedAt: undoneAt }))]);
      batches[index] = { ...batch, active: false, undoneAt };
      persist();
      const confirmations = candidate.overviewFieldConfirmations.map(value => value.batchId === batch.id ? { ...value, active: false, undoneAt } : value);
      return { candidate: store.update({ ...candidate, reviewState: confirmations.some(value => value.active) ? "partially_accepted" : "unreviewed", overviewFieldConfirmations: confirmations }), batch: batches[index]! };
    },
  };
}
