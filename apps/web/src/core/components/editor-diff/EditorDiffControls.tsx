import { ControlHint } from "../ControlHint";
import { Check, ChevronDown, ChevronUp, GitCompareArrows, X } from "lucide-react";
import "./EditorDiffControls.css";

/** Shared review controls. The host owns document, baseline, and history. */
export function EditorDiffControls({ count, active, disabled = false, showBulk = true, onMove, onResolve }: {
  readonly count: number;
  readonly active: number;
  readonly disabled?: boolean;
  readonly showBulk?: boolean;
  readonly onMove: (direction: "previous" | "next") => void;
  readonly onResolve: (action: "keep" | "undo", scope: "current" | "all") => void;
}): React.JSX.Element | null {
  if (!count) return null;
  return <div className="editor-diff-controls writing-diff-control-row" aria-label="变更控制">
    <div className="writing-diff-control-group writing-diff-navigation-group">
      <span className="writing-diff-control-icon" aria-hidden="true"><GitCompareArrows size={15} /></span>
      <span aria-label="当前变更位置">{`${active}/${count}`}</span>
      <ControlHint content="上一处变更"><button aria-label="上一处变更" disabled={disabled} onClick={() => onMove("previous")} type="button"><ChevronUp aria-hidden="true" size={15} /></button></ControlHint>
      <ControlHint content="下一处变更"><button aria-label="下一处变更" disabled={disabled} onClick={() => onMove("next")} type="button"><ChevronDown aria-hidden="true" size={15} /></button></ControlHint>
      <ControlHint content="应用当前变更"><button aria-label="应用当前变更" className="writing-diff-accept" disabled={disabled || !active} onClick={() => onResolve("keep", "current")} type="button"><Check aria-hidden="true" size={15} /></button></ControlHint>
      <ControlHint content="撤销当前变更"><button aria-label="撤销当前变更" className="writing-diff-reject" disabled={disabled || !active} onClick={() => onResolve("undo", "current")} type="button"><X aria-hidden="true" size={15} /></button></ControlHint>
    </div>
    {showBulk && <div className="writing-diff-control-group writing-diff-bulk-group">
      <ControlHint content="Undo all"><button aria-label="Undo all" className="writing-diff-all" disabled={disabled} onClick={() => onResolve("undo", "all")} type="button">Undo all</button></ControlHint>
      <ControlHint content="Keep all"><button aria-label="Keep all" className="writing-diff-all" disabled={disabled} onClick={() => onResolve("keep", "all")} type="button">Keep all</button></ControlHint>
    </div>}
  </div>;
}
