import { Portal } from "@ariakit/react";
import { useCallback, useLayoutEffect, useState, type ReactNode } from "react";
import { OverlayScopeContext, type OverlayScope } from "./overlayScope";
import { registerNotificationOverlay } from "./persistentOverlays";
import "./overlay.css";
import { useGlobalOverlayOwner } from "./useGlobalOverlayOwner";

/** Third-party global surfaces own a scope too. Their hints are siblings of
 * the surface, outside its transformed stack and gesture/overflow containers. */
export function OverlayHost({ children }: { readonly children: ReactNode }): React.JSX.Element {
  const [scope] = useState<OverlayScope>(() => ({ root: document.createElement("div") }));
  useGlobalOverlayOwner(scope.root);
  const portalElement = useCallback((element: HTMLElement) => {
    const root = scope.root;
    root.dataset.overlayScope = "notification";
    root.style.cssText = "position:relative;isolation:isolate;pointer-events:none;z-index:var(--overlay-notification-base)";
    (element.closest("dialog") ?? element.ownerDocument.fullscreenElement ?? element.ownerDocument.body).appendChild(root);
    return root;
  }, [scope]);
  useLayoutEffect(() => {
    const unregister = registerNotificationOverlay(scope.root);
    return () => { unregister(); scope.root.remove(); };
  }, [scope]);
  return <OverlayScopeContext value={scope}>
    <Portal portalElement={portalElement} style={{ display: "contents", pointerEvents: "auto" }}>{children}</Portal>
  </OverlayScopeContext>;
}
