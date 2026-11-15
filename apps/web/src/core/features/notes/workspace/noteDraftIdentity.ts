import {loadRecoveryDraft} from "../../../services/recoveryDraftStore";
import {readWorkspaceState,writeWorkspaceState,workspaceStrings} from "../../../runtime/workspaceState";

/** Stable across first publication, so editor/block/link identities never
 * change when a temporary page becomes a canonical page. */
export const noteDraftIdentity=(id:string)=>({scope:"note-new-page",resourceId:id,slot:"hbs"});
// Keep resource IDs valid for every existing native gateway. Only presentation
// state remembers unpublished identities; canonical pages never carry a draft prefix.
export function markNoteDraftIdentity(id:string,unpublished:boolean):void {
  const ids=readWorkspaceState("unpublished-note-ids",[],workspaceStrings).filter(value=>value!==id);
  writeWorkspaceState("unpublished-note-ids",unpublished ? [id,...ids].slice(0,256) : ids);
}
export function isAbandonedEmptyNoteDraft(id:string,openIds:readonly string[]):boolean {
  if(openIds.includes(id)||!readWorkspaceState("unpublished-note-ids",[],workspaceStrings).includes(id))return false;
  // A malformed recovery record belongs to the note recovery UI; do not turn
  // a navigation preference read into an application-wide render failure.
  try{return !loadRecoveryDraft(noteDraftIdentity(id));}catch{return false;}
}
