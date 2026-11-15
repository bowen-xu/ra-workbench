// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { getOriginalDoc } from "@codemirror/merge";
import { undo } from "@codemirror/commands";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TextEditor, type TextEditorProps } from "./TextEditor";

let container: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); container = document.createElement("div"); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
const props = { documentId: "test", path: "test.txt", value: "new", baseline: "old", onChange: () => {} };
async function render(overrides: Partial<TextEditorProps> = {}): Promise<void> { await act(async () => root.render(<TextEditor {...props} {...overrides} />)); }
function view(): EditorView { return EditorView.findFromDOM(container.querySelector<HTMLElement>(".cm-editor")!)!; }
function keep(): HTMLButtonElement { return container.querySelector<HTMLButtonElement>(".cm-content .writing-diff-chunk-keep")!; }

it("plans a controlled inline review without mutating editor state before host acknowledgement", async () => {
  const onReview = vi.fn(); await render({ onReview });
  await act(async () => keep().click());
  expect(onReview).toHaveBeenCalledWith("new", "new");
  expect(getOriginalDoc(view().state).toString()).toBe("old");
  expect(keep()).not.toBeNull();
  await render({ onReview, baseline: "new" }); expect(getOriginalDoc(view().state).toString()).toBe("new");
});
it("retains default inline review and immediate save behavior for existing callers", async () => {
  const onSave = vi.fn(); await render({ onSave });
  await act(async () => keep().click());
  expect(getOriginalDoc(view().state).toString()).toBe("new"); expect(onSave).toHaveBeenCalledTimes(1);
});
it("updates inline review locking without replacing the editable view", async () => {
  await render(); const mounted = view();
  await render({ reviewDisabled: true }); expect(view()).toBe(mounted); expect(keep().disabled).toBe(true);
  expect(view().contentDOM.contentEditable).toBe("true");
  await render(); expect(view()).toBe(mounted); expect(keep().disabled).toBe(false);
});
it("changes the injected highlight without replacing native input, selection or undo history", async () => {
  await render({ reviewConfiguration: { highlight: { addedUnderline: "#1266cc" } } });
  const mounted = view();
  await act(async () => mounted.dispatch({ changes: { from: 3, insert: "!" }, selection: { anchor: 4 }, userEvent: "input.type" }));
  await render({ reviewConfiguration: { highlight: { addedUnderline: "#8833bb", removedText: "#ffaabb" } } });
  expect(view()).toBe(mounted);
  expect(mounted.state.doc.toString()).toBe("new!");
  expect(mounted.state.selection.main.head).toBe(4);
  expect(getOriginalDoc(mounted.state).toString()).toBe("old");
  expect(container.querySelector<HTMLElement>(".editor-diff-highlight")?.style.getPropertyValue("--editor-diff-added-underline")).toBe("#8833bb");
  await act(async () => { expect(undo(mounted)).toBe(true); });
  expect(mounted.state.doc.toString()).toBe("new");
});
it("stages incoming-first Both with original CRLF bytes without mutating the controlled document", async () => {
  const onIncomingResolve = vi.fn();
  await render({ value: "mine\r\n", baseline: "old\r\n", incoming: [{ id: "one", from: 0, to: 4, text: "new", document: "new\r\n" }], onIncomingResolve });
  const both = [...container.querySelectorAll<HTMLButtonElement>(".cm-content button")].find(button => button.textContent === "Both")!;
  await act(async () => both.click());
  expect(onIncomingResolve).toHaveBeenCalledWith({ ids: ["one"], action: "both", value: "new\r\nmine\r\n", baseline: "old\r\n" });
  expect(view().state.doc.toString()).toBe("mine\r\n");
  expect(container.querySelector(".text-editor-incoming")?.getAttribute("contenteditable")).toBe("false");
});
it("mounts React controls inside the document flow without changing document text", async () => {
  const select = vi.fn();
  await render({ inlinePanels: [{ id: "conflict", at: 2, content: <button type="button" onClick={select}>选择冲突</button> }] });
  const button = container.querySelector<HTMLButtonElement>(".cm-content .text-editor-inline-panel button")!;
  expect(button).not.toBeNull(); await act(async () => button.click()); expect(select).toHaveBeenCalledOnce();
  expect(view().state.doc.toString()).toBe("new");
});

it("routes file actions through injected ports in an isolated host", async () => {
  const reveal = vi.fn(async () => {}), saveAs = vi.fn(async (_content: string) => {});
  await render({ fileActions: { reveal, saveAs } });
  const open = () => container.querySelector<HTMLButtonElement>(".text-editor-file-menu-trigger")!.click();
  const item = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === label)!;
  await act(async () => open());
  await act(async () => item("在 Finder 中显示").click()); expect(reveal).toHaveBeenCalledOnce();
  await act(async () => open());
  await act(async () => item("另存为…").click()); expect(saveAs).toHaveBeenCalledWith("new");
});
