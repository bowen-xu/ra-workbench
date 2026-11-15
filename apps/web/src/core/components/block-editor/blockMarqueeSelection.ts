import { getEditorScrollContainer, isEditorSurfaceActive, isSingleEditorPane, ownsEditorPaneBackground } from "./editorSurface";
import { startMarqueeSelection } from "../selection/marqueeSelection";
import { createMarqueeHitTest } from "../selection/marqueeHitTest";
import { blockDragScrollVelocity } from "./useBlockDragAutoScroll";
import { getBlockContentOwner } from "./blockContentOwner";

interface MarqueeCallbacks {
  readonly onActivate: () => void;
  readonly onSelect: (ids: readonly string[]) => void;
  readonly onFinish: (cancelled: boolean) => void;
}

/** Side and bottom whitespace start a block marquee. Text, widgets, tables, controls
 * and gestures from another pane retain their existing event ownership.
 */
export function isBlockMarqueeStart(editor: HTMLElement, event: PointerEvent): boolean {
  if (event.button !== 0 || !event.isPrimary || event.pointerType === "touch" || !(event.target instanceof Element)) return false;
  const surface = editor.closest<HTMLElement>(".block-editor");
  const pane = getEditorScrollContainer(editor) ?? surface;
  if (pane === null || !isEditorSurfaceActive(editor) || !pane.contains(event.target)) return false;
  // A shared scrolling pane can contain several editors. Never capture another
  // editor's whitespace or treat its text as this editor's marquee gesture.
  const owner = event.target.closest(".block-editor");
  if (owner !== null && owner !== surface) return false;
  // Ancestor layout backgrounds belong to this editor too. A sticky header
  // or relationship sibling can overlap the same bounds without owning it.
  if (owner === null && !ownsEditorPaneBackground(editor, event.target, pane)) return false;
  if (event.target.closest('.bn-block-content, .bn-side-menu, button, input, textarea, a, [role="menu"], [role="dialog"]')) return false;
  const bounds = editor.getBoundingClientRect();
  const surfaceBounds = surface?.getBoundingClientRect() ?? bounds;
  const visible = pane.getBoundingClientRect();
  // A single-editor pane owns its outer side margins as well as the editable
  // root's gutters. Shared panes keep each editor's horizontal footprint.
  const singleEditor = isSingleEditorPane(editor, pane);
  const horizontal = singleEditor ? visible : bounds;
  if (event.clientY < Math.max(bounds.top, visible.top)
    || event.clientY > (singleEditor ? visible.bottom : Math.min(Math.max(bounds.bottom, surfaceBounds.bottom), visible.bottom))
    || event.clientX < Math.max(horizontal.left, visible.left) || event.clientX > Math.min(horizontal.right, visible.right)) return false;
  const rows = Array.from(editor.querySelectorAll<HTMLElement>(".bn-block-content"))
    .filter(row => getBlockContentOwner(editor, row) !== undefined)
    .map((row) => row.getBoundingClientRect()).filter((rect) => rect.width > 0 && rect.height > 0);
  if (rows.length === 0) return false;
  const left = Math.min(...rows.map((rect) => rect.left));
  const right = Math.max(...rows.map((rect) => rect.right));
  const bottom = Math.max(...rows.map(rect => rect.bottom));
  return event.clientX < left || event.clientX > right || event.clientY >= bottom;
}

/** Block hit testing stays in the editor; pointer/scroll/overlay lifetime is shared with tables. */
export function startBlockMarqueeSelection(editor: HTMLElement, initial: PointerEvent, callbacks: MarqueeCallbacks): () => void {
  const pane = getEditorScrollContainer(editor) ?? editor;
  // Test content rows, not outer wrappers: selecting a child alone must not
  // implicitly select its ancestor. Root normalization stays in the editor.
  const hitTest = createMarqueeHitTest(editor, pane, ".bn-block-content", row => getBlockContentOwner(editor, row)?.dataset.id);
  return startMarqueeSelection(editor, initial, { ...callbacks, scrollContainer: pane,
    autoScroll: true, scrollVelocity: (pointer, bounds) => blockDragScrollVelocity(pointer.clientY, bounds.top, bounds.bottom),
    className: "note-block-selection-rect", getIds: hitTest.getIds,
    onFinish: cancelled => { hitTest.dispose(); callbacks.onFinish(cancelled); } });
}
