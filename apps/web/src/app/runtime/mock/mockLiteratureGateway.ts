import type { LiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { emptyLiteratureDraft, type AttachmentSummary, type ImportJob, type LiteratureDraft, type LiteratureItem, type MetadataPreview, type PageText } from "@research-assistant/core/literature";
import { readSamplePdf } from "./mockWritingGateway";
import { writingDiffDemoHash } from "../../previews/agent/WritingDiffDemoStorage";
import { previewLiteratureFixtures } from "./previewLiteratureFixtures";

export function createMockLiteratureGateway(base: LiteratureGateway, storage: Storage): LiteratureGateway {
  const jobs = new Map<string, ImportJob>(JSON.parse(storage.getItem("import-jobs") ?? "[]") as [string, ImportJob][]);
  const previews = new Map<string, MetadataPreview>();
  const indexKey = "preview-attachment-text-indexes";
  const indexes = new Map<string, {hash: string; pages: readonly PageText[]}>(JSON.parse(storage.getItem(indexKey) ?? "[]"));
  const persistJob = (job: ImportJob) => { jobs.set(job.id, job); storage.setItem("import-jobs", JSON.stringify([...jobs])); return job; };
  const pendingPreview = (id: string): MetadataPreview => {
    const preview = previews.get(id);
    if (!preview) throw new Error("模拟元数据预览已过期");
    return preview;
  };
  const finishPreview = (preview: MetadataPreview, item?: LiteratureItem): void => {
    const job = jobs.get(preview.importJobId);
    if (!job) throw new Error("模拟导入记录不存在");
    persistJob({ ...job, state: item ? "completed" : "cancelled", completedCount: item ? 1 : 0,
      cancellationRequested: !item, updatedAt: new Date().toISOString(),
      results: job.results.map(result => item ? { ...result, state: "imported", itemId: item.id } : { ...result, state: "cancelled" }) });
    previews.delete(preview.id);
  };
  const applyPreview = async (id: string, draft: LiteratureDraft): Promise<LiteratureItem> => {
    const preview = pendingPreview(id);
    const { id: _id, ...fields } = draft;
    const item = await base.save({ ...fields, ...(preview.itemId ? { id: preview.itemId } : {}), sourceKind: "mock" });
    finishPreview(preview, item);
    return item;
  };
  return {
    ...base,
    readAttachment: async id => {
      if (!(await base.list()).some(item => item.attachments.some(attachment => attachment.id === id))) throw new Error("模拟附件不存在");
      const fixture = previewLiteratureFixtures.find(value => value.item.attachments.some(attachment => attachment.id === id));
      if (!fixture) return readSamplePdf();
      const response = await fetch(fixture.pdfUrl);
      if (!response.ok) throw new Error("预览文献 PDF 无法加载");
      return new Uint8Array(await response.arrayBuffer());
    },
    revealAttachments: async ids => { for (const id of ids) if (!(await base.list()).some(item => item.attachments.some(attachment => attachment.id === id))) throw new Error("模拟附件不存在"); },
    attachPdf: async (itemId) => {
      const item = await base.get(itemId), bytes = await readSamplePdf();
      const attachment: AttachmentSummary = { id: crypto.randomUUID(), displayName: "sample.pdf", mimeType: "application/pdf", byteSize: bytes.length, contentSha256: await writingDiffDemoHash(bytes) };
      if (item.attachments.some(value => value.contentSha256 === attachment.contentSha256)) throw new Error("attachment_duplicate：此文献已包含内容相同的 PDF，不能重复添加");
      const items: readonly LiteratureItem[] = (await base.list()).map(value => value.id === itemId ? { ...item, attachments: [...item.attachments, attachment] } : value);
      storage.setItem("research-assistant.literature.v1", JSON.stringify(items)); return attachment;
    },
    importPdfs: async candidates => {
      const timestamp = new Date().toISOString(), results: ImportJob["results"][number][] = [];
      for (const candidate of candidates) {
        const item = await base.save({ itemType: "journalArticle", title: "模拟导入文献", abstractText: "合成数据", containerTitle: "", volume: "", issue: "", pages: "", doi: "", url: "", language: "en", bibtexKey: "", creators: [], tags: [] });
        await createMockLiteratureGateway(base, storage).attachPdf(item.id, "mock://sample.pdf");
        results.push({ inputLabel: candidate.path, rawInput: candidate.path, state: "imported", itemId: item.id });
      }
      return persistJob({ id: crypto.randomUUID(), state: "completed", sourceKind: "pdf", totalCount: results.length, completedCount: results.length, cancellationRequested: false, results, createdAt: timestamp, updatedAt: timestamp });
    },
    importBibtex: async drafts => persistJob(await base.importBibtex(drafts)),
    getImportJob: async id => { const job = jobs.get(id); if (!job) throw new Error("模拟导入记录不存在"); return job; },
    listImportJobs: async () => [...jobs.values()], cancelImportJob: async id => { const job = jobs.get(id); if (!job) throw new Error("模拟导入记录不存在"); persistJob({ ...job, cancellationRequested: true }); },
    previewDoi: async (doi, itemId) => {
      const current = itemId ? await base.get(itemId) : emptyLiteratureDraft();
      const value: MetadataPreview = { id: crypto.randomUUID(), importJobId: crypto.randomUUID(), ...(itemId ? { itemId } : {}), sourceKind: "mock", sourceReference: doi, draft: { ...current, doi, title: "模拟 DOI 元数据" }, diffs: [{ fieldName: "title", currentValue: current.title, candidateValue: "模拟 DOI 元数据", userConfirmed: false }] };
      const timestamp = new Date().toISOString();
      persistJob({ id: value.importJobId, sourceKind: "doi", state: "queued", totalCount: 1, completedCount: 0,
        cancellationRequested: false, results: [{ inputLabel: doi, rawInput: doi, state: "pending" }], createdAt: timestamp, updatedAt: timestamp });
      previews.set(value.id, value); return value;
    },
    applyMetadataCandidate: applyPreview,
    rejectMetadataCandidate: async id => { finishPreview(pendingPreview(id)); },
    applyMetadataCandidateFields: async (id, decisions) => { const preview = pendingPreview(id); const item = preview.itemId ? await base.get(preview.itemId) : preview.draft; return applyPreview(id, { ...item, ...(decisions.some(choice => choice.fieldName === "title" && choice.choice === "candidate") ? { title: preview.draft.title } : {}) }); },
    isAttachmentIndexed: async id => {
      const attachment = (await base.list()).flatMap(item => item.attachments).find(value => value.id === id);
      return !!attachment && indexes.get(id)?.hash === attachment.contentSha256;
    },
    indexAttachment: async (itemId, attachmentId, pages) => {
      const attachment = (await base.get(itemId)).attachments.find(value => value.id === attachmentId);
      if (!attachment) throw new Error("模拟附件不存在");
      indexes.set(attachmentId, {hash: attachment.contentSha256, pages: [...pages]});
      storage.setItem(indexKey, JSON.stringify([...indexes]));
    },
  };
}
