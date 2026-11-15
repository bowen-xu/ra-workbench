import {readWorkspaceState} from "../../../runtime/workspaceState";
import type {PreviewTab} from "../../../components/workspace-tabs/previewTabs";

export interface SavedNoteTab extends PreviewTab { readonly document?: boolean }
export function decodeNoteTabs(value: unknown): readonly SavedNoteTab[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  return value.slice(0,256).flatMap(row => {
    if (!row || typeof row.id !== "string" || !row.id || seen.has(row.id) || typeof row.title !== "string") return [];
    seen.add(row.id);
    return [{id:row.id,title:row.title,preview:row.preview === true,...(row.document === true ? {document:true} : {})}];
  });
}
export const loadNoteTabs = () => readWorkspaceState("note-tabs",[],decodeNoteTabs);
