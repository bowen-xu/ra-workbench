// @vitest-environment happy-dom
import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { completionStatus, startCompletion } from "@codemirror/autocomplete";
import { EditorView } from "@codemirror/view";
import { EquationSourceInput, type EquationSourceInputHandle } from "./EquationSourceInput";

let root: Root, host: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

async function editor(value: string) {
  const onCancel = vi.fn(), onComplete = vi.fn(), onChange = vi.fn();
  await act(async () => root.render(<EquationSourceInput inputRef={createRef<EquationSourceInputHandle>()}
    aria-label="测试公式" placeholder="x^2" value={value} onChange={onChange} onComplete={onComplete} onCancel={onCancel} />));
  const input = host.querySelector<HTMLElement>('[role="textbox"]')!;
  return { view: EditorView.findFromDOM(input)!, input, onCancel, onComplete, onChange };
}
function press(input: HTMLElement, key: string, extra: KeyboardEventInit = {}) {
  input.dispatchEvent(new KeyboardEvent("keydown", { key, keyCode: key === "Escape" ? 27 : 13, bubbles: true, cancelable: true, ...extra }));
}
it("does not let an invisible pending completion consume Escape", async () => {
  const { view, input, onCancel } = await editor("plain");
  await act(async () => { startCompletion(view); press(input, "Escape"); });
  expect(onCancel).toHaveBeenCalledOnce();
});
it("dismisses visible completion before cancelling the formula", async () => {
  const { view, input, onCancel } = await editor("\\fr");
  await act(async () => { view.dispatch({ selection: { anchor: view.state.doc.length } }); startCompletion(view); });
  await vi.waitFor(() => expect(completionStatus(view.state)).toBe("active"));
  await act(async () => press(input, "Escape"));
  expect(onCancel).not.toHaveBeenCalled();
  await act(async () => press(input, "Escape"));
  expect(onCancel).toHaveBeenCalledOnce();
});
it("leaves IME candidate confirmation to the input method", async () => {
  const { input, onCancel, onComplete } = await editor("x");
  await act(async () => {
    press(input, "Enter", { keyCode: 229, isComposing: true });
    press(input, "Escape", { keyCode: 229, isComposing: true });
  });
  expect(onCancel).not.toHaveBeenCalled(); expect(onComplete).not.toHaveBeenCalled();
});
it("retains the existing source limit without preventing deletion", async () => {
  const { view, onChange } = await editor("x".repeat(32_768));
  await act(async () => view.dispatch({ changes: { from: 32_768, insert: "y" } }));
  expect(view.state.doc.length).toBe(32_768); expect(onChange).not.toHaveBeenCalled();
  await act(async () => view.dispatch({ changes: { from: 0, to: 1 } }));
  expect(view.state.doc.length).toBe(32_767); expect(onChange).toHaveBeenCalledOnce();
});
