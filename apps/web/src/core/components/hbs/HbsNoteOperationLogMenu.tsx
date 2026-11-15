import {ChevronRight, Copy, Download, FileJson, Trash2} from "lucide-react";
import {useDeveloperMode} from "../../features/settings/DeveloperMode";
import {ActionMenu, ActionMenuItem} from "../ActionMenu";
import {useNoteOperationLogActions} from "./noteOperationLogActions";
import "./HbsNoteOperationLogMenu.css";

/** Both ordinary and reading-note headers expose the same diagnostic actions. */
export function HbsNoteOperationLogMenu({pageId}: {readonly pageId: string | undefined}): React.JSX.Element | null {
  const {enabled} = useDeveloperMode();
  const actions = useNoteOperationLogActions(pageId);
  if (!enabled || !actions) return null;
  return <ActionMenu label="笔记操作记录" className="note-operation-log-menu" contentClassName="note-page-action-menu"
    showTitle={false} portal triggerRender={<button type="button"/>}
    trigger={<><FileJson aria-hidden="true" size={16}/><span>笔记操作记录</span><ChevronRight aria-hidden="true" size={16}/></>}>
    <ActionMenuItem disabled={actions.copying} onClick={() => {void actions.copy();}}><Copy aria-hidden="true" size={16}/>复制操作记录</ActionMenuItem>
    <ActionMenuItem disabled={actions.downloading} onClick={() => {void actions.download();}}><Download aria-hidden="true" size={16}/>下载操作记录 JSON</ActionMenuItem>
    <ActionMenuItem onClick={actions.clear}><Trash2 aria-hidden="true" size={16}/>清空操作记录</ActionMenuItem>
  </ActionMenu>;
}
