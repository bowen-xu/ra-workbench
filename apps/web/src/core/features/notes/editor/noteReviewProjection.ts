import { docToBlocks } from "@blocknote/core";
import type { Node } from "@tiptap/pm/model";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import { blockNoteToNoteDocument, noteDocumentToBlockNote, retainNoteProperties } from "./blockNoteAdapter";
import { noteEditorSchema, type NoteEditorBlock } from "./noteEditorSchema";

function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => {
    // ProseMirror omits zero-length text nodes. Compare their visual
    // projection without changing the original document or stored content.
    if (Array.isArray(item)) return item.filter(part => !(part && typeof part === "object" && part.type === "text" && part.text === ""));
    return item && typeof item === "object"
      ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item;
  });
}

/** Compare the editor's projection, including known schema defaults. Hidden
 * native props never become differences solely because the editor omits them. */
export function sameNoteProjection(a: NoteDocumentV1, b: NoteDocumentV1): boolean {
  const normalize = (document: NoteDocumentV1) => {
    const walk = (blocks: ReturnType<typeof noteDocumentToBlockNote>): unknown => blocks.map(block => {
      const schema = noteEditorSchema.blockSchema[block.type as keyof typeof noteEditorSchema.blockSchema]?.propSchema as Record<string, { default: unknown }> | undefined;
      return { ...block, props: Object.fromEntries(Object.entries(schema ?? {}).map(([key, rule]) => [key, (block.props as Record<string, unknown> | undefined)?.[key] ?? rule.default])),
        children: walk(block.children as typeof blocks ?? []) };
    });
    return canonical(walk(noteDocumentToBlockNote(document)));
  };
  return a.pageId === b.pageId && a.title === b.title && normalize(a) === normalize(b);
}

export function noteReviewDocumentFromState(pageId: NoteDocumentV1["pageId"], node: Node, trusted: NoteDocumentV1): NoteDocumentV1 {
  const children = (blocks: readonly NoteBlockV1[]): { blockId: string; targetId: string }[] => blocks.flatMap(block => [
    ...(block.type === "childPage" && block.content.kind === "reference" ? [{ blockId: block.id, targetId: block.content.targetId }] : []), ...children(block.children),
  ]);
  return retainNoteProperties(blockNoteToNoteDocument(pageId, docToBlocks(node) as NoteEditorBlock[], { validChildPages: children(trusted.blocks) }),trusted,noteEditorSchema.blockSchema);
}

/** Ordinary roots follow user input in B and D. Pending roots keep their old
 * B, including deleted roots. Native resolution validates the durable result. */
export function synchronizeNoteReviewBaseline(baseline: NoteDocumentV1, before: NoteDocumentV1, after: NoteDocumentV1, pendingIds: readonly string[] = []): NoteDocumentV1 {
  const b = new Map(baseline.blocks.map(block => [block.id, block]));
  const old = new Map(before.blocks.map(block => [block.id, block]));
  const pending = new Set(baseline.blocks.filter(block => {
    const current = old.get(block.id);
    return !current || !sameNoteProjection({ ...baseline, blocks: [block] }, { ...before, blocks: [current] });
  }).map(block => block.id));
  pendingIds.forEach(id => pending.add(id as NoteBlockV1["id"]));
  // A move's removed/inserted fragments share one identity. Preserve the
  // original order in B even when every moved block's content is unchanged.
  const positions = new Map(before.blocks.map((block, index) => [block.id, index]));
  let highest = -1;
  baseline.blocks.forEach((block, index) => {
    const position = positions.get(block.id); if (position === undefined) return;
    if (position < highest) for (const prior of baseline.blocks.slice(0, index + 1)) {
      const priorPosition = positions.get(prior.id); if (priorPosition !== undefined && priorPosition >= position) pending.add(prior.id);
    }
    highest = Math.max(highest, position);
  });
  const insertedPending = new Set(before.blocks.filter(block => !b.has(block.id)).map(block => block.id));
  const preserve = (original: NoteBlockV1, edited: NoteBlockV1): NoteBlockV1 => {
    const descendants = new Map(original.children.map(child => [child.id, child]));
    return { ...edited, props: { ...original.props, ...edited.props }, children: edited.children.map(child => {
      const prior = descendants.get(child.id); return prior ? preserve(prior, child) : child;
    }) };
  };
  const edited = new Map(after.blocks.map(block => [block.id, block]));
  const blocks = baseline.blocks.filter(block => pending.has(block.id) || !old.has(block.id) || edited.has(block.id));
  // Mirror native review synchronization. Start with B's pending identities
  // in their original order and place ordinary/manual roots against the next
  // surviving identity, including a pending root. Appending old roots after
  // new manual blocks would look like a move and duplicate them in old Diff.
  for (let index = after.blocks.length - 1; index >= 0; index--) {
    const block = after.blocks[index]!;
    if (pending.has(block.id) || insertedPending.has(block.id)) continue;
    const original = b.get(block.id), current = blocks.findIndex(item => item.id === block.id);
    if (current >= 0) blocks.splice(current, 1);
    const next = after.blocks.slice(index + 1).find(item => blocks.some(candidate => candidate.id === item.id));
    const position = next ? blocks.findIndex(item => item.id === next.id) : blocks.length;
    blocks.splice(position, 0, original ? preserve(original, block) : block);
  }
  return { ...baseline, blocks };
}
