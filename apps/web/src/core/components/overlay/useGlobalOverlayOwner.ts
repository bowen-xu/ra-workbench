import { useLayoutEffect } from "react";

/** Native modal dialogs outrank every body z-index and make body siblings
 * inert. Keep global task controls within the active native/fullscreen owner,
 * without remounting their content or losing task/gesture state. */
export function useGlobalOverlayOwner(root: HTMLElement): void {
  useLayoutEffect(() => {
    const doc = root.ownerDocument;
    const update = () => {
      // A native dialog can be unmounted without calling close(). Its removal
      // disconnects this persistent root too; reattach it rather than losing
      // the notification tree with the discarded owner.
      let dialogs: HTMLDialogElement[];
      try { dialogs = [...doc.querySelectorAll<HTMLDialogElement>("dialog:modal")]; }
      catch { dialogs = []; }
      const focused = doc.activeElement?.closest("dialog");
      const native = dialogs.find(dialog => dialog === focused) ?? dialogs.at(-1);
      const destination = native ?? doc.fullscreenElement ?? doc.body;
      if (root.parentElement !== destination) destination.appendChild(root);
    };
    const observer = new MutationObserver(records => {
      const ownershipChanged = records.some(record => record.type === "attributes"
        ? record.target instanceof HTMLDialogElement
        : [...record.addedNodes, ...record.removedNodes].some(node => node instanceof Element
          && (node.matches("dialog") || node.querySelector("dialog") || node.contains(root))));
      if (ownershipChanged) update();
    });
    observer.observe(doc.body, {subtree: true, childList: true, attributes: true, attributeFilter: ["open"]});
    doc.addEventListener("fullscreenchange", update);
    doc.addEventListener("focusin", update);
    update();
    return () => {
      observer.disconnect();
      doc.removeEventListener("fullscreenchange", update);
      doc.removeEventListener("focusin", update);
    };
  }, [root]);
}
