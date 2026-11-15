import type { MouseEvent, ReactNode } from "react";
import { ArrowRight, ArrowUp, RotateCw, Search, Square, Undo2, X } from "lucide-react";
import { ControlHint } from "../components/ControlHint";

export interface NotificationAction {
  readonly id: string;
  readonly label: string;
  readonly icon?: ReactNode;
  readonly disabled?: boolean;
  readonly onClick: () => void | Promise<void>;
}

/** Pending actions are owned by the logical message, including while it is hidden. */
export function NotificationActions({ actions, pendingActionId, onRun }: {
  readonly actions: readonly NotificationAction[];
  readonly pendingActionId: string | undefined;
  readonly onRun: (action: NotificationAction, event: MouseEvent<HTMLButtonElement>) => Promise<void>;
}): React.JSX.Element {
  function icon(label: string): ReactNode {
    if (/核对|查看|读取/u.test(label)) return <Search aria-hidden="true" />;
    if (/返回|撤销/u.test(label)) return <Undo2 aria-hidden="true" />;
    if (/重试|刷新|恢复/u.test(label)) return <RotateCw aria-hidden="true" />;
    if (/发送/u.test(label)) return <ArrowUp aria-hidden="true" />;
    if (/终止|停止/u.test(label)) return <Square aria-hidden="true" />;
    if (/取消|关闭|放弃/u.test(label)) return <X aria-hidden="true" />;
    return <ArrowRight aria-hidden="true" />;
  }
  return <span className="app-notification-actions">
    {actions.map(action => <ControlHint key={action.id} content={action.label} wrap>
      <button type="button" className="app-toast-action app-toast-icon-action"
      aria-label={action.label}
      disabled={action.disabled || pendingActionId !== undefined} aria-busy={pendingActionId === action.id}
      onClick={event => { void onRun(action, event); }}>{action.icon ?? icon(action.label)}</button>
    </ControlHint>)}
  </span>;
}
