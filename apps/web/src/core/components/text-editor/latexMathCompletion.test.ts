// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { CompletionContext, acceptCompletion, autocompletion, completionStatus, currentCompletions,
  nextSnippetField, setSelectedCompletion, startCompletion } from "@codemirror/autocomplete";
import { history, undo } from "@codemirror/commands";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { latexMathCompletionSource } from "./latexMathCompletion";

const views: EditorView[] = [];
afterEach(() => { for (const view of views.splice(0)) view.destroy(); document.body.replaceChildren(); });

function editor(source: string, position = source.length) {
  const view = new EditorView({ parent: document.body, state: EditorState.create({ doc: source, selection: { anchor: position },
    extensions: [history(), autocompletion({ override: [latexMathCompletionSource], interactionDelay: 0 })],
  }) });
  views.push(view); return view;
}
async function choose(view: EditorView, label: string) {
  startCompletion(view);
  await vi.waitFor(() => expect(completionStatus(view.state)).toBe("active"));
  const index = currentCompletions(view.state).findIndex(option => option.label === label);
  expect(index).toBeGreaterThanOrEqual(0);
  view.dispatch({ effects: setSelectedCompletion(index) });
  expect(acceptCompletion(view)).toBe(true);
}
it("inserts fraction fields, traverses them and undoes completion as one operation", async () => {
  const view = editor("\\fr");
  await choose(view, "frac");
  expect(view.state.doc.toString()).toBe("\\frac{}{}");
  expect(view.state.selection.main.head).toBe(6);
  expect(nextSnippetField(view)).toBe(true);
  expect(view.state.selection.main.head).toBe(8);
  expect(undo(view)).toBe(true);
  expect(view.state.doc.toString()).toBe("\\fr");
});
it.each(["{a}{b}", "[3]{x}"])("preserves existing arguments: %s", async argumentsText => {
  const view = editor(`\\fr${argumentsText}`, 3);
  await choose(view, "frac");
  expect(view.state.doc.toString()).toBe(`\\frac${argumentsText}`);
});
it("completes a mathematical symbol without adding braces", async () => {
  const view = editor("\\alp"); await choose(view, "alpha");
  expect(view.state.doc.toString()).toBe("\\alpha");
});
it.each(["x", "\\\\", "a\\\\beta"])("does not treat plain text or escaped TeX line breaks as command prefixes: %s", async source => {
  const state = EditorState.create({ doc: source });
  expect(await latexMathCompletionSource(new CompletionContext(state, source.length, false))).toBeNull();
});
