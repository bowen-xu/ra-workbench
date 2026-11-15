import { planNoteMetadataBatch, isDependentReadingNote, type NoteMetadataBatchReceipt, type NoteMetadataBatchRequest,type NotePageId } from "@research-assistant/core/notes";
import { NotesGatewayError,type NoteDocumentSaveRequest } from "@research-assistant/data/notes";
import { now, readState, writeState, type BrowserNotesState } from "./browserNotesStorage";
import {applyPageEntryDeletion,creationEntryProofs,synchronizePageEntryBaselines} from "./browserPageEntries";
import type {NoteMetadataReplayRequest} from "@research-assistant/core/notes";
import {captureMetadataHistory,restoreMetadataEntries} from "./browserMetadataHistory";
import type {BrowserDocumentSave} from "./browserPageEntryLifecycle";

export function browserNoteMetadataBatch(synchronizeParents: (state: BrowserNotesState) => BrowserNotesState,
  save:BrowserDocumentSave) {
  // This command already owns the target statuses, scoped B changes and their
  // shared history. Its source-document writes must not replay those effects.
  const savePrepared=(state:BrowserNotesState,request:NoteDocumentSaveRequest)=>save(state,request,"prepared");
  return async (request: NoteMetadataBatchRequest,replay?:NoteMetadataReplayRequest): Promise<NoteMetadataBatchReceipt> => {
    const original=replay?(await readState({metadataOnly:true,operationId:replay.sourceOperationId})).metadataReceipts?.[replay.sourceOperationId]?.receipt.history:undefined;
    if(replay&&!original)throw new NotesGatewayError("invalid_input","note_history_unavailable: 原删除操作不可撤销。");
    const identity=replay??request;
    let state = await readState({metadataOnly: true, operationId: request.operationId});
    const previous = state.metadataReceipts?.[request.operationId];
    if (previous) {
      if (JSON.stringify(previous.request) !== JSON.stringify(identity)) throw new NotesGatewayError("invalid_input", "操作 ID 已用于不同请求");
      return previous.receipt;
    }
    const dependent = new Set(state.readingSources.filter(isDependentReadingNote).map(source => source.pageId));
    const timestamp = now();
    let plan: ReturnType<typeof planNoteMetadataBatch>;
    try { plan = planNoteMetadataBatch(state.pages, dependent, request, timestamp,!replay); }
    catch (reason) { throw new NotesGatewayError("invalid_input", reason instanceof Error ? reason.message : String(reason)); }
    if(replay&&original){
      const failure=plan.failures[0];
      if(failure)throw new NotesGatewayError(failure.code as import("@research-assistant/data/notes").NotesErrorCode,`note_history_changed: ${failure.message}（笔记 ${failure.id}）`,failure.currentRevision);
      if(request.items.length!==original.statuses.length||original.statuses.some(old=>!request.items.some(item=>item.id===old.id)||state.pages.find(page=>page.id===old.id)?.status!==(replay.direction==="undo"?"trashed":old.status)))
        throw new NotesGatewayError("revision_conflict","note_history_changed: 笔记状态已变化，请核对后重试。");
      if(replay.direction==="redo"&&state.pages.some(page=>page.status!=="trashed"&&page.parentPageId&&original.statuses.some(old=>old.id===page.parentPageId)&&!original.statuses.some(old=>old.id===page.id)))
        throw new NotesGatewayError("revision_conflict","note_history_changed: 笔记新增了子页面，请核对后重试。");
      if(replay.direction==="undo"){
        const statuses=new Map(original.statuses.map(page=>[page.id,page.status]));
        const update=(page:import("@research-assistant/core/notes").NotePage)=>statuses.has(page.id)?{...page,status:statuses.get(page.id)!}:page;
        plan={...plan,pages:plan.pages.map(update),changed:plan.changed.map(update)};
      }
    }
    const targets=new Set(plan.changed.map(page=>page.id)),trashed=new Set(plan.changed.filter(page=>page.status==="trashed").map(page=>page.id));
    const sourceIds=new Set(plan.parentIds);
    if(replay?.direction==="undo")for(const entry of original!.entries)sourceIds.add(entry.pageId);
    if(trashed.size){
      const loaded=await readState({metadataOnly:true,hbsPageIds:[...trashed],operationId:request.operationId});
      if(loaded.persistence?.version!==state.persistence?.version)throw new NotesGatewayError("revision_conflict","笔记关系已变化，请刷新后重试");
      state=loaded;
      for(const link of state.links)if(link.target.kind==="note_page" && trashed.has(link.target.id as NotePageId))sourceIds.add(link.sourcePageId);
      for(const id of creationEntryProofs(state,trashed).keys())sourceIds.add(id as NotePageId);
    }
    if (sourceIds.size) {
      const loaded = await readState({pageIds: [...sourceIds],hbsPageIds:[...sourceIds,...trashed], operationId: request.operationId});
      if (loaded.persistence?.version !== state.persistence?.version) throw new NotesGatewayError("revision_conflict", "笔记关系已变化，请刷新后重试");
      state = loaded;
    }
    const bodyChangedIds = new Set(plan.parentIds);
    const history=request.action==="trash"?captureMetadataHistory(state,trashed):undefined;
    const parents = new Set(bodyChangedIds);
    const before = new Map(state.pages.map(page => [page.id, page]));
    const pages = plan.pages;
    const changed = new Set([...plan.changed.map(page => page.id), ...bodyChangedIds]);
    const activeFavorites = pages.filter(page => page.isFavorite && !dependent.has(page.id)).map(page => page.id);
    const previousOrder = (state.favoriteOrder ?? []).filter(id => activeFavorites.includes(id));
    const newlyFavorited = request.action === "favorite" ? request.items.filter(item => changed.has(item.id) && !before.get(item.id)?.isFavorite).map(item => item.id).reverse() : [];
    const newFavorites = new Set(newlyFavorited);
    const favoritePageIds = [...newlyFavorited, ...activeFavorites.filter(id => !previousOrder.includes(id) && !newFavorites.has(id)), ...previousOrder];
    let next: BrowserNotesState = {...state, pages, favoriteOrder: favoritePageIds};
    if (plan.parentIds.length) {
      const synchronized=synchronizeParents({...next,documents:Object.fromEntries(Object.entries(next.documents).filter(([id])=>parents.has(id as NotePageId)))});
      next={...synchronized,documents:{...next.documents,...synchronized.documents}};
    }
    if(trashed.size)next=applyPageEntryDeletion(next,trashed,bodyChangedIds,savePrepared);
    if(replay?.direction==="undo")next=restoreMetadataEntries(next,original!,bodyChangedIds,savePrepared);
    next={...next,pages:next.pages.map(page=>parents.has(page.id) && page.revision===before.get(page.id)?.revision ? {...page,revision:page.revision+1,updatedAt:timestamp}:page)};
    next=synchronizePageEntryBaselines(next,targets,trashed,bodyChangedIds);
    for(const id of bodyChangedIds)changed.add(id);
    const receipt: NoteMetadataBatchReceipt = {operationId: request.operationId, succeededIds: plan.succeededIds, failures: plan.failures,
      pages: next.pages.filter(page => changed.has(page.id)), bodyChangedIds:[...bodyChangedIds], favoritePageIds,...(history?{history}:{})};
    next={...next,metadataReceipts:{...state.metadataReceipts,[request.operationId]:{request:identity,receipt}}};
    await writeState(next);
    return receipt;
  };
}
