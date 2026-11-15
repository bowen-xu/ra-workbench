import { noteBlockId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";

/** Fresh identities prevent collisions from moves and Keep both. A repeated or
 * no longer owned child-page link becomes an ordinary reference, not a second
 * hierarchy edge. No rich content or managed-asset identity is flattened. */
export function prepareConflictBlocks(blocks: readonly NoteBlockV1[], saved: NoteDocumentV1, copy = false): readonly NoteBlockV1[] {
  const owned = new Set<string>();
  const collect = (items: readonly NoteBlockV1[]) => items.forEach(block => {
    if (block.type === "childPage" && block.content.kind === "reference") owned.add(block.content.targetId);
    collect(block.children);
  });
  collect(saved.blocks);
  const ids = new Set<string>(), children = new Set<string>();
  const walk = (items: readonly NoteBlockV1[]): readonly NoteBlockV1[] => items.map(block => {
    const id = copy || ids.has(block.id) ? noteBlockId(crypto.randomUUID()) : block.id;
    ids.add(id);
    const child = block.type === "childPage" && block.content.kind === "reference" ? block.content.targetId : undefined;
    const reference = child !== undefined && (copy || !owned.has(child) || children.has(child));
    if (child !== undefined) children.add(child);
    return { ...block, id, ...(reference ? { type: "researchObjectReference" as const } : {}), children: walk(block.children) };
  });
  return walk(blocks);
}
