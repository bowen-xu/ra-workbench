import {getCurrentWebview} from "@tauri-apps/api/webview";
import {isDesktopRuntime} from "./platformCore";
import {readWorkspaceState, writeWorkspaceState} from "./workspaceState";

let confirmedZoomFactor = 1;

/** Native logical drop coordinates need the WebView's actual page zoom. */
export function getDesktopZoomFactor(): number { return confirmedZoomFactor; }

/** Keyboard-only WebView zoom. PDF wheel/pinch gestures keep their own owner. */
export function installDesktopZoom({onError}: {readonly onError: (message: string) => void}): () => void {
  if (!isDesktopRuntime()) return () => {};
  const webview = getCurrentWebview();
  const mac = /Mac|iPhone|iPad|iPod/u.test(navigator.platform);
  let active = true;
  const saved = readWorkspaceState("zoom",100,value => typeof value === "number" && Number.isFinite(value) && value >= 50 && value <= 200 ? Math.round(value / 10) * 10 : 100);
  let intended = saved, confirmed = 100, revision = 0;
  let pending = Promise.resolve();
  const apply = (percent: number) => {
    const request = ++revision;
    pending = pending.then(async () => {
      if (!active) return;
      try {
        await webview.setZoom(percent / 100);
        if (!active) return;
        confirmedZoomFactor = percent / 100;
        confirmed = percent;
        writeWorkspaceState("zoom",percent);
      } catch (reason) {
        if (request === revision) intended = confirmed;
        if (active) onError(`desktop_zoom_failed：界面缩放失败。${reason instanceof Error ? reason.message : String(reason)}`);
      }
    });
  };
  const keydown = (event: KeyboardEvent) => {
    // Shift reserves the PDF-local zoom chord, including shifted +/_/) keys.
    if (event.isComposing || event.keyCode === 229 || event.altKey || event.shiftKey
      || (mac ? !event.metaKey || event.ctrlKey : !event.ctrlKey || event.metaKey)) return;
    let next: number;
    if (event.key === "+" || event.key === "=") next = Math.min(200, intended + 10);
    else if (event.key === "-") next = Math.max(50, intended - 10);
    else if (event.key === "0") next = 100;
    else return;
    event.preventDefault();
    // Capture before editor/PDF handlers; typing focus does not change ownership.
    event.stopImmediatePropagation();
    if (next === intended && event.key !== "0") return;
    intended = next;
    apply(next);
  };
  apply(saved);
  window.addEventListener("keydown", keydown, true);
  return () => {active = false; window.removeEventListener("keydown", keydown, true);};
}
