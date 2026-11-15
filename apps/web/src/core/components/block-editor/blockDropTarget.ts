import { getEditorScrollContainer } from "./editorSurface";
import { blockNoteDragDom, type BlockDragDom } from "./blockDragDom";

interface DropCoordinates {
  readonly clientX: number;
  readonly clientY: number;
}

export interface BlockDropGeometry {
  readonly rows: readonly HTMLElement[];
  readonly rect: (element: HTMLElement) => DOMRect;
}

/** Hit testing uses the captured layout so animated rows cannot change the drop identity. */
export function captureBlockDropGeometry(editor: HTMLElement, dom: BlockDragDom = blockNoteDragDom): BlockDropGeometry {
  const pane = getEditorScrollContainer(editor);
  const scrollTop = pane?.scrollTop ?? 0;
  const rows = [...editor.querySelectorAll<HTMLElement>(dom.contentSelector)].filter(row => dom.contentId(editor, row, true) !== undefined);
  const nodes = [editor, ...editor.querySelectorAll<HTMLElement>(`${dom.outerSelector}, ${dom.contentSelector}`)];
  const rects = new Map(nodes.map(node => [node, node.getBoundingClientRect()]));
  return { rows, rect(element) {
    const rect = rects.get(element) ?? element.getBoundingClientRect();
    return new DOMRect(rect.left, rect.top - ((pane?.scrollTop ?? 0) - scrollTop), rect.width, rect.height);
  } };
}

export interface BlockDropTarget {
  readonly blockId: string;
  readonly placement: "before" | "after" | "inside";
  readonly line: { readonly left: number; readonly top: number; readonly width: number };
  readonly targetRect?: { readonly left: number; readonly top: number; readonly width: number; readonly height: number };
}

function resolveSiblingBoundary(editor: HTMLElement, outer: HTMLElement, placement: "before" | "after", rect: (element: HTMLElement) => DOMRect, dom: BlockDragDom): BlockDropTarget | undefined {
  const next = outer.nextElementSibling;
  // Canonicalize A-after and its immediate sibling B-before to B-before.
  // Never climb out of the current group: last-child-after and parent-after
  // may share a vertical coordinate but have different insertion semantics.
  const nextContent = next?.querySelector<HTMLElement>(dom.contentSelector);
  const target = placement === "after" && next instanceof HTMLElement && next.matches(dom.outerSelector)
    && nextContent && dom.contentId(editor, nextContent) ? next : outer;
  const atStart = placement === "before" || target !== outer;
  const content = target.querySelector<HTMLElement>(dom.contentSelector);
  const blockId = content ? dom.contentId(editor, content, true) : undefined;
  if (content === null || blockId === undefined) return undefined;
  const block = rect(target);
  let top = atStart ? rect(content).top : block.bottom;
  const previous = target.previousElementSibling;
  const previousContent = previous?.querySelector<HTMLElement>(dom.contentSelector);
  if (atStart && previous instanceof HTMLElement && previous.matches(dom.outerSelector)
    && previousContent && dom.contentId(editor, previousContent)) {
    // The same gap produces one line from either side, including empty rows,
    // quotes and preceding subtrees. Keep the writing margin unchanged.
    top = (rect(previous).bottom + top) / 2;
  }
  return {
    blockId,
    placement: atStart ? "before" : "after",
    line: { left: block.left, top, width: block.width },
  };
}

/** Resolve a handle gesture by block geometry, never by the target's text caret.
 * SiYuan/AppFlowy inform the boundary-first interaction (see reference records);
 * The same block identity/placement drives feedback and the public editor command.
 */
export function resolveBlockDropTarget(editor: HTMLElement, pointer: DropCoordinates, geometry?: BlockDropGeometry, dom: BlockDragDom = blockNoteDragDom): BlockDropTarget | undefined {
  const rect = geometry?.rect ?? ((element: HTMLElement) => element.getBoundingClientRect());
  const bounds = rect(editor);
  if (pointer.clientX < bounds.left - 64 || pointer.clientX > bounds.right + 24
    || pointer.clientY < bounds.top || pointer.clientY > bounds.bottom) return undefined;
  const hit = editor.ownerDocument.elementFromPoint(pointer.clientX, pointer.clientY);
  if (hit === null || !dom.owns(editor, hit)) return undefined;

  // The fast path covers text, quotes, inputs, table cells and other node views.
  // In whitespace or a fading toolbar, find the nearest content row instead.
  let content = hit.closest<HTMLElement>(dom.contentSelector);
  if (content && !dom.contentId(editor, content, true)) return undefined;
  if (geometry || content === null) {
    content = null;
    let distance = Infinity;
    for (const candidate of geometry?.rows ?? editor.querySelectorAll<HTMLElement>(dom.contentSelector)) {
      if (!dom.contentId(editor, candidate, true)) continue;
      const row = rect(candidate);
      if (row.height === 0 || row.width === 0) continue;
      const nextDistance = Math.max(row.top - pointer.clientY, pointer.clientY - row.bottom, 0);
      if (nextDistance < distance) { content = candidate; distance = nextDistance; }
    }
  }
  if (content === null) return undefined;
  const outer = content.closest<HTMLElement>(dom.outerSelector);
  if (outer === null) return undefined;
  const row = rect(content);
  if (dom.attached(content)) {
    const owner = outer.parentElement?.closest<HTMLElement>(dom.outerSelector);
    return owner ? resolveSiblingBoundary(editor, owner, "after", rect, dom) : undefined;
  }
  if (dom.fixed(content)) return resolveSiblingBoundary(editor, outer, "after", rect, dom);
  // Sibling-only hosts use the same middle band in the floating handle gutter.
  // Keeping the pointer left of the text must not change the move direction.
  if (dom.middlePlacement && row.height > 0 && pointer.clientY >= row.top + row.height / 4 && pointer.clientY < row.bottom - row.height / 4) {
    const placement = dom.middlePlacement(content);
    if (placement !== "inside") return resolveSiblingBoundary(editor, outer, placement, rect, dom);
  }
  // A central content hit has explicit child semantics. Gutters and the top /
  // bottom quarters remain sibling boundaries, so a nested destination never
  // steals the existing before/after insertion gaps.
  if (row.width > 24 && row.height > 0
    && pointer.clientX >= row.left + 24 && pointer.clientX <= row.right
    && pointer.clientY >= row.top + row.height / 4 && pointer.clientY < row.bottom - row.height / 4) {
    const blockId = dom.contentId(editor, content);
    if (blockId === undefined) return undefined;
    const block = rect(outer);
    return {
      blockId,
      placement: "inside",
      line: { left: row.left + 24, top: block.bottom, width: row.width - 24 },
      targetRect: { left: block.left, top: block.top, width: block.width, height: block.height },
    };
  }
  return resolveSiblingBoundary(editor, outer, pointer.clientY < row.top + row.height / 2 ? "before" : "after", rect, dom);
}
