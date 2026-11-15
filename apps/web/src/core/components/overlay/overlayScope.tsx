import { createContext, useCallback, useContext, useLayoutEffect, useMemo, useRef, type CSSProperties } from "react";
import "./overlay.css";

export interface OverlayScope { readonly root: HTMLElement }
export const OverlayScopeContext = createContext<OverlayScope | undefined>(undefined);
const stack = new WeakMap<Document, number>();

/** Retained nonmodal workspaces keep their geometry and DOM host. Their child
 * portals share this boundary without making the surrounding page inert. */
export function useOverlayPanelScope(root: HTMLElement | null): OverlayScope | undefined {
  const scope = useMemo(() => root ? { root } : undefined, [root]);
  useLayoutEffect(() => {
    if (!root) return;
    root.dataset.overlayScope = "panel";
    return () => { delete root.dataset.overlayScope; };
  }, [root]);
  return scope;
}

/** Each modal is a stacking context. Child popups never compete with another
 * modal's content; the next modal covers the previous scope as a whole. */
export function attachOverlayScope(root: HTMLElement, anchor: HTMLElement): void {
  const doc = anchor.ownerDocument;
  const order = (stack.get(doc) ?? 0) + 1; stack.set(doc, order);
  root.dataset.overlayScope = "modal";
  root.style.cssText = `position:relative;isolation:isolate;pointer-events:none;z-index:calc(var(--overlay-modal-base, 1000) + ${order})`;
  // A body portal cannot escape a native dialog's browser top layer. Keep
  // scoped roots inside that dialog, outside its own scrolling content.
  (anchor.closest("dialog") ?? doc.fullscreenElement ?? doc.body).appendChild(root);
}

export type OverlayKind = "surface" | "popup" | "hint";
export function overlayStyle(kind: OverlayKind, style?: CSSProperties): CSSProperties {
  return { ...style, zIndex: `var(--overlay-${kind}-level)`, pointerEvents: kind === "hint" ? "none" : "auto" };
}

function attachFloatingPortal(host: HTMLElement, element: HTMLElement, kind: OverlayKind, scope?: OverlayScope,
  explicit?: HTMLElement | null): void {
  const doc = element.ownerDocument;
  const scopeRoot = scope?.root ?? element.closest<HTMLElement>("[data-overlay-scope]");
  const native = element.closest("dialog");
  const owner = native && !native.contains(scopeRoot ?? null) ? native : scopeRoot ?? native;
  host.dataset.overlayPortal = kind;
  host.style.cssText = "position:relative;isolation:isolate;pointer-events:none";
  host.style.zIndex = owner ? `var(--overlay-${kind}-level)`
    : `calc(var(--overlay-page-base, 200) + var(--overlay-${kind}-level))`;
  (owner ?? explicit ?? doc.fullscreenElement ?? doc.body).appendChild(host);
}

/** Short-lived imperative gesture feedback follows the same owner as React UI. */
export function mountOverlayFeedback(anchor: HTMLElement, ...elements: HTMLElement[]): () => void {
  const host = anchor.ownerDocument.createElement("div");
  attachFloatingPortal(host, anchor, "popup");
  host.append(...elements);
  return () => host.remove();
}

/** Ariakit still owns positioning, nested dismissal and focus. Its portal
 * function receives the original DOM element before moving it. */
export function useOverlayPortal(kind: OverlayKind, custom?: HTMLElement | null | ((element: HTMLElement) => HTMLElement | null)) {
  const scope = useContext(OverlayScopeContext);
  const portal = useRef<HTMLElement | undefined>(undefined);
  useLayoutEffect(() => () => { portal.current?.remove(); portal.current = undefined; }, []);
  return useCallback((element: HTMLElement): HTMLElement => {
    const doc = element.ownerDocument;
    const host = portal.current ?? doc.createElement("div"); portal.current = host;
    const explicit = typeof custom === "function" ? custom(element) : custom;
    attachFloatingPortal(host, element, kind, scope, explicit);
    return host;
  }, [scope, kind, custom]);
}
