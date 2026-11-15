import type { DocumentChange, DocumentPath, DocumentReviewFragment, DocumentReviewProjector } from "@research-assistant/core/hbs";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import { blockForPath, pathOverlaps } from "./noteUnits";

export interface NoteReviewLocation {
  readonly side: "before" | "after";
  readonly key: string;
  readonly jump: () => void;
}

/** Both endpoints share a presentation key while retaining the original
 * decision identity. Navigation only scrolls/focuses the other action host. */
export function createNoteReviewLocation(root: HTMLElement, decisionId: string, fragment: DocumentReviewFragment,
  side: NoteReviewLocation["side"]): NoteReviewLocation {
  const key = JSON.stringify([decisionId, fragment.beforePath, fragment.path]);
  return { side, key, jump: () => {
    const targetSide = side === "before" ? "after" : "before";
    const target = [...root.querySelectorAll<HTMLElement>("[data-hbs-review-location]")]
      .find(element => element.dataset.hbsReviewLocation === key && element.dataset.hbsReviewSide === targetSide);
    if (!target) return;
    target.scrollIntoView({ block: "nearest" });
    target.querySelector<HTMLButtonElement>("[data-review-location-jump]")?.focus({ preventScroll: true });
  } };
}

/** A collection's placement decision and its member's content decision may
 * share a path prefix while owning independent dimensions. A content conflict
 * must not hide the compatible placement review (or the reverse). */
export function noteReviewConflictCovers(conflict: DocumentChange, change: DocumentChange, project: DocumentReviewProjector): boolean {
  if (!pathOverlaps(conflict.path, change.path)) return false;
  const placementOnly = (value: DocumentChange) => {
    let fragments = project(value).fragments;
    // A conflict can retain an unchanged current branch. Its incoming side
    // still identifies the dimension; an empty current projection cannot turn
    // an incoming placement conflict into a content ancestor conflict.
    if (!fragments.length && "incoming" in value) fragments = project({ ...value, after: value.incoming }).fragments;
    return fragments.length > 0 && fragments.every(fragment => fragment.positionChanged && !fragment.contentChanged);
  };
  return placementOnly(conflict) === placementOnly(change);
}

/** A sibling boundary inside a real BlockNote collection. An absent successor
 * means that collection's end, never the end of the whole note by default. */
export interface NoteReviewAnchor { readonly parentPath: DocumentPath; readonly beforeId?: string }

function siblings(document: NoteDocumentV1, path: DocumentPath): readonly NoteBlockV1[] | undefined {
  if (path.length === 1 && path[0] === "blocks") return document.blocks;
  if (path.at(-1) === "children") return blockForPath(document, path)?.children;
  return undefined;
}

function oldMember(fragment: DocumentReviewFragment): { readonly parentPath: DocumentPath; readonly id: string } | undefined {
  const path = fragment.beforePath;
  if (!path) return;
  for (let index = path.length - 1; index >= 0; index--) {
    const part = path[index];
    if (part && typeof part === "object" && part.key === "id") return { parentPath: path.slice(0, index), id: part.value };
  }
  return undefined;
}

/** Order old presentation at a shared native boundary by its source siblings,
 * independently of current order and independently of decision ownership.
 * A text fragment can borrow its collection's placement from a sibling review. */
export function noteReviewOldPosition(fragment: DocumentReviewFragment, fragments: readonly DocumentReviewFragment[]): number | undefined {
  const member = oldMember(fragment);
  if (!member) return;
  const sameParent = (candidate: DocumentReviewFragment) => JSON.stringify(oldMember(candidate)?.parentPath) === JSON.stringify(member.parentPath);
  const order = fragment.placement?.before ?? fragments.find(candidate => sameParent(candidate)
    && candidate.placement?.before.includes(member.id))?.placement?.before;
  const index = order?.indexOf(member.id);
  return index === undefined || index < 0 ? undefined : index;
}

/** Place an old fragment relative to stable siblings. Positions are local to
 * the collection; no global block search or reused array-index identity. */
export function noteReviewOldAnchor(fragment: DocumentReviewFragment, document: NoteDocumentV1,
  fragments: readonly DocumentReviewFragment[]): NoteReviewAnchor | undefined {
  const path = fragment.beforePath, identity = path?.at(-1);
  if (!path || !identity || typeof identity !== "object" || identity.key !== "id") return;
  const parentPath = path.slice(0, -1), current = siblings(document, parentPath);
  if (!current) return;
  const moved = new Set(fragments.filter(item => item.positionChanged
    && JSON.stringify(item.beforePath?.slice(0, -1)) === JSON.stringify(parentPath))
    .map(item => item.beforePath?.at(-1)).flatMap(item => item && typeof item === "object" ? [item.value] : []));
  const order = fragment.placement?.before ?? [];
  const index = order.indexOf(identity.value);
  const beforeId = index < 0 ? undefined : order.slice(index + 1).find(id => !moved.has(id) && current.some(block => block.id === id));
  return { parentPath, ...(beforeId ? { beforeId } : {}) };
}
