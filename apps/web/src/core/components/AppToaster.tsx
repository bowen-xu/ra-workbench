import { Toaster } from "sonner";
import { useEffect } from "react";
import { replayVisibleNotifications } from "../services/notifications";
import "./AppToaster.css";
import { OverlayHost } from "./overlay/OverlayHost";

export function AppToaster(): React.JSX.Element {
  useEffect(() => replayVisibleNotifications(), []);
  return (
    <OverlayHost><Toaster
      style={{ zIndex: "var(--overlay-surface-level)" }}
      closeButton
      containerAriaLabel="通知"
      duration={4_500}
      expand={false}
      gap={8}
      position="top-right"
      offset={44}
      richColors={false}
      toastOptions={{
        closeButtonAriaLabel: "关闭通知",
        classNames: {
          actionButton: "app-toast-action",
          cancelButton: "app-toast-cancel",
          closeButton: "app-toast-close",
          content: "app-toast-content",
          description: "app-toast-description",
          error: "app-toast-error",
          info: "app-toast-info",
          success: "app-toast-success",
          title: "app-toast-title",
          toast: "app-toast",
          warning: "app-toast-warning",
        },
      }}
      visibleToasts={4}
    /></OverlayHost>
  );
}
