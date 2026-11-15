import type {ReactNode} from "react";
import {ShimmerText} from "../../components/ShimmerText";
import "./AgentComposerInputState.css";

/** Keeps the editor mounted while a shared preparing state temporarily covers its draft. */
export function AgentComposerInputState({children, preparingText, action}: {
  readonly children: ReactNode;
  readonly preparingText: string | null;
  readonly action?: {readonly label: string; readonly onClick: () => void; readonly disabled?: boolean} | undefined;
}): React.JSX.Element {
  return <div className={`agent-composer-input-state${preparingText ? " is-preparing" : ""}`}>
    {children}
    {preparingText && <div className="agent-composer-state-overlay">
      <ShimmerText className="agent-composer-preparing-state" role="status" aria-label="输入框准备状态"
        maskWidthPercent={100} centerOpacityPercent={15} sweepDurationSeconds={1.5} intervalSeconds={0.5}>{preparingText}</ShimmerText>
      {action && <button className="agent-composer-state-action" type="button" disabled={action.disabled} onClick={action.onClick}>{action.label}</button>}
    </div>}
  </div>;
}
