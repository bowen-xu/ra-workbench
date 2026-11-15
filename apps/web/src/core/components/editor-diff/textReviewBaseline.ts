import { getChunks, getOriginalDoc, originalDocChangeEffect, updateOriginalDoc } from "@codemirror/merge";
import { ChangeSet, EditorState, Facet, type ChangeSpec } from "@codemirror/state";
import { invertedEffects, isolateHistory } from "@codemirror/commands";
import { shouldSynchronizeReviewBaseline, type ReviewOrigin } from "@research-assistant/core/document-review";

/** Controlled ACKs can change B′ after a history entry was recorded. Such a
 * host replays baseline deltas itself, instead of installing stale full docs. */
export const hostReviewHistory = Facet.define<boolean, boolean>({ combine: values => values.some(Boolean) });

/** D and B change together for ordinary editing outside pending chunks. Inside
 * a chunk only D changes. History replays the recorded inverse baseline effect;
 * it must not infer a new baseline from the undo transaction's content changes.
 */
export const textReviewBaseline = [
  invertedEffects.of((transaction) => {
    if (transaction.startState.facet(hostReviewHistory)) return [];
    const effect = transaction.effects.find((candidate) => candidate.is(updateOriginalDoc));
    if (!effect) return [];
    const original = getOriginalDoc(transaction.startState);
    return [updateOriginalDoc.of({ doc: original, changes: effect.value.changes.invert(original) })];
  }),
  EditorState.transactionExtender.of((transaction) => {
    if (transaction.isUserEvent("accept") || transaction.isUserEvent("revert")) {
      return { annotations: isolateHistory.of("full") };
    }
    const origin: ReviewOrigin = transaction.isUserEvent("input.agent") ? "agent"
      : (["review", "undo", "redo", "restore"] as const).find(event => transaction.isUserEvent(event)) ?? "manual";
    if (!shouldSynchronizeReviewBaseline(origin, transaction.docChanged,
      transaction.effects.some(effect => effect.is(updateOriginalDoc)))) return null;
    const original = getOriginalDoc(transaction.startState);
    const chunks = getChunks(transaction.startState)?.chunks ?? [];
    const length = transaction.startState.doc.length;
    const originalChanges: ChangeSpec[] = [];
    const map = (position: number, side: "start" | "end"): number => {
      let delta = 0;
      for (const chunk of chunks) {
        if (position < chunk.fromB || position === chunk.fromB && side === "end") break;
        if (position < chunk.toB) return Math.min(original.length, side === "end" ? chunk.fromA : chunk.toA);
        delta += (chunk.toB - chunk.fromB) - (chunk.toA - chunk.fromA);
      }
      return Math.min(original.length, Math.max(0, position - delta));
    };
    transaction.changes.iterChanges((from, to, _newFrom, _newTo, insert) => {
      // Split a cross-chunk deletion into unchanged segments. Deleting current
      // text can never delete the read-only old side of a pending chunk.
      let cursor = from;
      const remove = (start: number, end: number): void => {
        if (start < end) originalChanges.push({ from: map(start, "start"), to: map(end, "end") });
      };
      for (const chunk of chunks) {
        const end = Math.min(length, chunk.toB);
        if (end <= cursor || chunk.fromB >= to) continue;
        remove(cursor, Math.min(to, chunk.fromB));
        cursor = Math.max(cursor, end);
      }
      remove(cursor, to);
      const inside = chunks.some((chunk) => from >= chunk.fromB && from <= chunk.endB);
      if (insert.length && !inside) originalChanges.push({ from: map(from, "start"), insert });
    });
    if (!originalChanges.length) return null;
    return { effects: originalDocChangeEffect(transaction.startState, ChangeSet.of(originalChanges, original.length)) };
  }),
];
