import { ControlHint } from "../ControlHint";
import type { EditorReviewConfiguration } from "./EditorReviewActions";
import { editorReviewLocationActions, type EditorReviewLocationSide } from "./reviewLocationIntents";
import "./EditorDiffControls.css";

/** Both locations emit the same decision's intent. Jumping only belongs to
 * the host's scrolling/focus adapter and never invokes the review callback. */
export function EditorReviewLocationActions({side,onJump,onResolve,configuration,disabled=false}: {
  readonly side: EditorReviewLocationSide;
  readonly onJump: () => void;
  readonly onResolve: (action: "keep" | "undo") => void;
  readonly configuration?: EditorReviewConfiguration | undefined;
  readonly disabled?: boolean;
}): React.JSX.Element {
  const actions=editorReviewLocationActions(side), action=actions.resolve;
  const button=<button type="button" className={`writing-diff-chunk-${action}`} disabled={disabled}
    onClick={()=>onResolve(action)}>{configuration?.labels?.[action] ?? (action === "keep" ? "Keep" : "Undo")}</button>;
  const hint=configuration?.hints?.[action];
  return <div className="editor-diff-chunk-actions writing-diff-chunk-actions" data-review-location-side={side}>
    <div role="group" aria-label={side === "before" ? "旧位置审阅操作" : "新位置审阅操作"}>
      <ControlHint content={actions.jumpHint}><button type="button" className="writing-diff-chunk-undo"
        data-review-location-jump disabled={disabled} onClick={onJump}>{actions.jumpLabel}</button></ControlHint>
      {hint ? <ControlHint content={hint}>{button}</ControlHint> : button}
    </div>
  </div>;
}
