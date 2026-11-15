// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { BlockNoteEditor } from "@blocknote/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BlockEditor } from "./BlockEditor";
import { moveBlocks } from "./moveBlocks";
import { useBlockHighlight } from "./BlockHighlight";

describe("shared editor host boundary", () => {
  let host: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });
  function editor() {
    return BlockNoteEditor.create({ initialContent: [
      { id: "first", type: "paragraph", content: "Idea" },
      { id: "last", type: "paragraph", content: "Tail" },
    ] });
  }

  it("mounts without a Notes provider, emits edits not selection, and preserves history across save echoes", async () => {
    const engine = editor();
    const save = vi.fn();
    const render = () => root.render(<BlockEditor editor={engine} onChange={() => save(engine.document)} />);
    await act(async () => render());
    expect(save).not.toHaveBeenCalled();
    const dom = host.querySelector(".bn-editor");
    await act(async () => engine.setTextCursorPosition("first"));
    expect(save).not.toHaveBeenCalled();
    await act(async () => engine.updateBlock("first", { content: "Revised idea" }));
    expect(save).toHaveBeenCalled();
    expect(JSON.stringify(save.mock.lastCall)).toContain("Revised idea");
    await act(async () => render());
    expect(host.querySelector(".bn-editor")).toBe(dom);
    await act(async () => { expect(engine.undo()).toBe(true); });
    expect(host.textContent).toContain("Idea");
    expect(host.textContent).not.toContain("Revised idea");
  });

  it("keeps two hosts' changes, undo and instance selectors separate even with identical block IDs", async () => {
    const left = editor(); const right = editor();
    const leftSave = vi.fn(); const rightSave = vi.fn();
    await act(async () => root.render(<>
      <BlockEditor editor={left} onChange={leftSave} />
      <BlockEditor editor={right} onChange={rightSave} />
    </>));
    const surfaces = [...host.querySelectorAll<HTMLElement>(".block-editor")];
    expect(surfaces[0]?.dataset.editorInstance).not.toBe(surfaces[1]?.dataset.editorInstance);
    await act(async () => { expect(moveBlocks(left, ["last"], { blockId: "first", placement: "before" })).toBe(true); });
    expect(left.document.map((b) => b.id)).toEqual(["last", "first"]);
    expect(right.document.map((b) => b.id)).toEqual(["first", "last"]);
    expect(leftSave).toHaveBeenCalledOnce();
    expect(rightSave).not.toHaveBeenCalled();
    await act(async () => { expect(left.undo()).toBe(true); });
    expect(left.document.map((b) => b.id)).toEqual(["first", "last"]);
  });

  it("replaces only an explicitly keyed document session and exposes a read-only surface", async () => {
    const first = editor(); const next = editor(); const save = vi.fn();
    await act(async () => root.render(<BlockEditor key="first-session" editor={first} onChange={save} />));
    const old = host.querySelector(".bn-editor");
    await act(async () => root.render(<BlockEditor key="next-session" editor={next} onChange={save} readOnly />));
    expect(old?.isConnected).toBe(false);
    expect(host.querySelectorAll(".bn-editor")).toHaveLength(1);
    expect(host.querySelector(".bn-editor")?.getAttribute("contenteditable")).toBe("false");
    expect(save).not.toHaveBeenCalled();
  });

  it("keeps the same mounted view and keyboard entry point across temporary read-only fences", async () => {
    const engine = editor(), save = vi.fn();
    await act(async () => root.render(<BlockEditor editor={engine} onChange={save} />));
    const view = engine.prosemirrorView, dom = engine.domElement;
    const mounting = vi.fn(); const stop = engine.onMount(mounting);
    await act(async () => root.render(<BlockEditor editor={engine} onChange={save} readOnly />));
    expect(engine.isEditable).toBe(false);
    expect(dom?.getAttribute("contenteditable")).toBe("false");
    await act(async () => root.render(<BlockEditor editor={engine} onChange={save} />));
    expect(engine.isEditable).toBe(true);
    expect(engine.domElement).toBe(dom); expect(engine.prosemirrorView).toBe(view);
    expect(dom?.tabIndex).toBe(0); expect(mounting).not.toHaveBeenCalled(); stop();
  });

  it("retains movement callbacks across host renders while applying the latest constraints", async () => {
    const engine = editor();
    let actions: ReturnType<typeof useBlockHighlight> | undefined;
    function Probe(): null { actions = useBlockHighlight(); return null; }
    const render = (canNest: boolean, readOnly = false) => root.render(
      <BlockEditor editor={engine} onChange={() => undefined} movePolicy={{ canNest: () => canNest }} readOnly={readOnly}><Probe /></BlockEditor>,
    );
    await act(async () => render(true));
    const original = actions!;
    const target = { blockId: "first", placement: "inside" } as const;
    expect(original.canDrop(["last"], target)).toBe(true);
    await act(async () => render(false));
    expect(actions!.canDrop).toBe(original.canDrop);
    expect(actions!.move).toBe(original.move);
    expect(original.canDrop(["last"], target)).toBe(false);
    expect(original.move(["last"], target)).toBe(false);
    await act(async () => render(true, true));
    expect(original.move(["last"], target)).toBe(false);
    expect(engine.document.map(block => block.id)).toEqual(["first", "last"]);
  });
});
