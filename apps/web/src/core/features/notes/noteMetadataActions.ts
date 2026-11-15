import type {NotePage, NotePageId, NoteMetadataAction, NoteMetadataBatchReceipt, NoteMetadataReplayRequest} from "@research-assistant/core/notes";
import {NotesGatewayError, type NotesErrorCode} from "@research-assistant/data/notes";
import type {NoteResourceAccess} from "../../contracts/noteResources";
import {getReaderCloseCoordinator} from "../../workspace/DocumentCloseCoordinator";
import {holdTextEditor, holdNoteEditorInput, synchronizeNoteReview} from "./editor/noteTextEditorPorts";
import {notify} from "../../services/notifications";
import {createElement} from "react";
import {ArchiveRestore, Trash2} from "lucide-react";
import {NOTE_METADATA_CHANGED, NOTE_PAGES_DELETED, type NoteMetadataChanged} from "./noteResourceEvents";

/** Publish only committed metadata. The returned page set includes cascade members. */
export function publishNoteMetadataReceipt(client: NoteResourceAccess, receipt: NoteMetadataBatchReceipt, action: NoteMetadataAction): void {
  if (!receipt.pages.length) return;
  client.notifyChanged({kind: "metadata", receipt});
  window.dispatchEvent(new CustomEvent<NoteMetadataChanged>(NOTE_METADATA_CHANGED, {detail: {...receipt, action}}));
}

/** Toolbar and embedded owners use the same guarded transaction as list actions. */
export async function applyNotePageStatus(client: NoteResourceAccess, id: NotePageId, status: "active" | "archived" | "trashed"): Promise<NotePage> {
  const action = status === "trashed" ? "trash" : status === "archived" ? "archive" : "restore";
  const {receipt, pages, warnings} = await commitNoteMetadata(client, [id], action);
  if (warnings.length) notify.error(warnings.join("；"));
  const failure = receipt.failures.find(value => value.id === id);
  if (failure) throw new NotesGatewayError(failure.code as NotesErrorCode,
    `${failure.code}: ${failure.message}（操作 ${receipt.operationId}）`, failure.currentRevision);
  const page = receipt.pages.find(value => value.id === id) ?? pages.find(value => value.id === id);
  if (!page || !receipt.succeededIds.includes(id)) throw new NotesGatewayError("not_found", `找不到笔记页面（操作 ${receipt.operationId}）`);
  return page;
}

export async function commitNoteMetadata(client: NoteResourceAccess, ids: readonly NotePageId[], action: NoteMetadataAction,replay?:NoteMetadataReplayRequest): Promise<{
  readonly receipt: NoteMetadataBatchReceipt; readonly pages: readonly NotePage[]; readonly warnings: readonly string[];
}> {
  const operationId = replay?.operationId??crypto.randomUUID();
  const releaseInput = holdNoteEditorInput(operationId);
  try {
    if (!await getReaderCloseCoordinator().prepareAll()) throw new Error("请先完成保存或解决冲突，再管理笔记。");
    for (const id of ids) holdTextEditor(id, operationId, true);
    const pages = await client.gateway.listPageMetadata(ids);
    const versions = new Map(pages.map(page => [page.id, page.revision]));
    const request = {operationId, action, items: ids.map(id => ({id, expectedRevision: versions.get(id) ?? 0}))};
    const execute=()=>replay?client.gateway.replayMetadataBatch?.(replay)??Promise.reject(new NotesGatewayError("source_unavailable","note_history_unavailable: 当前存储不支持笔记历史操作。")):client.gateway.applyMetadataBatch(request);
    const receipt = await execute().catch(reason => {
      if (reason instanceof NotesGatewayError && reason.code === "operation_unknown") return execute();
      throw reason;
    });
    let warnings: readonly string[] = [];
    if (receipt.pages.length) {
      publishNoteMetadataReceipt(client, receipt, action);
      if (action === "trash" && receipt.succeededIds.length&&!replay) {
        showTrashNotice(client, receipt);
      }
      const synchronized = await Promise.allSettled(receipt.pages.map(page => synchronizeNoteReview(page.id)));
      warnings = synchronized.flatMap(result => result.status === "fulfilled" ? [] : [
        `笔记操作已提交，但打开的编辑器同步失败：${result.reason instanceof NotesGatewayError ? `${result.reason.code}: ${result.reason.message}` : result.reason instanceof Error ? result.reason.message : String(result.reason)}（操作 ${operationId}）`,
      ]);
      if(action==="trash"&&receipt.history&&!replay)recordDeletion(client,receipt);
    }
    return {receipt, pages, warnings};
  } catch (reason) {
    if (reason instanceof NotesGatewayError) throw new NotesGatewayError(reason.code, `${reason.code}: ${reason.message}（操作 ${operationId}）`, reason.currentRevision);
    throw reason;
  } finally { releaseInput(); }
}

