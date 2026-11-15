import type {TextDocumentRequest,TextDocumentReopen} from "./textDocumentEvents";
import type {WorkspaceView} from "../../useWorkspaceNavigation";
import {readWorkspaceState} from "../../runtime/workspaceState";
import {systemDocumentGateway} from "../../services/systemDocumentGateway";
import {systemTextDocument} from "../../features/system-documents/openSystemDocument";
import {readAgentSkillDocument} from "../../services/agentSkillDocumentGateway";

export interface SavedTextTab {readonly id:string;readonly owner:WorkspaceView;readonly reopen:TextDocumentReopen;readonly active:boolean;readonly title?:string}
export function loadTextTabs():readonly SavedTextTab[] {
  return readWorkspaceState("text-tabs",[],value=>Array.isArray(value)?value.slice(0,128).filter(row=>{
    if (!row || typeof row.id !== "string" || !["library","notes","writing","settings"].includes(row.owner)) return false;
    const reopen=row.reopen;
    if (reopen?.kind === "system") return typeof reopen.id === "string" && row.id === `system:${reopen.id}`;
    if (reopen?.kind !== "skill" || !reopen.identity) return false;
    const identity = reopen.identity;
    return [identity.environment_id,identity.source_id,identity.revision].every(v=>typeof v === "string" && v.length>0)
      && row.id === `skill:${identity.environment_id}:${identity.source_id}`;
  }):[]);
}
export async function restoreTextTab(tab:SavedTextTab):Promise<TextDocumentRequest> {
  if (tab.reopen.kind === "system") return {...systemTextDocument(await systemDocumentGateway.read(tab.reopen.id)),owner:tab.owner};
  return {id:tab.id,owner:tab.owner,reopen:tab.reopen,path:tab.reopen.identity.source_id,
    ...(tab.title ? {title:tab.title} : {}),content:await readAgentSkillDocument(tab.reopen.identity,tab.reopen.scope),readOnly:true};
}
