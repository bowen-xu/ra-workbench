import type { EditorState } from "@tiptap/pm/state";
import type { DocumentChange, DocumentConflict, DocumentPath, DocumentReviewFragment, DocumentReviewProjection, HbsSessionState } from "@research-assistant/core/hbs";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import { noteUnitRanges } from "./noteReviewRanges";
import { blockForPath } from "./noteUnits";
import { noteReviewOldAnchor } from "./noteReviewProjection";

interface Interval { readonly from: number; readonly to: number }
interface BlockSpan {
  readonly parent: string;
  readonly before: Interval;
  readonly after: Interval;
}
export interface NoteReviewUnit {
  readonly members: readonly DocumentChange[];
  readonly decision: DocumentChange;
  readonly conflicts: readonly DocumentConflict[];
  readonly projection: DocumentReviewProjection;
}
export interface NoteReviewGroup {
  readonly members: readonly DocumentChange[];
  readonly firstUnit: number;
  readonly lastUnit: number;
  readonly number: number;
}

function member(document: NoteDocumentV1, path: DocumentPath): { parent: string; interval: Interval } | undefined {
  let index = path.length - 1;
  for (; index >= 0; index--) {
    const part = path[index];
    if (typeof part === "object" && part.key === "id") break;
  }
  const identity = path[index];
  if (!identity || typeof identity !== "object") return;
  const parentPath = path.slice(0, index);
  let siblings: readonly NoteBlockV1[] | undefined;
  if (parentPath.length === 1 && parentPath[0] === "blocks") siblings = document.blocks;
  else if (parentPath.at(-1) === "children") siblings = blockForPath(document, parentPath)?.children;
  const position = siblings?.findIndex(block => block.id === identity.value);
  if (position === undefined || position < 0) return;
  return { parent: JSON.stringify(parentPath), interval: { from: position, to: position + 1 } };
}

function joinInterval(left: Interval, right: Interval): Interval {
  return { from: Math.min(left.from, right.from), to: Math.max(left.to, right.to) };
}
function separated(left: Interval, right: Interval): boolean {
  return left.to < right.from || right.to < left.from;
}
function joinSpan(left: BlockSpan, right: BlockSpan): BlockSpan | undefined {
  if (left.parent !== right.parent || separated(left.before, right.before) || separated(left.after, right.after)) return;
  return { parent: left.parent, before: joinInterval(left.before, right.before), after: joinInterval(left.after, right.after) };
}

function reverseFragment(fragment: DocumentReviewFragment): DocumentReviewFragment {
  const { beforePath, path, before, after, placement, ...flags } = fragment;
  return { ...flags, ...(path ? { beforePath: path } : {}), ...(beforePath ? { path: beforePath } : {}), before: after, after: before,
    ...(placement ? { placement: { before: placement.after, after: placement.before } } : {}) };
}

function gap(document: NoteDocumentV1, fragment: DocumentReviewFragment,
  fragments: readonly DocumentReviewFragment[]): { parent: string; interval: Interval } | undefined {
  const anchor = noteReviewOldAnchor(fragment, document, fragments);
  if (!anchor) return;
  const siblings = anchor.parentPath.length === 1 ? document.blocks : blockForPath(document, anchor.parentPath)?.children;
  if (!siblings) return;
  const position = anchor.beforeId ? siblings.findIndex(block => block.id === anchor.beforeId) : siblings.length;
  if (position < 0) return;
  return { parent: JSON.stringify(anchor.parentPath), interval: { from: position, to: position } };
}

/** Group only reliable complete sibling blocks. Cells, location endpoints and
 * fallback scopes retain their existing action capability and authorization. */
function blockSpan(state: EditorState, value: HbsSessionState<NoteDocumentV1>, unit: NoteReviewUnit,
  topology: { before: readonly DocumentReviewFragment[]; after: readonly DocumentReviewFragment[] }): BlockSpan | undefined {
  if (unit.projection.fallbackReason || !unit.projection.fragments.length) return;
  let result: BlockSpan | undefined;
  for (const fragment of unit.projection.fragments) {
    if (fragment.positionChanged) return;
    if (fragment.path) {
      const ranges = noteUnitRanges(state, value.content, { ...unit.decision, path: fragment.path });
      if (!ranges.length || ranges.some(range => range.node.type.name !== "blockContainer")) return;
    }
    const path = fragment.path ?? fragment.beforePath;
    if (!path) return;
    const before = member(value.baselineProjection, fragment.beforePath ?? path)
      ?? gap(value.baselineProjection, reverseFragment(fragment), topology.before);
    const after = member(value.content, fragment.path ?? path) ?? gap(value.content, fragment, topology.after);
    if (!before || !after || before.parent !== after.parent) return;
    const span: BlockSpan = { parent: before.parent, before: before.interval, after: after.interval };
    if (!result) result = span;
    else {
      result = joinSpan(result, span);
      if (!result) return;
    }
  }
  return result;
}

/** Presentation groups retain every original decision. Adjacency must hold in
 * both B′ and H, so a deleted/moved unchanged sibling cannot become a bridge.
 * The returned map gives every unit its group, including the final action host. */
export function noteReviewGroups(state: EditorState, value: HbsSessionState<NoteDocumentV1>, units: readonly NoteReviewUnit[]): readonly NoteReviewGroup[] {
  const groups: { members: DocumentChange[]; firstUnit: number; lastUnit: number; number: number; conflict: boolean; span?: BlockSpan }[] = [];
  const membership: number[] = [];
  const fragments = units.flatMap(unit => unit.projection.fragments);
  const topology = { after: fragments, before: fragments.map(reverseFragment) };
  for (const [index, unit] of units.entries()) {
    const span = blockSpan(state, value, unit, topology), conflict = !!unit.conflicts.length, previous = groups.at(-1);
    const joined = previous?.span && span && previous.conflict === conflict ? joinSpan(previous.span, span) : undefined;
    if (previous && joined) {
      previous.members.push(...unit.members); previous.lastUnit = index; previous.span = joined;
    } else groups.push({ members: [...unit.members], firstUnit: index, lastUnit: index, number: groups.length + 1, conflict, ...(span ? { span } : {}) });
    membership.push(groups.length - 1);
  }
  return membership.map(index => groups[index]!);
}
