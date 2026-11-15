import { isolateHistory } from "@codemirror/commands";
import { diff, getChunks, getOriginalDoc, originalDocChangeEffect } from "@codemirror/merge";
import { ChangeSet, Text, Transaction, type EditorState, type TransactionSpec } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { resolveReviewDocument, type ReviewDocumentAdapter } from "@research-assistant/core/document-review";

interface TextRange { readonly from: number; readonly to: number }
const textReviewAdapter: ReviewDocumentAdapter<Text, TextRange> = {
  equal: (a, b) => a.eq(b),
  replace(target, targetRange, source, sourceRange) {
    // Merge ranges include the terminating line break, including EOF's virtual
    // break. Explicit Text slices preserve CRLF bytes.
    let insert = source.slice(sourceRange.from, Math.max(sourceRange.from, sourceRange.to - 1));
    if (sourceRange.from !== sourceRange.to && targetRange.to <= target.length) insert = insert.append(Text.of(["", ""]));
    return ChangeSet.of({ from: targetRange.from, to: Math.min(target.length, targetRange.to), insert }, target.length).apply(target);
  },
};

export function textChanges(before: string, after: string): ChangeSet {
  return ChangeSet.of(diff(before, after, { scanLimit: 1000, timeout: 50 }).map((change) => ({
    from: change.fromA, to: change.toA,
    insert: Text.of(after.slice(change.fromB, change.toB).split("\n")),
  })), before.length);
}

export function resolveTextChunk(view: EditorView, position: number, action: "keep" | "undo"): boolean {
  if (view.state.readOnly || view.composing) return false;
  const transaction = textChunkResolution(view.state, position, action);
  if (!transaction) return false;
  view.dispatch(transaction);
  return true;
}

/** Pure transaction planning lets controlled hosts persist before installing a review. */
export function textChunkResolution(state: EditorState, position: number, action: "keep" | "undo"): TransactionSpec | null {
  if (state.readOnly) return null;
  const chunk = getChunks(state)?.chunks.find((candidate) => candidate.fromB <= position && position <= candidate.endB);
  if (!chunk) return null;
  const original = getOriginalDoc(state);
  const next = resolveReviewDocument({ content: state.doc, reviewBaseline: original }, action, textReviewAdapter,
    { baseline: { from: chunk.fromA, to: chunk.toA }, current: { from: chunk.fromB, to: chunk.toB } });
  const changes = action === "keep" ? textChanges(original.toString(), next.reviewBaseline.toString())
    : textChanges(state.doc.toString(), next.content.toString());
  return {
    ...(action === "keep" ? { effects: originalDocChangeEffect(state, changes) } : { changes }),
    annotations: [isolateHistory.of("full"), Transaction.userEvent.of(`review.${action}`)],
  };
}

/** Resolve the entire current file in one transaction, including baseline-only
 * confirmation. The reversible original-document effect shares CM history.
 */
export function resolveTextDiff(view: EditorView, action: "keep" | "undo"): boolean {
  if (view.composing) return false;
  const transaction = textDiffResolution(view.state, action);
  if (!transaction) return false;
  view.dispatch(transaction);
  return true;
}

/** The same command also works on a file without an attached view. */
export function textDiffResolution(state: EditorState, action: "keep" | "undo"): TransactionSpec | null {
  if (state.readOnly) return null;
  const original = getOriginalDoc(state).toString();
  const current = state.doc.toString();
  if (original === current) return null;
  const next = resolveReviewDocument({ content: state.doc, reviewBaseline: getOriginalDoc(state) }, action, textReviewAdapter);
  return {
    ...(action === "keep"
      ? { effects: originalDocChangeEffect(state, textChanges(original, next.reviewBaseline.toString())) }
      : { changes: textChanges(current, next.content.toString()) }),
    annotations: [isolateHistory.of("full"), Transaction.userEvent.of(`review.${action}`)],
  };
}
