import { decodeNoteDocument, type NoteDocumentV1 } from "@research-assistant/core/notes";
import type { TextPatchResolution } from "../../agent/textPatchGateway";

/** Recovery preserves D/B and the exact uncertain request; it authorizes no native write. */
export interface NoteReviewRecovery {
  readonly document: NoteDocumentV1;
  readonly baseline: NoteDocumentV1;
  readonly proposalId?: string;
  readonly conversationId?: string;
  readonly request?: TextPatchResolution;
  readonly expectedDocument?: NoteDocumentV1;
}
export function decodeNoteReviewRecovery(value: unknown, pageId: string): NoteReviewRecovery {
  if (!value || typeof value !== "object" || !("document" in value) || !("baseline" in value)) throw new Error("审阅恢复草稿格式无效，原记录已保留。");
  const document = decodeNoteDocument(value.document), baseline = decodeNoteDocument(value.baseline);
  if (document.pageId !== pageId || baseline.pageId !== pageId) throw new Error("审阅恢复草稿身份不匹配。");
  const result: NoteReviewRecovery = { document, baseline };
  if ("request" in value && value.request !== undefined) {
    const request = value.request;
    if (!request || typeof request !== "object" || !("pageId" in request) || request.pageId !== pageId
      || !("requestId" in request) || typeof request.requestId !== "string" || !("conversationId" in request) || typeof request.conversationId !== "string"
      || !("proposalId" in request) || typeof request.proposalId !== "string" || !("generation" in request) || (!Number.isSafeInteger(request.generation) || Number(request.generation) < 0)
      || !("action" in request) || !["edit", "restore", "keep", "undo", "retain-current"].includes(String(request.action))) throw new Error("审阅恢复请求无效，原记录已保留。");
    const decoded = request as unknown as TextPatchResolution;
    const requestDocument = decoded.document && decodeNoteDocument(decoded.document);
    const expectedDocument = "expectedDocument" in value && value.expectedDocument ? decodeNoteDocument(value.expectedDocument) : undefined;
    if (requestDocument && requestDocument.pageId !== pageId || expectedDocument && expectedDocument.pageId !== pageId
      || decoded.action === "edit" && !requestDocument || decoded.action === "restore" && (!decoded.historyRequestId || !["undo", "redo"].includes(decoded.direction ?? "")))
      throw new Error("审阅恢复请求的正文或历史身份无效，原记录已保留。");
    if (decoded.action === "retain-current" && (!requestDocument || !Number.isSafeInteger(decoded.expectedRevision) || Number(decoded.expectedRevision) < 0))
      throw new Error("保留当前内容的恢复请求缺少版本或正文，原记录已保留。");
    return { ...result, proposalId: decoded.proposalId, conversationId: decoded.conversationId,
      request: { ...decoded, ...(requestDocument ? { document: requestDocument } : {}) },
      ...(expectedDocument ? { expectedDocument } : {}) };
  }
  return { ...result, ...("proposalId" in value && typeof value.proposalId === "string" ? { proposalId: value.proposalId } : {}),
    ...("conversationId" in value && typeof value.conversationId === "string" ? { conversationId: value.conversationId } : {}) };
}

export function noteReviewErrorMessage(reason: unknown): string {
  let message = reason instanceof Error ? reason.message : typeof reason === "string" ? reason
    : reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string" ? reason.message
      : "笔记保存结果尚未确认，请保留当前输入并重试或比较版本。";
  if (message.trim().startsWith("{")) {
    try {
      const value: unknown = JSON.parse(message);
      if (value && typeof value === "object" && "message" in value && typeof value.message === "string") message = value.message;
    } catch { /* Non-JSON messages remain readable text. */ }
  }
  if (!/^MODEL_[A-Z_]+[:：]/u.test(message) && /saved note changed|proposal changed/.test(message)) return "笔记或审阅版本发生变化，当前输入已保留，请比较版本。";
  return message;
}
export function isNoteReviewConflict(reason: unknown): boolean {
  return /saved note changed|proposal changed|版本.*变化|已保存版本变化/.test(noteReviewErrorMessage(reason));
}
