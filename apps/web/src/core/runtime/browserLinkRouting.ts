import type { AppRuntime } from "./appRuntime";
import { isWebUrl, openBrowserPage } from "./browserAdapter";
import { notify } from "../services/notifications";
import {trackLinkTextGesture} from "./linkTextGesture";

export function openBrowserPageFromUi(url: string, runtime?: AppRuntime): void {
  void openBrowserPage(url, runtime).catch(reason => notify.error(reason instanceof Error ? reason.message : String(reason)));
}

/** One application owner covers React, portaled controls and PDF.js annotations.
 * Shared editor defaults disable its mouseup open; capture owns the final click
 * before Tauri's anchor interceptor.
 * Internal addresses, fragments, downloads and context menus keep their owners.
 */
export function installBrowserLinkRouting(document: Document, runtime: AppRuntime): () => void {
  const textGesture = trackLinkTextGesture(document);
  const open = (event: MouseEvent) => {
    if (event.defaultPrevented || (event.type === "click" ? event.button !== 0 : event.button !== 1)) return;
    // Selection owns this gesture before any protocol's navigation handler,
    // including internal source links handled later by the note surface.
    if (event.type === "click" && textGesture.consumeClick(event)) {
      event.preventDefault(); event.stopPropagation(); return;
    }
    const anchor = event.composedPath().find((node): node is HTMLAnchorElement => node instanceof HTMLAnchorElement);
    const href = anchor?.getAttribute("href");
    if (!anchor || anchor.hasAttribute("download") || !href || !isWebUrl(href)) return;
    event.preventDefault(); event.stopPropagation();
    openBrowserPageFromUi(href, runtime);
  };
  document.addEventListener("click", open, true);
  document.addEventListener("auxclick", open, true);
  return () => {
    textGesture.dispose();
    document.removeEventListener("click", open, true);
    document.removeEventListener("auxclick", open, true);
  };
}
