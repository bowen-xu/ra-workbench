import { Portal } from "@ariakit/react";
import { useLayoutEffect, useState, type ReactNode, type RefObject } from "react";
import { useOverlayPortal, type OverlayKind } from "./overlayScope";
import "./overlay.css";

/** For third-party or coordinate-positioned content. New interactive popups
 * use OverlayPopup instead so Ariakit also owns positioning and dismissal. */
export function OverlayPortal({ children, kind = "popup" }: {
  readonly children: ReactNode;
  readonly kind?: OverlayKind;
}): React.JSX.Element {
  const portalElement = useOverlayPortal(kind);
  return <Portal portalElement={portalElement} style={{ display: "contents", pointerEvents: kind === "hint" ? "none" : "auto" }}>
    {children}
  </Portal>;
}

/** Stable public mount target for libraries such as BlockNote. The library
 * retains ownership of its content container, classes and local UI ordering. */
export function useOverlayTarget(kind: OverlayKind, anchor: RefObject<HTMLElement | null>): HTMLElement {
  const [target] = useState(() => document.createElement("div"));
  const portalElement = useOverlayPortal(kind);
  useLayoutEffect(() => {
    if (!anchor.current) return;
    target.style.pointerEvents = kind === "hint" ? "none" : "auto";
    target.style.display = "contents";
    portalElement(anchor.current).appendChild(target);
  }, [anchor, kind, portalElement, target]);
  return target;
}
