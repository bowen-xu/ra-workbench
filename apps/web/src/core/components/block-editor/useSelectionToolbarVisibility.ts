import {posToDOMRect} from "@tiptap/core";
import type {EditorView} from "@tiptap/pm/view";
import {useLayoutEffect, useState} from "react";

/** A portaled selection toolbar must not become a viewport-sticky toolbar
 * when its virtual reference scrolls out of the editor's clipped surface. */
function isSelectionVisible(view: EditorView): boolean {
  if (view.isDestroyed || !view.dom.isConnected || view.state.selection.empty) return false;
  const window = view.dom.ownerDocument.defaultView;
  if (!window) return false;
  const {from, to} = view.state.selection;
  const selection = posToDOMRect(view, from, to);
  let top = 0, bottom = window.innerHeight, left = 0, right = window.innerWidth;
  for (let node: HTMLElement | null = view.dom; node; node = node.parentElement) {
    const style = window.getComputedStyle(node);
    const clipsY = /auto|scroll|hidden|clip|overlay/u.test(style.overflowY);
    const clipsX = /auto|scroll|hidden|clip|overlay/u.test(style.overflowX);
    if (!clipsY && !clipsX) continue;
    const rect = node.getBoundingClientRect();
    if (clipsY) { top = Math.max(top, rect.top); bottom = Math.min(bottom, rect.bottom); }
    if (clipsX) { left = Math.max(left, rect.left); right = Math.min(right, rect.right); }
  }
  // Anchor below the selection, not below an offscreen range clamped to the
  // browser edge. A partially visible range can resume when its bottom returns.
  return selection.bottom > top && selection.bottom <= bottom && selection.top < bottom
    && selection.right > left && selection.left < right;
}

export function useSelectionToolbarVisibility(editor: {readonly prosemirrorView: EditorView}, from: number, to: number): boolean {
  const [scrolling, setScrolling] = useState(false);
  const [visible, setVisible] = useState(false);
  useLayoutEffect(() => { setVisible(isSelectionVisible(editor.prosemirrorView)); }, [editor, from, to]);
  useLayoutEffect(() => {
    const view = editor.prosemirrorView;
    const dom = view.dom, document = dom.ownerDocument, window = document.defaultView;
    let idle: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => setVisible(isSelectionVisible(view));
    const onScroll = (event: Event) => {
      const target = event.target;
      // Ignore sibling panes and portaled controls' own horizontal scrolling.
      if (target !== document && !(target instanceof Element && (target.contains(dom) || dom.contains(target)))) return;
      setScrolling(true);
      clearTimeout(idle);
      // Actual scroll events include momentum; scrollend is not consistently
      // available in the desktop WebKit versions we support.
      idle = setTimeout(() => { refresh(); setScrolling(false); }, 150);
    };
    document.addEventListener("scroll", onScroll, {capture: true, passive: true});
    window?.addEventListener("resize", refresh);
    return () => {
      document.removeEventListener("scroll", onScroll, true);
      window?.removeEventListener("resize", refresh);
      clearTimeout(idle);
    };
  }, [editor]);
  return visible && !scrolling;
}
