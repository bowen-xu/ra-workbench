import {useState} from "react";
import {Check,CircleAlert, Download, ExternalLink,FileJson,RotateCw} from "lucide-react";
import {NoteActionMenu, NoteActionMenuItem,NoteActionMenuGroup} from "../../../components/block-editor/NoteActionMenu";
import type {NoteReviewMenuAction} from "../editor/noteTextEditorPorts";
import {ControlHint} from "../../../components/ControlHint";
import {getNotesGateway} from "../gateways";
import {collectNoteRecovery} from "../transfer/noteRecoveryExport";
import {saveNoteRecoveryFile} from "../transfer/saveNoteRecoveryFile";
import {notify} from "../../../services/notifications";

export function NoteRecoveryMenu({pageId,label,status,message,actions=[]}: {
  readonly pageId:string;readonly label:string;readonly status:string;readonly message:string|undefined;
  readonly actions?:readonly NoteReviewMenuAction[];
}):React.JSX.Element {
  const [busy,setBusy]=useState(false);
  async function exportFile(kind:"recovery"|"diagnostics") {
    if(busy)return;setBusy(true);
    try {
      const data=await collectNoteRecovery(pageId,getNotesGateway(),{status,...(message?{message}:{})},kind==="recovery");
      const result=await saveNoteRecoveryFile(`note-${kind}-${new Date().toISOString().replace(/[:.]/g,"-")}.json`,kind==="recovery"?data:data.diagnostics);
      if(result!=="cancelled")notify.success(`${result==="saved"?"已导出":"已发起下载"}${kind==="recovery"?"数据恢复包":"错误日志"}${data.missing.length?`（${data.missing.length} 项材料不可读取，详见文件内清单）`:""}。`);
    }catch(reason){notify.error(reason instanceof Error?reason.message:String(reason));}
    finally {setBusy(false);}
  }
  return <ControlHint content={label}><span className="note-recovery-menu-host" role="status" aria-label={label}>
    <NoteActionMenu label={label} menuLabel="笔记恢复与诊断" className="note-recovery-menu" triggerClassName={`note-save-state note-page-icon-button ${status}`}
      trigger={<CircleAlert aria-hidden="true" size={16}/>} portal hint={false} showTitle={false} disabled={busy}>
      {actions.map(action=>{const Icon={keep:Check,retry:RotateCw,open:ExternalLink}[action.icon];return <NoteActionMenuGroup key={action.id} label={action.group}>
        <NoteActionMenuItem disabled={busy||action.disabled} onClick={action.run}><Icon size={16}/>{action.label}</NoteActionMenuItem>
      </NoteActionMenuGroup>;})}
      <NoteActionMenuItem disabled={busy} onClick={()=>void exportFile("recovery")}><Download size={16}/>导出数据恢复包</NoteActionMenuItem>
      <NoteActionMenuItem disabled={busy} onClick={()=>void exportFile("diagnostics")}><FileJson size={16}/>导出错误日志</NoteActionMenuItem>
    </NoteActionMenu>
  </span></ControlHint>;
}
