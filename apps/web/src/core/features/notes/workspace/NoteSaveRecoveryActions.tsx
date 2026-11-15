import { RotateCw } from "lucide-react";
import { ControlHint } from "../../../components/ControlHint";
import type { NoteSaveStatus } from "../editor/noteSaveController";

export interface NoteSaveRecoveryActionsProps {
  readonly saveStatus: NoteSaveStatus;
  readonly saveMessage?: string | undefined;
  readonly onRetrySave: () => void;
}

/** Recovery stays next to the save status; the document has no error banner. */
export function NoteSaveRecoveryActions(props: NoteSaveRecoveryActionsProps): React.JSX.Element | null {
  if (props.saveStatus !== "failed" && props.saveStatus !== "conflict") return null;
  const conflict = props.saveStatus === "conflict";
  return <>
    <ControlHint content={conflict ? "保存并保全当前输入；不接受候选或清除冲突" : "重试保存／核对原操作"}><button aria-label={conflict ? "保存并保全当前输入" : "重试保存或核对原操作"} className="note-page-icon-button" onClick={props.onRetrySave} type="button"><RotateCw aria-hidden="true" size={15} /></button></ControlHint>
  </>;
}