/** Deletion is a shared note command, regardless of its list/page entrypoint.
 * Its inverse uses the original durable receipt and the same guarded tx. */
function recordDeletion(client:NoteResourceAccess,original:NoteMetadataBatchReceipt):void {
  let latest=original,pending:NoteMetadataReplayRequest|undefined;
  const replay=async(direction:"undo"|"redo")=>{
    pending??={operationId:crypto.randomUUID(),sourceOperationId:original.operationId,direction,
      items:original.history!.statuses.map(({id})=>({id,expectedRevision:latest.pages.find(page=>page.id===id)!.revision}))};
    if(pending.direction!==direction)throw new Error("note_history_pending: 请先核对尚未完成的笔记历史操作。");
    const result=await commitNoteMetadata(client,pending.items.map(item=>item.id),direction==="undo"?"restore":"trash",pending);
    if(result.receipt.failures.length)throw new Error("note_history_changed: 笔记状态已变化，请核对后重试。");
    latest=result.receipt;pending=undefined;
    if(result.warnings.length)notify.error(result.warnings.join("；"));
    notify.update(`note-trash:${original.operationId}`,{message:direction==="undo"?"已撤销删除":"已重做删除",actions:[]});
  };
  client.history.closeGroups();
  client.history.record({id:original.operationId,undo:()=>replay("undo"),redo:()=>replay("redo")});
}

/** Buttons are rendered and locked by the shared NotificationActions component. */
function showTrashNotice(client: NoteResourceAccess, receipt: NoteMetadataBatchReceipt): void {
  const id = `note-trash:${receipt.operationId}`, ids = receipt.succeededIds;
  notify.success("笔记已移到回收站", { id, actions: [
    {id: "restore", label: "恢复笔记", icon: createElement(ArchiveRestore, {"aria-hidden": true}), onClick: async () => {
      if(receipt.history){await client.history.run("undo",receipt.operationId);return;}
      const result = await commitNoteMetadata(client, ids, "restore");
      if (result.receipt.failures.length) throw new Error(result.receipt.failures.map(failure => `${failure.code}: ${failure.message}`).join("；"));
      if (result.warnings.length) notify.error(result.warnings.join("；"));
      notify.update(id, {message: "笔记已恢复", actions: []});
    }},
    {id: "delete", label: "永久删除", icon: createElement(Trash2, {"aria-hidden": true}), onClick: async () => {
      const pages = await client.gateway.listPageMetadata(ids);
      if (pages.length !== ids.length || pages.some(page => page.status !== "trashed")) throw new NotesGatewayError("revision_conflict", "笔记状态已改变，请重新核对回收站。");
      if (!window.confirm(`确定永久删除${pages.length === 1 ? `“${pages[0]!.title}”` : `这 ${pages.length} 篇笔记`}吗？\n\n正文、历史和项目引用将无法恢复；引用它们的其他笔记会保留为断链。子页面会移动到顶层。`)) return;
      for (const page of pages) {
        const report = await client.mutate(gateway => gateway.permanentlyDeletePage({id: page.id, expectedRevision: page.revision}));
        window.dispatchEvent(new CustomEvent<readonly NotePageId[]>(NOTE_PAGES_DELETED, {detail: [page.id]}));
        if (report.cleanupWarnings.length) notify.error(report.cleanupWarnings.join("；"));
      }
      notify.update(id, {message: "笔记已永久删除", actions: []});
    }},
  ] });
}
