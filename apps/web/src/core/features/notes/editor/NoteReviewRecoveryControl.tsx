import { createPortal } from "react-dom";
import { CircleAlert, RotateCw } from "lucide-react";
import { ControlHint } from "../../../components/ControlHint";

export function NoteReviewRecoveryControl({ error, retry, target }: {
  readonly error: string | undefined;
  readonly retry: (() => void) | undefined;
  readonly target: HTMLElement | null | undefined;
}): React.JSX.Element | null {
  if (!error) return null;
  const control = <ControlHint content={error}>{retry
    ? <button className="note-page-icon-button note-review-recovery" aria-label={`恢复原操作：${error}`} onClick={retry} type="button"><RotateCw aria-hidden="true" size={15} /></button>
    : <span className="note-review-recovery" aria-label={error} tabIndex={0}><CircleAlert aria-hidden="true" size={15} /></span>}
  </ControlHint>;
  return target ? createPortal(control, target) : control;
}
