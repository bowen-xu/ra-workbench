import { getDesktopZoomFactor } from "./desktopZoom";

/** Adapt the locked Wry 0.55.1 implementation, not Tauri's PhysicalPosition
 * annotation: AppKit NSPoint and GTK drag-motion use logical view coordinates;
 * WebView2 converts screen points to physical client coordinates.
 * WKWebView pageZoom does not fold into devicePixelRatio. */
export function desktopDropClientPosition(position: { readonly x: number; readonly y: number }): {
  readonly x: number; readonly y: number;
} {
  const scale = /Win/u.test(navigator.platform) ? window.devicePixelRatio : getDesktopZoomFactor();
  return { x: position.x / scale, y: position.y / scale };
}
