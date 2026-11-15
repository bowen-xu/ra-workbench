import { invoke } from "../../runtime/platformCore";
import { runtimeAgentGateway } from "../../runtime/appRuntime";
import { deliverTextReceipt, type AppliedNotePage } from "../notes/editor/noteTextEditorPorts";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
export interface TextPatchBlockChange {
  readonly id: string; readonly beforeBlockIds: readonly string[]; readonly afterBlockIds: readonly string[];
  readonly beforeIndex: number; readonly afterIndex: number;
}
export interface TextPatchPageReview {
  readonly pageId: string; readonly path: string; readonly title: string; readonly kind: "create" | "trash" | "move" | "update";
  readonly before: string | null; readonly after: string | null; readonly expectedRevision: number | null;
  readonly beforeDocument?: NoteDocumentV1 | null; readonly afterDocument?: NoteDocumentV1 | null;
  readonly reviewDocument?: NoteDocumentV1 | null; readonly chunks?: readonly TextPatchBlockChange[];
  readonly metadataPending?: boolean;
  readonly structuralMetadataPending?: boolean;
  readonly recovery?: { readonly currentRevision: number; readonly status: string };
}
export interface TextPatchReview {
  readonly persistenceVersion?: number;
  readonly proposalId: string; readonly generation: number; readonly pages: readonly TextPatchPageReview[];
  readonly baselineMatches: boolean; readonly state: "pending" | "accept" | "reject" | "undo";
  readonly operationGeneration?: number; readonly wholeUndoAvailable?: boolean;
}
export interface TextPatchDecision {
  readonly conversationId: string; readonly proposalId: string; readonly generation: number;
  readonly requestId: string; readonly action: "accept" | "reject" | "undo";
}
export interface TextPatchGateway {
  forPage(pageId: string): Promise<readonly string[]>;
  review(conversationId: string, proposalId?: string, generation?: number, pageId?: string): Promise<TextPatchReview | null>;
  decide(request: TextPatchDecision): Promise<{ readonly requestId: string; readonly proposalId: string; readonly action: string }>;
  resolve(request: TextPatchResolution): Promise<TextPatchResolutionReceipt>;
}
/** Private editor protocol. Model text tools never receive these documents. */
export interface TextPatchResolution {
  readonly conversationId: string; readonly proposalId: string; readonly generation: number; readonly requestId: string;
  readonly pageId: string; readonly action: "keep" | "undo" | "edit" | "restore" | "retain-current";
  readonly chunkId?: string; readonly document?: NoteDocumentV1;
  readonly historyRequestId?: string; readonly direction?: "undo" | "redo";
  readonly expectedRevision?: number;
}
export interface TextPatchResolutionReceipt {
  readonly requestId: string; readonly proposalId: string; readonly action: string; readonly generation: number;
  readonly review: TextPatchReview | null; readonly pages: readonly AppliedNotePage[];
}
export const textPatchGateway = runtimeAgentGateway<TextPatchGateway>("textPatchGateway", {
  forPage: pageId => invoke("agent_note_text_patch_conversations", { pageId }),
  review: (conversationId, proposalId, generation, pageId) => invoke("agent_read_text_patch_proposal", { conversationId, proposalId: proposalId ?? null, generation: generation ?? null, pageId:pageId??null }),
  decide: async request => {
    const receipt = await invoke<{ readonly requestId: string; readonly proposalId: string; readonly action: string; readonly pages: readonly AppliedNotePage[] }>("agent_decide_text_patch", { request });
    if (receipt.requestId !== request.requestId || receipt.proposalId !== request.proposalId || receipt.action !== request.action) throw new Error("原生修改回执身份不匹配");
    await deliverTextReceipt(receipt.pages); return receipt;
  },
  resolve: async request => {
    const receipt = await invoke<TextPatchResolutionReceipt>("agent_resolve_note_text_patch", { request });
    if (receipt.requestId !== request.requestId || receipt.proposalId !== request.proposalId || receipt.action !== request.action)
      throw new Error("笔记审阅回执身份不匹配，请恢复原操作。");
    if (receipt.pages.length) await deliverTextReceipt(receipt.pages.map(page => {
      const baseline = receipt.review?.pages.find(item => item.pageId === page.pageId)?.reviewDocument;
      return baseline ? { ...page, review: { baseline, origin: request.action === "edit" ? "manual" as const
        : request.action === "restore" ? "restore" as const : "review" as const,
        ...(request.action === "keep" || request.action === "undo" ? { requestId: request.requestId } : {}),
        addToHistory: request.action === "keep" || request.action === "undo" } } : page;
    }));
    return receipt;
  },
});
const observers = new Set<() => void>();
export function subscribeTextPatches(observer: () => void): () => void { observers.add(observer); return () => observers.delete(observer); }
export function notifyTextPatchesChanged(): void { observers.forEach(observer => observer()); }
