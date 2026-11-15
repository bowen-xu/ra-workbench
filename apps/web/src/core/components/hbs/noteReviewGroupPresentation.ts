import type { EditorState } from "@tiptap/pm/state";
import type { DocumentChange, DocumentConflict, DocumentReviewFragment, HbsSessionState } from "@research-assistant/core/hbs";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import { noteAnchorPosition, noteUnitRanges } from "./noteReviewRanges";
import { noteReviewOldAnchor } from "./noteReviewProjection";
import type { NoteReviewGroup, NoteReviewUnit } from "./noteReviewGroups";
import { noteUnitSnapshot } from "./noteUnits";

export interface NoteReviewGroupPresentation {
  readonly key: string;
  readonly from: number;
  readonly firstCurrent: number | undefined;
  readonly before: readonly NoteBlockV1[];
  readonly after: readonly NoteBlockV1[];
  readonly incoming: readonly NoteBlockV1[] | undefined;
  readonly firstFragment: DocumentReviewFragment;
}

/** Ordering is branch-specific, not the current plan order. Keep complete
 * production blocks and original identities for the serializer's matching. */
function ordered(document: NoteDocumentV1, blocks: readonly NoteBlockV1[]): readonly NoteBlockV1[] {
  const selected = new Map(blocks.map(block => [block.id, block]));
  const result: NoteBlockV1[] = [];
  const visit = (siblings: readonly NoteBlockV1[]) => {
    for (const block of siblings) {
      const snapshot = selected.get(block.id);
      if (snapshot) { result.push(snapshot); selected.delete(block.id); }
      else visit(block.children);
    }
  };
  visit(document.blocks);
  return result;
}

/** Consolidate only groups already proven to be continuous legal siblings.
 * Native current nodes stay in the editor; only read-only widgets move. */
export function noteReviewGroupPresentation(state: EditorState, value: HbsSessionState<NoteDocumentV1>,
  group: NoteReviewGroup, plans: readonly NoteReviewUnit[],
  incomingDocument: (change: DocumentConflict, current: NoteDocumentV1) => NoteDocumentV1): NoteReviewGroupPresentation | undefined {
  if (group.firstUnit === group.lastUnit) return;
  const units = plans.slice(group.firstUnit, group.lastUnit + 1), allFragments = plans.flatMap(plan => plan.projection.fragments);
  const before: NoteBlockV1[] = [], after: NoteBlockV1[] = [], incoming: NoteBlockV1[] = [];
  const starts: number[] = [], liveStarts: number[] = [];
  const conflicts = units.flatMap(unit => unit.conflicts);
  const incomingValue = conflicts.length ? conflicts.reduce((document, conflict) => incomingDocument(conflict, document), value.content) : undefined;
  for (const unit of units) for (const fragment of unit.projection.fragments) {
    const change: DocumentChange = { ...unit.decision, path: fragment.path ?? fragment.beforePath ?? unit.decision.path,
      beforePath: fragment.beforePath ?? fragment.path ?? unit.decision.beforePath, before: fragment.before, after: fragment.after };
    if (fragment.beforePath) before.push(...noteUnitSnapshot(value.baselineProjection, change, "before"));
    if (fragment.path) {
      after.push(...noteUnitSnapshot(value.content, change, "after"));
      if (incomingValue) incoming.push(...noteUnitSnapshot(incomingValue, change, "after"));
      const ranges = noteUnitRanges(state, value.content, change);
      starts.push(...ranges.map(range => range.from)); liveStarts.push(...ranges.map(range => range.from));
    } else {
      const anchor = noteReviewOldAnchor(fragment, value.content, allFragments);
      const position = anchor ? noteAnchorPosition(state, value.content, anchor) : undefined;
      if (position !== undefined) starts.push(position);
    }
  }
  const firstFragment = units[0]?.projection.fragments[0];
  if (!starts.length || !firstFragment) return;
  return { key: JSON.stringify(group.members.map(member => member.id)), from: Math.min(...starts),
    firstCurrent: liveStarts.length ? Math.min(...liveStarts) : undefined,
    before: ordered(value.baselineProjection, before), after: ordered(value.content, after),
    incoming: incomingValue ? ordered(incomingValue, incoming) : undefined, firstFragment };
}
