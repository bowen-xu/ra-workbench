import {useState} from "react";
import {FlaskConical} from "lucide-react";
import {useAppRuntime} from "../runtime/AppRuntimeProvider";
import {noteRecoveryCases, noteRecoveryGroups} from "../runtime/noteRecoveryPreview";
import {NoteActionMenu, NoteActionMenuGroup, NoteActionMenuItem} from "./block-editor/NoteActionMenu";
import {OverlayPortal} from "./overlay/OverlayPortal";
import {notify} from "../services/notifications";
import "./NotesPreviewControl.css";

/** Harness controls only; the App owns every editor and recovery surface. */
export function NoteRecoveryPreviewControl(): React.JSX.Element | null {
  const runtime = useAppRuntime(), recovery = runtime.noteRecovery;
  const [busy, setBusy] = useState(false);
  if (!recovery) return null;
  const current = noteRecoveryCases.find(item => item[0] === recovery.scenario)!;
  const release = async () => {
    setBusy(true);
    try {await recovery.releaseFault(); window.location.reload();}
    catch (reason) {notify.error(String(reason)); setBusy(false);}
  };
  const reset = async () => {
    setBusy(true);
    try {await runtime.preview?.reset();}
    catch (reason) {notify.error(String(reason)); setBusy(false);}
  };
  return <OverlayPortal><div className="notes-preview-control note-recovery-preview-control">
    <NoteActionMenu label="笔记故障预览" className="notes-preview-menu" portal placement="top-end" triggerClassName="notes-preview-trigger"
      trigger={<><FlaskConical size={14}/>故障预览：{current[1]}</>}>
      <NoteActionMenuItem disabled>{current[2]}</NoteActionMenuItem>
      <NoteActionMenuItem disabled={busy} onClick={() => void release()}>解除模拟 I/O 故障并重开</NoteActionMenuItem>
      <NoteActionMenuItem disabled={busy} onClick={() => void reset()}>重置本场景的 mock 数据</NoteActionMenuItem>
      {noteRecoveryGroups.map(group => <NoteActionMenuGroup key={group.label} label={group.label}>
        {group.cases.map(([id, label]) => <NoteActionMenuItem key={id} disabled={busy || id === recovery.scenario}
          onClick={() => {const url = new URL(window.location.href); url.search = new URLSearchParams({preview: "app", "note-recovery": id}).toString(); window.location.assign(url.href);}}>{label}</NoteActionMenuItem>)}
      </NoteActionMenuGroup>)}
    </NoteActionMenu>
  </div></OverlayPortal>;
}
