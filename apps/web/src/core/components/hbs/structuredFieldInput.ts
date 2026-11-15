import { EditorState } from "@codemirror/state";
import { getOriginalDoc, unifiedMergeView } from "@codemirror/merge";
import type { HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { textReviewBaseline } from "../editor-diff/textReviewBaseline";
import { textChanges } from "../editor-diff/textDiffCommands";
import { structuredFieldUnits, structuredTextLeaves } from "./structuredReviewUnits";

/** A card's textarea presents the same old/new text as the text editor. Use
 * its actual review chunks and transaction extender, rather than inferring
 * ordinary input from a finer structural/character merge. No view or second
 * history is created here; the host's native history records the document. */
export function projectStructuredFieldInput(adapter: HbsDocumentAdapter<unknown>, baseline: unknown,
  previous: unknown, next: unknown, fieldId: string): unknown {
  let projected = adapter.projectManual(baseline, previous, next);
  const unit = structuredFieldUnits(next).find(value => value.field.id === fieldId);
  if (!unit) return projected;
  for (const leaf of structuredTextLeaves(unit.field.value, unit.path)) {
    const before = adapter.read(previous, leaf.path), original = adapter.read(baseline, leaf.path);
    if (typeof before !== "string" || typeof original !== "string" || before === leaf.text) continue;
    const state = EditorState.create({ doc: before, extensions: [textReviewBaseline,
      unifiedMergeView({ original, mergeControls: false, allowInlineDiffs: false,
        diffConfig: { scanLimit: 1000, timeout: 50 } })] });
    const updated = state.update({ changes: textChanges(before, leaf.text), userEvent: "input.type" }).state;
    projected = adapter.replace(projected, leaf.path, getOriginalDoc(updated).toString());
  }
  return projected;
}
