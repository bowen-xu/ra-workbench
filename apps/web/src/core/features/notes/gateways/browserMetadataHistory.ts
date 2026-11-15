import {capturePageEntries,restorePageEntries,type NoteMetadataHistory,type NotePageId} from "@research-assistant/core/notes";
import type {NoteDocumentSaveRequest} from "@research-assistant/data/notes";
import type {BrowserNotesState} from "./browserNotesStorage";
import {creationEntryProofs} from "./browserPageEntries";

export function captureMetadataHistory(state:BrowserNotesState,targets:ReadonlySet<NotePageId>):NoteMetadataHistory {
  const proofs=creationEntryProofs(state,targets);
  const entries=Object.entries(state.documents).flatMap(([id,document])=>{
    const saved=capturePageEntries(document,targets,proofs.get(id));
    const baseline=state.hbsRecords?.[id]?.baseline;
    return saved.length?[{pageId:document.pageId,saved,baseline:baseline?capturePageEntries(baseline,targets,proofs.get(id)):saved}]:[];
  });
  return {statuses:state.pages.filter(page=>targets.has(page.id)).map(({id,status})=>({id,status})),entries};
}
export function restoreMetadataEntries(state:BrowserNotesState,history:NoteMetadataHistory,changed:Set<NotePageId>,
  save:(state:BrowserNotesState,request:NoteDocumentSaveRequest)=>BrowserNotesState):BrowserNotesState {
  let next=state;
  for(const entry of history.entries){
    const document=next.documents[entry.pageId],page=next.pages.find(page=>page.id===entry.pageId);
    if(!document||!page)throw new Error("note_entry_history_changed: 来源笔记已不存在。");
    next=save(next,{document:restorePageEntries(document,entry.saved),expectedRevision:page.revision,saveReason:"structure"});
    const record=next.hbsRecords?.[entry.pageId];
    if(record?.baseline)next={...next,hbsRecords:{...next.hbsRecords,[entry.pageId]:{...record,baseline:restorePageEntries(record.baseline,entry.baseline)}}};
    changed.add(entry.pageId);
  }
  return next;
}
