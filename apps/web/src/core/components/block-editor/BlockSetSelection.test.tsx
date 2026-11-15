// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { BlockNoteEditor } from "@blocknote/core";
import { Selection } from "@tiptap/pm/state";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BlockEditor } from "./BlockEditor";
import { useBlockHighlight } from "./BlockHighlight";
import { BlockSetSelection } from "./BlockSetSelection";
import type { BlockMovePolicy } from "./moveBlocks";

function engine() {
  return BlockNoteEditor.create({ initialContent: [
    { id: "first", type: "paragraph", content: "First" },
    { id: "middle", type: "paragraph", content: "Middle" },
    { id: "last", type: "paragraph", content: "Last", children: [
      { id: "child", type: "paragraph", content: "Child" },
    ] },
  ] });
}

function clipboard(type: "copy" | "cut" | "paste", values = new Map<string, string>()) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: {
    get types() { return [...values.keys()]; },
    getData: (mime: string) => values.get(mime) ?? "",
    setData: (mime: string, value: string) => values.set(mime, value),
    clearData: () => values.clear(),
  } });
  return { event, values };
}

describe("engine-owned block selection", () => {
  let host: HTMLDivElement; let root: Root; let editor: ReturnType<typeof engine>;
  let actions: ReturnType<typeof useBlockHighlight>;
  let policy: BlockMovePolicy;
  const save = vi.fn();
  function Probe(): null { actions = useBlockHighlight(); return null; }
  const render = (readOnly = false) => root.render(<BlockEditor editor={editor} onChange={save} readOnly={readOnly} movePolicy={policy}><Probe /></BlockEditor>);
  const ids = () => editor.document.map(block => block.id);
  async function select(selected: readonly string[]) { await act(async () => { actions.select(selected); editor.focus(); }); }
  async function key(key: string, options: KeyboardEventInit = {}) {
    await act(async () => editor.domElement!.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...options })));
  }
  beforeEach(async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); save.mockClear(); policy = {};
    host = document.createElement("div"); document.body.append(host); root = createRoot(host); editor = engine();
    await act(async () => render());
  });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

  it("Tab indents the whole selected group with its subtrees, preserves selection and undoes once", async () => {
    const original = JSON.stringify(editor.document);
    await select(["last", "child", "middle"]);
    await key("Tab");
    expect(ids()).toEqual(["first"]);
    expect(editor.getBlock("first")!.children.map(block => block.id)).toEqual(["middle", "last"]);
    expect(editor.getBlock("last")!.children.map(block => block.id)).toEqual(["child"]);
    expect(actions.selectedIds()).toEqual(["middle", "last"]);
    expect(save).toHaveBeenCalledOnce();
    await act(async () => { expect(editor.undo()).toBe(true); });
    expect(JSON.stringify(editor.document)).toBe(original);
    expect(actions.selectedIds()).toEqual(["middle", "last"]);
    await act(async () => { expect(editor.redo()).toBe(true); });
    expect(ids()).toEqual(["first"]);
    expect(actions.selectedIds()).toEqual(["middle", "last"]);
  });

  it("Shift+Tab outdents the selected group and each quick command remains independently undoable", async () => {
    await select(["middle", "last"]);
    await key("Tab");
    const nested = JSON.stringify(editor.document);
    await key("Tab", {shiftKey: true});
    expect(ids()).toEqual(["first", "middle", "last"]);
    expect(actions.selectedIds()).toEqual(["middle", "last"]);
    await act(async () => { expect(editor.undo()).toBe(true); });
    expect(JSON.stringify(editor.document)).toBe(nested);
    await act(async () => { expect(editor.undo()).toBe(true); });
    expect(ids()).toEqual(["first", "middle", "last"]);
  });

  it("moves disjoint roots as one ordered group without moving unselected gaps", async () => {
    await act(async () => editor.insertBlocks([{id: "tail", type: "paragraph", content: "Tail"}], "last", "after"));
    await select(["tail", "middle"]);
    await key("Tab");
    expect(ids()).toEqual(["first", "last"]);
    expect(editor.getBlock("first")!.children.map(block => block.id)).toEqual(["middle", "tail"]);
    expect(editor.getBlock("last")!.children.map(block => block.id)).toEqual(["child"]);
    expect(actions.selectedIds()).toEqual(["middle", "tail"]);
  });

  it("does not partially indent a blocked group, change a readonly document or consume an IME Tab", async () => {
    const original = JSON.stringify(editor.document);
    await select(["first", "middle"]); await key("Tab");
    expect(JSON.stringify(editor.document)).toBe(original);
    await select(["middle", "last"]);
    policy = {canNest: block => block.id !== "last"};
    await act(async () => render()); await key("Tab");
    expect(JSON.stringify(editor.document)).toBe(original);
    expect(actions.selectedIds()).toEqual(["middle", "last"]);
    policy = {};
    await act(async () => render()); await key("Tab", {isComposing: true});
    expect(JSON.stringify(editor.document)).toBe(original);
    await act(async () => render(true)); await key("Tab");
    expect(JSON.stringify(editor.document)).toBe(original);
  });

  it.each(["Backspace", "Delete"])("%s deletes the selected roots once, retains intervening blocks and restores selection on undo", async keyName => {
    const original = JSON.stringify(editor.document);
    await select(["last", "child", "first"]);
    expect(actions.selectedIds()).toEqual(["first", "last"]);
    expect(editor.prosemirrorView.state.selection).toBeInstanceOf(BlockSetSelection);
    expect(editor.prosemirrorView.state.selection.empty).toBe(false);
    expect(editor.prosemirrorView.state.selection.visible).toBe(false);
    expect(save).not.toHaveBeenCalled();
    await key(keyName);
    expect(ids()).toEqual(["middle"]);
    expect(save).toHaveBeenCalledOnce();
    expect(actions.selectedIds()).toEqual([]);
    await act(async () => { expect(editor.undo()).toBe(true); });
    expect(JSON.stringify(editor.document)).toBe(original);
    expect(actions.selectedIds()).toEqual(["first", "last"]);
    await act(async () => { expect(editor.redo()).toBe(true); });
    expect(ids()).toEqual(["middle"]);
  });

  it("deleting every block leaves one valid editable paragraph; undo restores the whole subtree", async () => {
    const original = JSON.stringify(editor.document);
    await select(["first", "middle", "last"]);
    await key("Backspace");
    expect(editor.document).toHaveLength(1);
    expect(editor.document[0]!.type).toBe("paragraph");
    expect(editor.document[0]!.content).toEqual([]);
    expect(editor.prosemirrorView.state.selection.empty).toBe(true);
    await act(async () => editor.undo());
    expect(JSON.stringify(editor.document)).toBe(original);
  });

  it("native insertion replaces exactly the structural ranges as one undoable edit", async () => {
    const original = JSON.stringify(editor.document);
    await select(["first", "last"]);
    await act(async () => {
      const view = editor.prosemirrorView;
      view.dispatch(view.state.tr.insertText("Replacement"));
    });
    expect(JSON.stringify(editor.document)).toContain("Replacement");
    expect(JSON.stringify(editor.document)).toContain("Middle");
    expect(JSON.stringify(editor.document)).not.toContain("Child");
    expect(JSON.stringify(editor.document)).not.toContain('"text":"First"');
    await act(async () => editor.undo());
    expect(JSON.stringify(editor.document)).toBe(original);
  });

  it("copy and cut use identical full subtrees, and rich paste replaces the selection", async () => {
    const original = JSON.stringify(editor.document);
    await select(["first", "last", "child"]);
    const copied = clipboard("copy"); editor.domElement!.dispatchEvent(copied.event);
    expect(JSON.stringify(editor.document)).toBe(original);
    const cut = clipboard("cut");
    await act(async () => editor.domElement!.dispatchEvent(cut.event));
    expect(cut.values).toEqual(copied.values);
    expect(cut.values.get("text/plain")).toContain("Child");
    expect(cut.values.get("text/plain")).not.toContain("Middle");
    expect(ids()).toEqual(["middle"]);
    await act(async () => editor.undo());
    await select(["middle"]);
    await act(async () => editor.domElement!.dispatchEvent(clipboard("paste", copied.values).event));
    expect(JSON.stringify(editor.document)).not.toContain("Middle");
    expect(editor.document.map(block => block.content)).toHaveLength(4);
  });

  it("plain-text paste replaces disjoint selected roots without deleting intervening content", async () => {
    await select(["first", "last"]);
    await act(async () => editor.domElement!.dispatchEvent(clipboard("paste", new Map([["text/plain", "Pasted"]])).event));
    const text = JSON.stringify(editor.document);
    expect(text).toContain("Pasted"); expect(text).toContain("Middle");
    expect(text).not.toContain('"text":"First"'); expect(text).not.toContain("Child");
  });

  it("browser beforeinput uses the same ranges and a composition starts in its own empty paragraph", async () => {
    const original = JSON.stringify(editor.document);
    await select(["first", "last"]);
    const input = new InputEvent("beforeinput", { bubbles: true, cancelable: true, inputType: "insertText", data: "Typed" });
    await act(async () => editor.domElement!.dispatchEvent(input));
    expect(input.defaultPrevented).toBe(true);
    expect(JSON.stringify(editor.document)).toContain("Typed");
    expect(JSON.stringify(editor.document)).toContain("Middle");
    await act(async () => editor.undo());
    expect(JSON.stringify(editor.document)).toBe(original);
    await act(async () => editor.domElement!.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
    expect(editor.document[0]!.content).toEqual([]);
    expect(ids()).toHaveLength(2);
    expect(ids()[1]).toBe("middle");
    expect(editor.prosemirrorView.state.selection.$from.parent.type.name).toBe("paragraph");
  });

  it("deletes nested selected roots from different levels without deleting either unselected parent", async () => {
    await act(async () => editor.updateBlock("middle", { children: [
      { id: "other-child", type: "paragraph", content: "Other child" },
      { id: "kept-child", type: "paragraph", content: "Kept child" },
    ] }));
    await select(["other-child", "child"]);
    await key("Delete");
    expect(ids()).toEqual(["first", "middle", "last"]);
    expect(editor.getBlock("middle")!.children.map(block => block.id)).toEqual(["kept-child"]);
    expect(editor.getBlock("last")!.children).toEqual([]);
    await act(async () => editor.undo());
    expect(actions.selectedIds()).toEqual(["other-child", "child"]);
  });

  it("maps by stable identity, serializes and follows normal text selection without stale highlights", async () => {
    await select(["last", "child"]);
    await act(async () => editor.updateBlock("first", { content: "Longer text before the selection" }));
    expect(actions.selectedIds()).toEqual(["last"]);
    const state = editor.prosemirrorView.state;
    expect(Selection.fromJSON(state.doc, state.selection.toJSON()).eq(state.selection)).toBe(true);
    expect(state.selection.content().content.textBetween(0, state.selection.content().content.size)).toBe("LastChild");
    await act(async () => editor.setTextCursorPosition("middle", "end"));
    expect(actions.selectedIds()).toEqual([]);
    expect(host.querySelector("[data-note-block-highlight]")).toBeNull();
  });

  it("retains surviving selected roots when undo removes a subsequently selected new block", async () => {
    await act(async () => editor.insertBlocks([{ id: "new", type: "paragraph", content: "New" }], "last", "after"));
    await select(["first", "new"]);
    await act(async () => editor.undo());
    expect(ids()).toEqual(["first", "middle", "last"]);
    expect(actions.selectedIds()).toEqual(["first"]);
  });

  it("Escape collapses selection, read-only cut preserves content, and editors with equal IDs remain isolated", async () => {
    const right = engine();
    await act(async () => root.render(<>
      <BlockEditor editor={editor} onChange={save}><Probe /></BlockEditor>
      <BlockEditor editor={right} onChange={() => undefined} />
    </>));
    await select(["last"]); await key("Escape");
    expect(actions.selectedIds()).toEqual([]);
    await select(["first", "last"]);
    await key("Delete");
    expect(ids()).toEqual(["middle"]);
    expect(right.document.map(block => block.id)).toEqual(["first", "middle", "last"]);
    await act(async () => render(true));
    await select(["middle"]);
    const original = JSON.stringify(editor.document);
    await act(async () => editor.domElement!.dispatchEvent(clipboard("cut").event));
    expect(JSON.stringify(editor.document)).toBe(original);
  });
});
