import { MAX_NOTE_DEPTH } from "@research-assistant/core/notes";
import type { BlockMovePolicy, BlockTreeNode } from "../../../components/block-editor";

function containsChildPage(block: BlockTreeNode): boolean {
  return block.type === "childPage" || block.children.some(containsChildPage);
}

/** Note Page aggregation is a host invariant, not a generic editor constraint. */
export const noteBlockMovePolicy: BlockMovePolicy = {
  maxDepth: MAX_NOTE_DEPTH,
  canContain: (block) => block.type !== "childPage",
  canNest: (block) => !containsChildPage(block),
};
