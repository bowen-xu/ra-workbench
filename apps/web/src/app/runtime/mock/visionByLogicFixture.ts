import { decodePaperPortrait, type AiCandidate, type AiTaskPreview, type AiEvidenceItem, type PaperOverviewFieldV1 } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import data from "./visionByLogicFixtureData.json";

type Attachment = LiteratureItem["attachments"][number];
export function isVisionByLogicAttachment(attachment: Attachment): boolean {
  return attachment.displayName === "vision-by-logic.pdf" && attachment.contentSha256 === data.sha256;
}

export function visionByLogicEvidence(paper: LiteratureItem, attachment: Attachment): AiTaskPreview["evidencePreview"] {
  return data.evidence.map(source => ({ ...source, anchorId: `mock-vision:${attachment.id}:${source.citationKey}`,
    paperTitle: paper.title, attachmentId: attachment.id, contentSha256: attachment.contentSha256,
    contextText: source.quoteExact, relationKind: "direct_quote" as const }));
}

export function visionByLogicPortrait(paper: LiteratureItem, attachment: Attachment, runId = crypto.randomUUID()): AiCandidate {
  const now = new Date().toISOString();
  const payload = decodePaperPortrait(data.portrait, { allowedEvidenceItemIds: new Set(data.evidence.map(source => source.citationKey)) });
  return { id: crypto.randomUUID(), runId, providerId: "fixture-generation", modelId: "原文整理样例 · 非实时生成",
    candidateKind: "paper_portrait_v1", schemaVersion: 1, currentRevision: 1, reviewState: "unreviewed", validationStatus: "valid",
    originalPayloadHash: data.sha256, createdAt: now, updatedAt: now,
    evidenceBundle: { id: crypto.randomUUID(), runId, scopeHash: data.sha256, retrievalConfigVersion: "vision-source-fixture-v1", validationStatus: "valid", createdAt: now,
      items: visionByLogicEvidence(paper, attachment).map((source, ordinal) => ({ ...source, id: crypto.randomUUID(), ordinal,
        paperId: paper.id, quotePrefix: data.evidence[ordinal]!.quotePrefix, quoteSuffix: data.evidence[ordinal]!.quoteSuffix,
        currentAnchorStatus: "page_level", validationStatus: "valid", createdAt: now })) },
    revisions: [{ id: crypto.randomUUID(), revision: 1, payload, payloadHash: data.sha256, editorKind: "ai", sourceRunId: runId,
      changeReason: "source_grounded_static_mock", createdAt: now }],
    decisions: [], attentionFeedback: [], suggestionItemDecisions: [], citationAssessments: [], overviewFieldConfirmations: [], inspectedEvidenceItemIds: [] };
}

/** Only known stable fields receive source-derived text; user-added fields stay empty. */
export function visionByLogicReadingFields(fieldIds: readonly string[], sources?: readonly AiEvidenceItem[]): readonly PaperOverviewFieldV1[] {
  const aliases: Readonly<Record<string, string>> = { method: "recognition_learning", result: "findings", conclusion: "conclusion", limitation: "limitations", open_question: "open_questions" };
  const records: Readonly<Record<string, { text: string; evidenceItemIds: readonly string[] }>> = data.reading;
  return fieldIds.map(field => {
    const record = records[field] ?? records[aliases[field] ?? ""];
    const quotes = record?.evidenceItemIds.map(key => {
      const original = data.evidence.find(source => source.citationKey === key)!;
      const frozen = sources?.find(source => source.contentSha256 === data.sha256 && source.quoteExact.includes(original.quoteExact));
      return sources !== undefined && frozen === undefined ? undefined : { evidenceItemId: frozen?.citationKey ?? key, quote: original.quoteExact };
    });
    const available = record !== undefined && quotes !== undefined && quotes.every(quote => quote !== undefined);
    return { field, status: available ? "filled" : "not_found", statements: !available ? [] : [{
      id: `vision_${field}`, text: record.text, epistemicRole: "source_paraphrase", evidenceItemIds: quotes.map(quote => quote.evidenceItemId), evidenceQuotes: quotes,
    }] };
  });
}
