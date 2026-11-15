import { useLayoutEffect } from "react";

/** A Tab action is declared on the exact keyboard owner, never a whole panel.
 * `forward` permits Tab; `both` also permits an explicitly implemented Shift+Tab.
 * Editable descendants/controls cannot inherit permission from an outer owner.
 */
function ownsTab(event: KeyboardEvent): boolean {
  for (const node of event.composedPath()) {
    if (!(node instanceof Element)) continue;
    const action = node.getAttribute("data-tab-action");
    if ((action === "both" || action === "forward" && !event.shiftKey)
      && !node.matches(':disabled, [aria-disabled="true"], [contenteditable="false"]')) return true;
    if (node.matches("input, textarea, button, select, a[href], [contenteditable]")) return false;
  }
  return false;
}

export function installApplicationTabPolicy(window: Window): () => void {
  const isTab = (event: KeyboardEvent) => event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey;
  const capture = (event: KeyboardEvent) => {
    if (!isTab(event)) return;
    if (!event.isComposing && event.keyCode !== 229 && ownsTab(event)) return;
    // Block library focus traversal too, before dialogs/menus handle the key.
    event.preventDefault(); event.stopImmediatePropagation();
  };
  const fallback = (event: KeyboardEvent) => { if (isTab(event)) event.preventDefault(); };
  window.addEventListener("keydown", capture, true);
  window.addEventListener("keydown", fallback);
  return () => {
    window.removeEventListener("keydown", capture, true);
    window.removeEventListener("keydown", fallback);
  };
}

export function ApplicationKeyboardPolicy(): null {
  useLayoutEffect(() => installApplicationTabPolicy(window), []);
  return null;
}
