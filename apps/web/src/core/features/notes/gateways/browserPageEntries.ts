import {pageEntryProofs,removePageEntries,synchronizeLegacyPageEntries,type NotePageId} from "@research-assistant/core/notes";
import type {NoteDocumentSaveRequest} from "@research-assistant/data/notes";
import type {BrowserNotesState} from "./browserNotesStorage";

export function creationEntryProofs(state:BrowserNotesState,targets:ReadonlySet<string>):ReadonlyMap<string,ReadonlyMap<string,string>> {
  const result=new Map<string,Map<string,string>>();
  for(const target of targets)for(const receipt of Object.values(state.hbsRecords?.[target]?.receipts ?? {})){
    const source=receipt.creation?.source?.document;if(!source)continue;
    const entries=result.get(source.pageId) ?? new Map<string,string>();
    for(const [id,page] of pageEntryProofs(source,target))entries.set(id,page);
    result.set(source.pageId,entries);
  }
  return result;
}

export function applyPageEntryDeletion(state:BrowserNotesState,targets:ReadonlySet<string>,changedBodies:Set<NotePageId>,
  save:(state:BrowserNotesState,request:NoteDocumentSaveRequest)=>BrowserNotesState):BrowserNotesState {
  let next=state;const proofs=creationEntryProofs(state,targets);
  for(const [id,document] of Object.entries(state.documents)){
    const after=removePageEntries(document,targets,proofs.get(id));
    if(JSON.stringify(after)===JSON.stringify(document))continue;
    const page=next.pages.find(page=>page.id===id);if(!page)continue;
    next=save(next,{document:after,expectedRevision:page.revision,saveReason:"structure"});changedBodies.add(page.id);
  }
  return next;
}

/** A user lifecycle edit advances only its entry identities in B. It must not
 * accept unrelated pending text or ordinary links by assigning B = S. */
export function synchronizePageEntryBaselines(state:BrowserNotesState,targets:ReadonlySet<string>,trashed:ReadonlySet<string>,changedBodies:ReadonlySet<NotePageId>):BrowserNotesState {
  const records={...state.hbsRecords},proofs=creationEntryProofs(state,trashed);
  for(const id of changedBodies){
    const record=records[id],current=state.documents[id];if(!record?.baseline || !current)continue;
    const baseline=synchronizeLegacyPageEntries(record.baseline,current,targets);
    records[id]={...record,baseline:removePageEntries(baseline,trashed,proofs.get(id))};
  }
  return {...state,hbsRecords:records};
}
