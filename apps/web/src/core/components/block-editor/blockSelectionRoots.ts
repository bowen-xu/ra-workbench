import {isFixedBlock} from "./blockEditingConstraints";

export interface BlockTreeNode {
  readonly id: string;
  readonly type?: string;
  readonly children: readonly BlockTreeNode[];
}

/** A selected parent already owns its descendants. Return each moving subtree
 * once, in document order rather than pointer travel or selection order.
 */
export function getSelectedBlockRootIds(blocks: readonly BlockTreeNode[], selectedIds: readonly string[]): string[] {
  const selected = new Set(selectedIds);
  const roots: string[] = [];
  const visit = (siblings: readonly BlockTreeNode[]): void => {
    for (const block of siblings) {
      if (isFixedBlock(block)) continue;
      if (selected.has(block.id)) roots.push(block.id);
      else visit(block.children);
    }
  };
  visit(blocks);
  return roots;
}
