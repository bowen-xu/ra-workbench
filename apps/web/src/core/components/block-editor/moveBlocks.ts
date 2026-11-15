import { getSelectedBlockRootIds, type BlockTreeNode } from "./blockSelectionRoots";
import {isAttachedTextBlock, isFixedBlock} from "./blockEditingConstraints";

export interface BlockPlacement { readonly blockId: string; readonly placement: "before" | "after" | "inside" }
export interface BlockMovePolicy {
  readonly maxDepth?: number;
  readonly canContain?: (block: BlockTreeNode) => boolean;
  readonly canNest?: (block: BlockTreeNode) => boolean;
}

/** The public BlockNote operations used by movement; no schema or storage type. */
export interface BlockMoveEditor<Block extends BlockTreeNode> {
  readonly document: readonly Block[];
  getBlock(id: string): Block | undefined;
  removeBlocks(ids: string[]): unknown;
  insertBlocks(blocks: NoInfer<Block>[], anchor: string, placement: "before" | "after"): unknown;
  updateBlock(id: string, update: { children: NoInfer<Block>[] }): unknown;
  transact(callback: () => void): unknown;
  setTextCursorPosition(id: string, placement: "start"): void;
  focus(): void;
}
interface BlockLocation {
  readonly block: BlockTreeNode;
  readonly siblings: readonly BlockTreeNode[];
  readonly depth: number;
  readonly parentId: string | undefined;
}
interface ResolvedMove { readonly roots: readonly string[]; readonly target: BlockPlacement }

function indexBlocks(blocks: readonly BlockTreeNode[]): ReadonlyMap<string, BlockLocation> {
  const index = new Map<string, BlockLocation>();
  const visit = (siblings: readonly BlockTreeNode[], depth: number, parentId?: string): void => {
    for (const block of siblings) {
      index.set(block.id, { block, siblings, depth, parentId });
      visit(block.children, depth + 1, block.id);
    }
  };
  visit(blocks, 0);
  return index;
}

function subtreeHeight(block: BlockTreeNode): number {
  return block.children.filter(child => !isAttachedTextBlock(child)).reduce((height, child) => Math.max(height, 1 + subtreeHeight(child)), 0);
}

/** Resolve the displayed gap against the document with the moving roots
 * removed. A canonical B-after/C-before gap may be anchored on selected C;
 * the final command must use a surviving sibling, never the removed ID.
 */
function resolveMove(blocks: readonly BlockTreeNode[], sourceIds: readonly string[], target: BlockPlacement, policy: BlockMovePolicy): ResolvedMove | undefined {
  const index = indexBlocks(blocks);
  if (sourceIds.length === 0 || sourceIds.some((id) => !index.has(id))) return undefined;
  const roots = getSelectedBlockRootIds(blocks, sourceIds);
  const selected = new Set(roots);
  const destination = index.get(target.blockId);
  if (destination && isAttachedTextBlock(destination.block)) {
    return destination.parentId && target.placement !== "inside"
      ? resolveMove(blocks, sourceIds, {blockId: destination.parentId, placement: "after"}, policy) : undefined;
  }
  if (destination === undefined || isFixedBlock(destination.block) && target.placement !== "after" || roots.length === 0
    || sourceIds.some(id => isFixedBlock(index.get(id)!.block))) return undefined;
  // A selected root's before/after gap can be valid for a disjoint selection.
  // Its descendants cannot: their destination parent would itself be removed.
  let ancestor: BlockLocation | undefined = destination;
  while (ancestor !== undefined) {
    if (selected.has(ancestor.block.id) && (ancestor !== destination || target.placement === "inside")) return undefined;
    ancestor = ancestor.parentId === undefined ? undefined : index.get(ancestor.parentId);
  }
  if (target.placement === "inside" && policy.canContain?.(destination.block) === false) return undefined;
  const depth = destination.depth + (target.placement === "inside" ? 1 : 0);
  const sources = roots.map((id) => index.get(id)).filter((location) => location !== undefined);
  if (sources.some(({ block }) => depth + subtreeHeight(block) > (policy.maxDepth ?? Infinity)
    || (depth > 0 && policy.canNest?.(block) === false))) return undefined;

  const siblings = target.placement === "inside" ? destination.block.children : destination.siblings;
  const remaining = siblings.filter((block) => !selected.has(block.id)).map((block) => block.id);
  const boundary = siblings.findIndex((block) => block.id === target.blockId) + (target.placement === "after" ? 1 : 0);
  const insertAt = target.placement === "inside" ? remaining.length
    : siblings.slice(0, boundary).filter((block) => !selected.has(block.id)).length;
  if (sources.every((source) => source.siblings === siblings)) {
    const reordered = [...remaining.slice(0, insertAt), ...roots, ...remaining.slice(insertAt)];
    if (reordered.every((id, position) => id === siblings[position]?.id)) return undefined;
  }
  if (target.placement === "inside") return { roots, target };
  const next = remaining[insertAt];
  if (next !== undefined) return { roots, target: { blockId: next, placement: "before" } };
  const previous = remaining[insertAt - 1];
  if (previous !== undefined) return { roots, target: { blockId: previous, placement: "after" } };
  // Moving a child together with a root from elsewhere can empty this sibling
  // group. Its surviving parent is the supported anchor for recreating it.
  const parent = destination.parentId === undefined ? undefined : index.get(destination.parentId);
  return parent === undefined || policy.canContain?.(parent.block) === false ? undefined
    : { roots, target: { blockId: parent.block.id, placement: "inside" } };
}

