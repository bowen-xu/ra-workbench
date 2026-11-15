import { BellRing } from "lucide-react";
import { useSyncExternalStore } from "react";
import { getHiddenNotifications, notify, subscribeHiddenNotifications } from "../services/notifications";
import { ControlHint } from "./ControlHint";
import "./AppToaster.css";

export function NotificationRestoreButton(): React.JSX.Element | null {
  const hidden = useSyncExternalStore(subscribeHiddenNotifications, getHiddenNotifications);
  if (hidden.length === 0) return null;
  const label = `显示 ${hidden.length} 条隐藏消息`;
  return <ControlHint content={label}>
    <button className="app-notification-restore" type="button" aria-label={label} onClick={() => notify.showHidden()}>
      <BellRing size={16} aria-hidden="true" /><span>{hidden.length}</span>
    </button>
  </ControlHint>;
}
