import {useLayoutEffect} from "react";
import {BLOCK_CAPTION, captionBlockId} from "./captionBlockProjection";
import type {BlockEditorInstance} from "./BlockEditor";
import type {BlockTreeNode} from "./blockSelectionRoots";
import "./BlockCaption.css";

/** Geometry only: native text remains in the editor's own contentDOM. */
export function CaptionBlockLayout<Block extends BlockTreeNode>({editor}: {readonly editor: BlockEditorInstance<Block>}): null {
  useLayoutEffect(() => {
    const root = editor.domElement;
    if (!root) return;
    let active = true;
    let observed = new Set<HTMLElement>();
    const resize = new ResizeObserver(() => refresh());
    const refresh = () => {
      if (!active) return;
      const next = new Set<HTMLElement>();
      for (const content of root.querySelectorAll<HTMLElement>(`[data-content-type="${BLOCK_CAPTION}"]`)) {
        const block = content.closest<HTMLElement>(".bn-block");
        const group = block?.parentElement?.parentElement;
        const owner = group?.closest<HTMLElement>(".bn-block");
        const ownerId = owner?.dataset.id;
        if (!block || !group || !owner || !ownerId || block.dataset.id !== captionBlockId(ownerId)) continue;
        const surface = owner.firstElementChild;
        const media = surface?.querySelector<HTMLElement>(".note-image-block-media, table");
        if (!media) continue;
        // The caption's own container is display:none until focused. Its group
        // stays laid out, so both hidden and visible captions use the same origin.
        const rect = media.getBoundingClientRect(), base = group.getBoundingClientRect();
        const width = `${rect.width}px`, left = `${rect.left - base.left}px`;
        const captionGap = media.matches("table") ? 10 : 2;
        const gap = `${captionGap - (surface!.getBoundingClientRect().bottom - rect.bottom)}px`;
        if (content.style.width !== width) content.style.width = width;
        if (content.style.marginLeft !== left) content.style.marginLeft = left;
        if (content.style.marginTop !== gap) content.style.marginTop = gap;
        next.add(media);
        if (!observed.has(media)) resize.observe(media);
        next.add(group);
        if (!observed.has(group)) resize.observe(group);
      }
      for (const element of observed) if (!next.has(element)) resize.unobserve(element);
      observed = next;
    };
    const observer = new MutationObserver(refresh);
    observer.observe(root, {childList: true, subtree: true});
    const off = editor.onChange(refresh);
    const offSelection = editor.onSelectionChange(refresh);
    refresh();
    return () => {active = false; observer.disconnect(); resize.disconnect(); off(); offSelection();};
  }, [editor]);
  return null;
}