/** Validate all moving roots before feedback or mutation. Selected descendants
 * are not removed twice; optional host constraints apply to the whole move.
 */
export function canMoveBlocks(blocks: readonly BlockTreeNode[], sourceIds: readonly string[], target: BlockPlacement, policy: BlockMovePolicy = {}): boolean {
  return resolveMove(blocks, sourceIds, target, policy) !== undefined;
}

/** Structural selections move as the same ordered group as a block drag.
 * Use the first selected root's sibling/parent as the destination; unselected
 * gaps stay in place and existing selected subtrees are never flattened. */
export function indentSelectedBlocks<Block extends BlockTreeNode>(editor: BlockMoveEditor<Block>, sourceIds: readonly string[], direction: "indent" | "outdent", policy: BlockMovePolicy = {}): boolean {
  const roots = getSelectedBlockRootIds(editor.document, sourceIds);
  const first = roots[0];
  const location = first === undefined ? undefined : indexBlocks(editor.document).get(first);
  if (!location) return false;
  const previous = location.siblings[location.siblings.findIndex(block => block.id === first) - 1];
  const target: BlockPlacement | undefined = direction === "indent"
    ? previous && {blockId: previous.id, placement: "inside"}
    : location.parentId === undefined ? undefined : {blockId: location.parentId, placement: "after"};
  return target !== undefined && moveBlocks(editor, roots, target, policy);
}

/** Move by identity, independent of a native selection changed during the gesture.
 * AppFlowy/SiYuan inform gesture → validated placement → atomic editor command;
 * this adapter uses only BlockNote 0.52.1 public APIs, not their implementations.
 */
export function moveBlocks<Block extends BlockTreeNode>(editor: BlockMoveEditor<Block>, sourceIds: readonly string[], target: BlockPlacement, policy: BlockMovePolicy = {}): boolean {
  const resolved = resolveMove(editor.document, sourceIds, target, policy);
  if (resolved === undefined) return false;
  const { roots, target: placement } = resolved;
  const sources = roots.map((id) => editor.getBlock(id)).filter((block) => block !== undefined);
  const first = sources[0];
  if (first === undefined || sources.length !== roots.length) return false;
  editor.transact(() => {
    editor.removeBlocks([...roots]);
    if (placement.placement === "inside") {
      // Re-read after removal: the destination can be an ancestor of a source,
      // so its old children snapshot could otherwise duplicate the moved ID.
      const destination = editor.getBlock(placement.blockId);
      if (destination === undefined) throw new Error("Validated drop target disappeared during move");
      const lastChild = destination.children.at(-1);
      if (lastChild === undefined) editor.updateBlock(placement.blockId, { children: sources });
      else editor.insertBlocks(sources, lastChild.id, "after");
    } else {
      editor.insertBlocks(sources, placement.blockId, placement.placement);
    }
    editor.setTextCursorPosition(first.id, "start");
  });
  // Selection and DOM focus are separate public APIs. The handle prevents
  // native focus on pointerdown, so subsequent typing/undo must be handed back
  // to the editor after a successful move, not to a previously focused field.
  editor.focus();
  return true;
}
