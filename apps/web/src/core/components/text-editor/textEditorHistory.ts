import { invertedEffects } from "@codemirror/commands";
import { StateEffect } from "@codemirror/state";
import { getOriginalDoc, updateOriginalDoc } from "@codemirror/merge";

export interface EditorHistoryPoint {
  readonly value: string;
  readonly baseline: string;
  readonly restore: (value: string, baseline: string) => void;
}
export interface EditorHistoryAction { readonly before: EditorHistoryPoint; readonly after: EditorHistoryPoint }
export interface TextEditorHistoryPort {
  readonly perform: (action: () => void, review?: { readonly value: string; readonly baseline: string }) => EditorHistoryAction | undefined;
  readonly restoreEdit?: (value: string, changes: readonly BaselineHistoryChange[]) => void;
}
export interface BaselineHistoryChange { readonly before: string; readonly after: string }
export const baselineHistoryChange = StateEffect.define<BaselineHistoryChange>();

/** Domain reconciliation context travels with the same CM history event as its
 * text/baseline change. No separate undo stack or transport snapshot is replayed. */
export const editorHistoryAction = StateEffect.define<EditorHistoryAction>();
export const editorActionHistory = invertedEffects.of(transaction => {
  const actions = transaction.effects.filter(effect => effect.is(editorHistoryAction));
  if (actions.length) return actions.map(effect => editorHistoryAction.of({ before: effect.value.after, after: effect.value.before }));
  const replay = transaction.effects.filter(effect => effect.is(baselineHistoryChange));
  // CM combines typing events newest-first when undoing. Redo must replay the
  // inverse baseline deltas in the opposite order, preserving later remote B.
  if (replay.length) return replay.reverse().map(effect => baselineHistoryChange.of({ before: effect.value.after, after: effect.value.before }));
  const baseline = transaction.effects.find(effect => effect.is(updateOriginalDoc));
  if (!baseline) return [];
  const before = getOriginalDoc(transaction.startState).toString(), after = baseline.value.doc.toString();
  return before === after ? [] : [baselineHistoryChange.of({ before: after, after: before })];
});
