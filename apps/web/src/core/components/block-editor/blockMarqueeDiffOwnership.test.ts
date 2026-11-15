// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isBlockMarqueeStart, startBlockMarqueeSelection } from "./blockMarqueeSelection";

describe("block marquee ownership beside readonly diff snapshots", () => {
  let editor: HTMLElement;
  let pane: HTMLElement;
  let parent: HTMLElement;
  let cleanup: (() => void) | undefined;
  let frame: FrameRequestCallback | undefined;
  const callbacks = { onActivate: vi.fn(), onSelect: vi.fn(), onFinish: vi.fn() };
  const pointer = (type: string, x: number, y: number): PointerEvent => new PointerEvent(type, {
    bubbles: true, cancelable: true, pointerId: 1, isPrimary: true, pointerType: "mouse", button: 0,
    buttons: type === "pointerup" ? 0 : 1, clientX: x, clientY: y,
  });
  function bounds(element: Element, x: number, y: number, width = 500, height = 26): void {
    vi.spyOn(element, "getBoundingClientRect").mockReturnValue(new DOMRect(x, y, width, height));
  }
  function snapshot(className: string, y = 150): HTMLElement {
    const fragment = document.createElement("div");
    fragment.className = className;
    fragment.contentEditable = "false";
    fragment.innerHTML = '<div class="bn-block-group"><div class="bn-block"><div class="bn-block-content snapshot-row"><p>Readonly snapshot</p></div></div></div>';
    parent.insertBefore(fragment, parent.querySelector(".live-child"));
    bounds(fragment.querySelector(".snapshot-row")!, 134, y, 476);
    return fragment;
  }
  function select(startY: number, endY: number): void {
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, startY), callbacks);
    window.dispatchEvent(pointer("pointerup", 450, endY));
  }
  function paint(): void { const pending = frame; frame = undefined; pending?.(0); }
  beforeEach(() => {
    frame = undefined;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frame = callback; return 1; });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => { frame = undefined; });
    document.body.innerHTML = '<div class="host-pane" data-block-editor-scroll-container><div class="block-editor"><div class="bn-editor" contenteditable="true"><div class="bn-block" data-id="parent"><div class="bn-block-content parent-row"><p>Parent</p></div><div class="bn-block live-child" data-id="child"><div class="bn-block-content child-row"><p>Current child</p></div></div></div><div class="bn-block" data-id="peer"><div class="bn-block-content peer-row"><p>Peer</p></div></div></div></div></div>';
    editor = document.querySelector<HTMLElement>(".bn-editor")!;
    pane = document.querySelector<HTMLElement>(".host-pane")!;
    parent = editor.querySelector<HTMLElement>('[data-id="parent"]')!;
    bounds(pane, 20, 50, 680, 450); bounds(editor, 60, 100, 600, 600);
    bounds(editor.querySelector(".parent-row")!, 110, 120);
    bounds(editor.querySelector(".child-row")!, 134, 220, 476);
    bounds(editor.querySelector(".peer-row")!, 110, 260);
    let captured = false;
    editor.setPointerCapture = () => { captured = true; };
    editor.hasPointerCapture = () => captured;
    editor.releasePointerCapture = () => { captured = false; };
    vi.clearAllMocks();
  });
  afterEach(() => {
    cleanup?.(); cleanup = undefined; vi.restoreAllMocks(); document.body.innerHTML = "";
  });

  it.each(["note-diff-deleted-blocks", "note-diff-deleted-blocks hbs-note-incoming", "readonly-rendered-fragment"])("does not borrow the enclosing parent's ID for an ID-less %s block", className => {
    snapshot(className);

    select(149, 179);

    expect(callbacks.onSelect).toHaveBeenLastCalledWith([]);
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith(false);
  });

  it("selects the real nested current child without its parent or the old/incoming rows", () => {
    snapshot("note-diff-deleted-blocks", 150);
    snapshot("note-diff-deleted-blocks hbs-note-incoming", 180);

    select(149, 249);

    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["child"]);
  });

  it("ignores snapshot geometry when deciding side and bottom whitespace starts", () => {
    const old = snapshot("note-diff-deleted-blocks");
    bounds(old.querySelector(".snapshot-row")!, 20, 350, 680, 200);
    for (const [x, y] of [[80, 200], [400, 400]]) {
      const event = pointer("pointerdown", x!, y!); pane.dispatchEvent(event);
      expect(isBlockMarqueeStart(editor, event)).toBe(true);
    }
    const content = pointer("pointerdown", 180, 360);
    old.querySelector("p")!.dispatchEvent(content);
    expect(isBlockMarqueeStart(editor, content)).toBe(false);
  });

  it.each(["formulaBlock", "codeBlock"])("keeps a live readonly %s atom row selectable by its own ID", type => {
    const atom = editor.querySelector<HTMLElement>(".peer-row")!;
    atom.dataset.contentType = type;
    atom.contentEditable = "false";
    atom.innerHTML = type === "codeBlock" ? '<pre><code>print(80)</code></pre>' : '<span class="katex">E = mc²</span>';
    snapshot("note-diff-deleted-blocks");

    select(249, 289);

    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["peer"]);
  });

  it("does not admit rows from a nested editor or an adjacent editor in the same pane", () => {
    const nested = document.createElement("div");
    nested.className = "block-editor";
    nested.innerHTML = '<div class="bn-editor"><div class="bn-block" data-id="foreign-nested"><div class="bn-block-content">Nested editor</div></div></div>';
    parent.append(nested);
    bounds(nested.querySelector(".bn-block-content")!, 134, 150, 476);
    const adjacent = document.createElement("div");
    adjacent.className = "block-editor";
    adjacent.innerHTML = '<div class="bn-editor"><div class="bn-block" data-id="foreign-adjacent"><div class="bn-block-content">Adjacent editor</div></div></div>';
    pane.append(adjacent);
    bounds(adjacent.querySelector(".bn-block-content")!, 134, 180, 476);

    select(149, 249);

    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["child"]);
    const down = pointer("pointerdown", 80, 180);
    adjacent.dispatchEvent(down);
    expect(isBlockMarqueeStart(editor, down)).toBe(false);
  });

  it("invalidates gesture geometry when old/new DOM is rebuilt and returns only the new live IDs", () => {
    const old = snapshot("note-diff-deleted-blocks");
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, 149), callbacks);
    window.dispatchEvent(pointer("pointermove", 450, 249)); paint();
    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["child"]);
    const replacement = document.createElement("div");
    replacement.className = "bn-block live-child"; replacement.dataset.id = "replacement-child";
    replacement.innerHTML = '<div class="bn-block-content">Replacement current child</div>';
    editor.querySelector(".live-child")!.replaceWith(replacement);
    bounds(replacement.querySelector(".bn-block-content")!, 134, 220, 476);
    old.replaceWith(snapshot("note-diff-deleted-blocks hbs-note-incoming", 180));

    window.dispatchEvent(pointer("pointerup", 450, 249));

    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["replacement-child"]);
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith(false);
  });
});
