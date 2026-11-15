import {decodeNoteDocument} from "@research-assistant/core/notes";
import type {NoteDraftCommit, NoteDraftReceipt} from "@research-assistant/data/notes";
import {commitHbsDocumentRequest} from "../../../services/hbsDocumentGateway";
import {getAppRuntime} from "../../../runtime/appRuntime";
import {invoke} from "../../../runtime/platformCore";
import {publishBrowserNoteDraft} from "../gateways/browserNoteDraft";

function decodeReply(reply:unknown,id:string,requestId:string):NoteDraftReceipt {
  if (!reply || typeof reply !== "object") throw new Error("MODEL_INVALID_RECEIPT：缺少草稿保存回执");
  const value = reply as NoteDraftReceipt;
  if (value.type !== "receipt" || value.requestId !== requestId || value.page?.page.id !== id
    || !Number.isSafeInteger(value.snapshot?.version) || value.page.page.revision !== value.snapshot.version
    || decodeNoteDocument(value.page.document).pageId !== id || decodeNoteDocument(value.snapshot.saved).pageId !== id
    || decodeNoteDocument(value.snapshot.baseline).pageId !== id) throw new Error("MODEL_INVALID_RECEIPT：草稿保存回执身份不匹配");
  return value;
}
export async function publishNoteDraft(id: string, request: NoteDraftCommit, onReceipt?: (receipt:NoteDraftReceipt)=>Promise<void>): Promise<NoteDraftReceipt> {
  const reply = await commitHbsDocumentRequest("note",id,request.commit,async () => {
    try {
      const result=getAppRuntime().mode === "browser" ? await publishBrowserNoteDraft(id,request)
        : await invoke("publish_note_draft",{resourceId:id,request});
      const receipt=decodeReply(result,id,request.commit.id);
      // Acknowledge creation in the recovery slot before the immutable outbox
      // is cleared. A crash at either side can still replay the exact receipt.
      await onReceipt?.(receipt);return receipt;
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : String(reason);
      const code = reason && typeof reason === "object" && "code" in reason ? String(reason.code) : /^([A-Z_]+|revision_conflict):?[：:]/u.exec(message)?.[1];
      throw Object.assign(new Error(`${message}（请求 ${request.commit.id}）`),code ? {code,requestId:request.commit.id} : {requestId:request.commit.id});
    }
  });
  return decodeReply(reply,id,request.commit.id);
}
