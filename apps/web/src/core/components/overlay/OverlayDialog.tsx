import { Dialog as AriakitDialog, useDialogContext, useStoreState, type DialogProps } from "@ariakit/react";
import { useCallback, useLayoutEffect, useState } from "react";
import { attachOverlayScope, OverlayScopeContext, overlayStyle, useOverlayPortal, type OverlayScope } from "./overlayScope";
import "./overlay.css";
import { persistentOverlays } from "./persistentOverlays";

/** App modal boundary; visual classes remain feature-owned. Inline/nonmodal
 * surfaces retain their existing layout and the enclosing overlay scope. */
export function Dialog(props: DialogProps): React.JSX.Element {
  const context = useDialogContext();
  const storeOpen = useStoreState(props.store ?? context, "open");
  const active = props.open ?? storeOpen ?? true;
  const managed = props.modal !== false && props.portal !== false;
  const [scope] = useState<OverlayScope>(() => ({ root: document.createElement("div") }));
  const surfacePortal = useOverlayPortal("surface", props.portalElement);
  const getPersistentElements = useCallback(() => [
    ...(props.getPersistentElements?.() ?? []), ...persistentOverlays(scope.root.ownerDocument),
  ], [props.getPersistentElements, scope]);
  const modalPortal = useCallback((element: HTMLElement) => {
    // Attach only when Ariakit has registered the nested dialog. An empty
    // root attached a render earlier could be made inert by the parent modal.
    if (!scope.root.isConnected) attachOverlayScope(scope.root, element);
    return scope.root;
  }, [scope]);
  useLayoutEffect(() => () => { scope.root.remove(); }, [scope, active]);
  if (!managed) return <AriakitDialog {...props} {...(props.portal ? { portalElement: surfacePortal, style: overlayStyle("surface", props.style) } : {})} />;
  return <>{active && <OverlayScopeContext value={scope}>
    <AriakitDialog {...props} portal portalElement={modalPortal} getPersistentElements={getPersistentElements} data-overlay-surface=""
      style={{ ...props.style, zIndex: "var(--overlay-surface-level)", pointerEvents: "auto" }} />
  </OverlayScopeContext>}</>;
}
