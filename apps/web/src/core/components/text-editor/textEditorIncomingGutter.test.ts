// @vitest-environment happy-dom
import { Compartment, EditorState } from "@codemirror/state";
import { unifiedMergeView } from "@codemirror/merge";
import { EditorView, lineNumbers } from "@codemirror/view";
import { afterEach, expect, it } from "vitest";
import { incomingDecorations, incomingGroups } from "./textEditorIncoming";

let view: EditorView | undefined;
afterEach(() => { view?.destroy(); view = undefined; document.body.replaceChildren(); });

it.each([
  { name: "pending review", baseline: "old", original: "cm-deletedLineGutter", current: "cm-changedLineGutter" },
  { name: "ordinary edit", baseline: "mine", original: "text-editor-conflict-original-gutter", current: "text-editor-current-conflict-gutter" },
])("uses the existing merge gutter for all conflict bands: $name", ({ baseline, original, current }) => {
  const compartment = new Compartment();
  let state = EditorState.create({ doc: "mine", extensions: [lineNumbers(), unifiedMergeView({ original: baseline,
    mergeControls: false, allowInlineDiffs: false }), compartment.of([])] });
  const groups = incomingGroups(state, [{ id: "conflict", from: 0, to: 4, text: "incoming", base: "old" }], new Map());
  state = state.update({ effects: compartment.reconfigure(incomingDecorations(state, groups)) }).state;
  view = new EditorView({ state, parent: document.body });

  const gutters = view.dom.querySelectorAll(".cm-changeGutter");
  expect(gutters).toHaveLength(1);
  for (const marker of [original, "text-editor-incoming-gutter", current]) {
    expect(gutters[0]?.querySelector(`.${marker}`)).not.toBeNull();
  }
  expect(view.dom.querySelector(".text-editor-incoming")?.getAttribute("contenteditable")).toBe("false");
  expect(view.state.doc.toString()).toBe("mine");
});
