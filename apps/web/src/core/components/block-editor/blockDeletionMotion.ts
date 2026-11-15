import {createExtension} from "@blocknote/core";
import {Plugin, type Transaction} from "@tiptap/pm/state";
import {animateReorderMotion, cancelReorderMotion} from "../reorder/reorderMotion";
import {getEditorScrollContainer, isEditorSurfaceActive} from "./editorSurface";

interface Row {readonly element: HTMLElement; readonly top: number}

function removesBlock(tr: Transaction): boolean {
  if (tr.getMeta("uiEvent") === "drop") return false;
  const surviving = new Set<string>();
  tr.doc.descendants(node => {if (typeof node.attrs.id === "string") surviving.add(node.attrs.id);});
  let removed = false;
  tr.before.descendants(node => {
    if (typeof node.attrs.id === "string" && !surviving.has(node.attrs.id)) removed = true;
  });
  return removed;
}

/** Animate surviving identities, never delay a transaction or keep deleted
 * editable DOM alive. Reuse the application's reorder motion and timing. */
export const blockDeletionMotion = createExtension(({editor}) => ({
  key: "block-deletion-motion",
  // A Plugin view is also installed when an extension is registered after
  // mount. BlockNote 0.52.1's extension.mount only runs on editor mount events.
  prosemirrorPlugins: [new Plugin({view(view) {
    const dom = view.dom;
    const window = dom.ownerDocument.defaultView;
    if (!window) return {};
    let snapshot: Map<string, Row> | undefined;
    let frame: number | undefined;
    let disposed = false;
    const animated = new Set<HTMLElement>();
    const enabled = () => typeof dom.animate === "function" && editor.isEditable && !editor.prosemirrorView.composing
      && isEditorSurfaceActive(dom)
      && dom.closest<HTMLElement>(".block-editor")?.dataset.blockDragging !== "true"
      && !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const rows = (): Map<string, Row> => {
      const origin = dom.getBoundingClientRect().top;
      const bounds = getEditorScrollContainer(dom)?.getBoundingClientRect();
      const result = new Map<string, Row>();
      for (const element of dom.querySelectorAll<HTMLElement>(".bn-block-outer")) {
        // Diff/readonly widgets can contain presentation copies of block DOM.
        // Only the live editor's direct block identity participates in motion.
        if (element.closest('.ProseMirror-widget, [contenteditable="false"]')) continue;
        const id = element.dataset.id ?? element.querySelector<HTMLElement>(":scope > .bn-block[data-id]")?.dataset.id;
        if (!id || element.closest(".bn-editor") !== dom) continue;
        const rect = element.getBoundingClientRect();
        if (!rect.height || bounds && (rect.bottom < bounds.top || rect.top > bounds.bottom)) continue;
        result.set(id, {element, top: rect.top - origin});
      }
      return result;
    };
    const cancel = () => {
      if (frame !== undefined) window.cancelAnimationFrame(frame);
      frame = undefined;
      snapshot = undefined;
      cancelReorderMotion(animated); animated.clear();
    };
    const before = editor.onBeforeChange(({tr}) => {
      if (!tr.docChanged) return;
      // onBeforeChange runs in filterTransaction, BEFORE UniqueID's appended
      // transaction. Native paste legitimately contains null IDs here. Calling
      // getChanges() eagerly requires complete IDs and aborts the entire paste.
      // Motion only needs to know whether an existing identity disappeared;
      // it must never parse/validate new blocks or participate in admission.
      const deletion = enabled() && removesBlock(tr);
      // Capture in-flight pixels BEFORE canceling. Repeated deletions then
      // continue from the visible position, rather than snapping to the old end.
      const previous = deletion ? snapshot ?? rows() : undefined;
      cancel();
      snapshot = previous;
    });
    const changed = editor.onChange(() => {
      if (!snapshot || frame !== undefined) return;
      // ProseMirror scrolls the selection after plugin view updates. Start at
      // the next pre-paint frame so its native caret scrolling reads real layout,
      // not our temporary transform. Root-relative coordinates exclude scrolling.
      frame = window.requestAnimationFrame(() => {
        frame = undefined;
        const previous = snapshot; snapshot = undefined;
        if (disposed || !previous || !enabled()) return;
        const current = rows(), offsets = new Map<HTMLElement, number>();
        for (const [id, row] of current) {
          const old = previous.get(id);
          if (old) offsets.set(row.element, old.top - row.top);
        }
        const style = window.getComputedStyle(dom);
        const duration = Number.parseFloat(style.getPropertyValue("--reorder-motion-duration") || "180ms");
        const easing = style.getPropertyValue("--reorder-motion-easing").trim() || "cubic-bezier(.2,.8,.2,1)";
        for (const [element, offset] of offsets) {
          const parent = element.parentElement?.closest<HTMLElement>(".bn-block-outer");
          // Nested rows already inherit their parent's motion; only animate
          // the remaining relative displacement, otherwise a subtree moves twice.
          const relative = offset - (parent ? offsets.get(parent) ?? 0 : 0);
          if (Math.abs(relative) < .5 || duration <= 0) continue;
          animateReorderMotion(element, relative, "vertical", duration, easing);
          animated.add(element);
        }
      });
    });
    // Remove transient transforms before editing/IME or a new pointer gesture.
    // A deletion key keeps them until `before` captures its current pixels.
    const keydown = (event: KeyboardEvent) => {
      if (!["Backspace", "Delete"].includes(event.key)) cancel();
    };
    dom.addEventListener("keydown", keydown, true);
    dom.addEventListener("compositionstart", cancel, true);
    dom.addEventListener("pointerdown", cancel, true);
    return {destroy() {
      disposed = true; before(); changed(); cancel();
      dom.removeEventListener("keydown", keydown, true);
      dom.removeEventListener("compositionstart", cancel, true);
      dom.removeEventListener("pointerdown", cancel, true);
    }};
  }})],
}));
