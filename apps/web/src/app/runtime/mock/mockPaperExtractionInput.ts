import { getStructuredResourceGateway } from "../../../core/services/structuredResourceGateway";
import { decodePaperTemplateSnapshot } from "@research-assistant/core/ai";
import { PROFILED_OVERVIEW_FIELDS, selectPaperPortraitTemplate, type AiCandidate, type AiEvidenceItem, type PaperAnalysisStartDraft,
  type PaperOverviewUnitCandidateV1 } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { browserReadingProfiles, getBrowserPaperProfileSelection } from "../../../../../../plugins/literature_library/src/services/browserReadingProfiles";

export interface MockPaperExtractionInput {
  readonly definition: Pick<PaperOverviewUnitCandidateV1, "profileId" | "profileRefs" | "paperTemplate">;
  readonly fieldIds: readonly string[];
  readonly source: AiEvidenceItem;
  readonly sources: readonly AiEvidenceItem[];
}

/** Resolve the same explicit template/profile choice as production, without a provider. */
export async function mockPaperExtractionInput(draft: PaperAnalysisStartDraft, paper: LiteratureItem,
  candidates: readonly AiCandidate[]): Promise<MockPaperExtractionInput> {
  const reference = draft.paperTemplateRef;
  if (reference !== undefined && "templateResourceId" in reference) {
    const template=await getStructuredResourceGateway().getVersion(reference.templateResourceId,reference.templateResourceRevision);
    const snapshot=decodePaperTemplateSnapshot({...reference,fields:(template.content as {fields:unknown}).fields});
    const portrait=selectPaperPortraitTemplate(candidates,draft.paperId);
    const sources=portrait?.candidate.evidenceBundle.items ?? [];
    const attachment=paper.attachments.find(value=>draft.attachmentIds.includes(value.id));
    if(!attachment)throw new Error("请至少选择一个来源 PDF");
    const quote=paper.title.split(/[ :]/u)[0]??paper.title;
    const source=sources[0]??{id:crypto.randomUUID(),citationKey:"E1",ordinal:0,anchorId:`mock-template:${attachment.id}`,paperId:paper.id,paperTitle:paper.title,attachmentId:attachment.id,contentSha256:attachment.contentSha256,pageIndex:0,quoteExact:quote,quotePrefix:"",quoteSuffix:"",contextText:quote,relationKind:"direct_quote" as const,currentAnchorStatus:"page_level" as const,validationStatus:"valid" as const,createdAt:new Date().toISOString()};
    return {definition:{profileId:"paper_template_v1",paperTemplate:snapshot},fieldIds:snapshot.fields.map(field=>field.id),source,sources};
  }
  if (reference !== undefined && "portraitCandidateId" in reference) {
    const accepted = selectPaperPortraitTemplate(candidates, draft.paperId);
    if (accepted?.candidate.id !== reference.portraitCandidateId || accepted.revision !== reference.portraitRevision) throw new Error("本文模板不可用或已更新");
    const portrait = accepted.candidate.revisions.find(value => value.revision === reference.portraitRevision)?.payload;
    if (portrait?.kind !== "paper_portrait_v1") throw new Error("画像结构错误");
    const source = accepted.candidate.evidenceBundle.items[0];
    if (source === undefined || accepted.candidate.evidenceBundle.items.some(source => !draft.attachmentIds.includes(source.attachmentId))) throw new Error("模拟提取需包含画像来源 PDF");
    if (accepted.candidate.evidenceBundle.items.some(source => paper.attachments.find(value => value.id === source.attachmentId)?.contentSha256 !== source.contentSha256)) throw new Error("画像来源 PDF 已变化，请重新生成画像");
    return { definition: { profileId: "paper_template_v1", paperTemplate: { ...reference, fields: portrait.template } },
      fieldIds: portrait.template.map(field => field.id), source, sources: accepted.candidate.evidenceBundle.items };
  }
  const selection = getBrowserPaperProfileSelection(draft.paperId);
  if (!selection.isPersisted || selection.profiles.length === 0) throw new Error("请先生成文献画像与本文模板，或选择阅读框架");
  if (selection.revision !== draft.overviewProfileSelectionRevision) throw new Error("阅读框架已变化，请重新提取");
  const profiles = selection.profiles.map(reference => {
    const profile = browserReadingProfiles.find(value => value.profileId === reference.profileId && value.profileVersion === reference.profileVersion && value.reviewStatus === "approved");
    if (profile === undefined) throw new Error("阅读框架版本未审核或不可用");
    return profile;
  });
  const attachment = paper.attachments.find(value => value.id === draft.attachmentIds[0]);
  if (attachment === undefined) throw new Error("请至少选择一个来源 PDF");
  const quote = attachment.displayName === "sample.pdf" ? "Synthetic Research Preview" : paper.title.split(/[ :]/u)[0] ?? paper.title;
  return { definition: { profileId: "reviewed_profile_composition_v1", profileRefs: selection.profiles },
    sources: [], fieldIds: PROFILED_OVERVIEW_FIELDS.filter(field => profiles.some(profile => profile.fieldKinds.includes(field))),
    source: { id: crypto.randomUUID(), citationKey: "E1", ordinal: 0, anchorId: `mock-profile:${attachment.id}`, paperId: paper.id,
      paperTitle: paper.title, attachmentId: attachment.id, contentSha256: attachment.contentSha256, pageIndex: 0,
      quoteExact: quote, quotePrefix: "", quoteSuffix: "", contextText: quote, relationKind: "direct_quote",
      currentAnchorStatus: "page_level", validationStatus: "valid", createdAt: new Date().toISOString() } };
}
