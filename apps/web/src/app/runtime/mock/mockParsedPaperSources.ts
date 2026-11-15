import type { PaperAiReadingAttachmentStatus, PaperAiReadingContextItem, PaperAiReadingPrepareResult } from "@research-assistant/core/ai";
import type { DoclingParseRunDetail } from "../../../core/services/doclingGateway";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";

export const MOCK_DOCLING_MODEL_IDENTITY = "preview-models";
export function mockDoclingIdentity(storage: Storage): string {
  return storage.getItem("docling-model-identity") ?? MOCK_DOCLING_MODEL_IDENTITY;
}
export function savedMockDoclingRuns(storage: Storage): readonly DoclingParseRunDetail[] {
  return (JSON.parse(storage.getItem("docling-runs") ?? "[]") as [string, DoclingParseRunDetail][]).map(([,run]) => run);
}
export function currentMockDoclingRun(storage: Storage, paper: string, file: string, sha: string): DoclingParseRunDetail | undefined {
  return [...savedMockDoclingRuns(storage)].reverse().find(run => run.itemId === paper && run.attachmentId === file
    && run.contentSha256 === sha && ["success", "partial_success"].includes(run.parseStatus) && run.jsonContent != null
    && typeof run.rawResponse === "object" && run.rawResponse !== null
    && (run.rawResponse as {runtime?: {identity?: string}}).runtime?.identity === mockDoclingIdentity(storage));
}

/** Mock SQLite reads only: availability derives from saved conversion results. */
export async function readMockPaperContext(storage: Storage, paperId: string, files: readonly string[]): Promise<PaperAiReadingPrepareResult> {
  const paper = await getLiteratureGateway().get(paperId);
  const contextItems: PaperAiReadingContextItem[] = [];
  const attachmentStatuses: PaperAiReadingAttachmentStatus[] = files.map(id => {
    const attachment = paper.attachments.find(value => value.id === id);
    if (!attachment) throw new Error("模拟附件不存在");
    const run = currentMockDoclingRun(storage, paperId, id, attachment.contentSha256);
    if (run) {
      const document = run.jsonContent as {texts?: readonly {self_ref?: string; text?: string; prov?: readonly {page_no?: number}[]}[]};
      for (const [index, text] of (document.texts ?? []).entries()) contextItems.push({
        id: `${run.id}:T${index + 1}`, paperId, attachmentId: id, contentSha256: attachment.contentSha256,
        parseRunId: run.id, rawRef: text.self_ref ?? `#/texts/${index}`, kind: "text", regions: [],
        pageIndex: (text.prov?.[0]?.page_no ?? 1) - 1, text: text.text ?? "",
      });
    }
    return {attachmentId: id, displayName: attachment.displayName, contentSha256: attachment.contentSha256,
      ...(run ? {parseRunId: run.id} : {}), parseStatus: run ? "available"
        : savedMockDoclingRuns(storage).some(value => value.itemId === paperId && value.attachmentId === id) ? "stale" : "missing"};
  });
  return {attachmentStatuses, contextItems, imageRequests: [], textItemCount: contextItems.length, tableItemCount: 0, pictureItemCount: 0};
}
