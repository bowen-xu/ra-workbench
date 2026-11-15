// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resolveBlockDropTarget } from "./blockDropTarget";

describe("block-boundary drop coordinates", () => {
  let editor: HTMLElement;
  let hit: Element | null;
  beforeEach(() => {
    document.body.innerHTML = '<div class="block-editor"><div class="bn-editor"><div class="bn-block-outer"><div class="bn-block" data-id="parent"><div class="bn-block-content"><blockquote>Quote</blockquote></div><div class="bn-block-group"><div class="bn-block-outer"><div class="bn-block" data-id="child"><div class="bn-block-content"><p>Child</p></div></div></div></div></div></div></div><div class="bn-formatting-toolbar"><button>Bold</button></div></div>';
    editor = document.querySelector<HTMLElement>(".bn-editor")!;
    const contents = editor.querySelectorAll(".bn-block-content");
    const outers = editor.querySelectorAll(".bn-block-outer");
    vi.spyOn(editor, "getBoundingClientRect").mockReturnValue(new DOMRect(50, 100, 600, 400));
    vi.spyOn(contents[0]!, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 130, 500, 40));
    vi.spyOn(outers[0]!, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 130, 500, 90));
    vi.spyOn(contents[1]!, "getBoundingClientRect").mockReturnValue(new DOMRect(124, 180, 476, 40));
    vi.spyOn(outers[1]!, "getBoundingClientRect").mockReturnValue(new DOMRect(124, 180, 476, 40));
    hit = editor.querySelector("blockquote");
    vi.spyOn(document, "elementFromPoint").mockImplementation(() => hit);
  });
  afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

  it.each([110, 350, 599])("uses the same upper boundary at x=%s", (clientX) => {
    expect(resolveBlockDropTarget(editor, { clientX, clientY: 134 })).toEqual({ blockId: "parent", placement: "before", line: { left: 100, top: 130, width: 500 } });
  });
  it("places after a whole subtree rather than into its children", () => {
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 166 })).toEqual({ blockId: "parent", placement: "after", line: { left: 100, top: 220, width: 500 } });
  });
  it("keeps the indentation when targeting an existing nested row", () => {
    hit = editor.querySelector("p");
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 184 })).toEqual({ blockId: "child", placement: "before", line: { left: 124, top: 180, width: 476 } });
  });

  it("maps an attached caption hit to the owning resource's trailing boundary", () => {
    const content = editor.querySelectorAll<HTMLElement>(".bn-block-content")[1]!;
    content.dataset.contentType = "blockCaption"; hit = content.querySelector("p");
    expect(resolveBlockDropTarget(editor, {clientX: 350, clientY: 194})).toEqual({
      blockId: "parent", placement: "after", line: {left: 100, top: 220, width: 500},
    });
  });
  it("finds the underlying row when a fading toolbar is hit", () => {
    hit = document.querySelector("button");
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 134 })).toEqual({ blockId: "parent", placement: "before", line: { left: 100, top: 130, width: 500 } });
  });
  it("resolves whitespace by the nearest row", () => {
    hit = editor;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 178 })).toEqual({ blockId: "child", placement: "before", line: { left: 124, top: 180, width: 476 } });
  });
  it("rejects other panels and out-of-editor releases", () => {
    hit = document.body;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 134 })).toBeUndefined();
    hit = editor;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 70 })).toBeUndefined();
  });

  it.each([
    ["paragraph", "<p>Paragraph</p>"],
    ["quote", "<blockquote>Quote</blockquote>"],
    ["bulletListItem", "<p>Item</p>"],
    ["numberedListItem", "<p>Numbered item</p>"],
    ["checkListItem", '<p><input type="checkbox">Task</p>'],
    ["custom", '<div><input aria-label="Formula"></div>'],
  ])("resolves the middle of %s as a child destination with subtree feedback", (type, html) => {
    const content = editor.querySelector<HTMLElement>(".bn-block-content")!;
    content.dataset.contentType = type;
    content.innerHTML = html;
    hit = content.querySelector("input") ?? content.firstElementChild;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 150 })).toEqual({
      blockId: "parent", placement: "inside",
      line: { left: 124, top: 220, width: 476 },
      targetRect: { left: 100, top: 130, width: 500, height: 90 },
    });
  });

  it("targets a nested block without promoting its ancestor", () => {
    hit = editor.querySelector("p");
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 200 })).toEqual({
      blockId: "child", placement: "inside",
      line: { left: 148, top: 220, width: 452 },
      targetRect: { left: 124, top: 180, width: 476, height: 40 },
    });
  });

  it.each([
    [139.99, "before"], [140, "inside"], [159.99, "inside"], [160, "after"],
  ] as const)("uses distinct 25/50/25 regions at y=%s", (clientY, placement) => {
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY })?.placement).toBe(placement);
  });

  it.each([86, 100, 123.99, 601, 665])("keeps horizontal gutters at x=%s as sibling destinations", (clientX) => {
    hit = editor;
    const target = resolveBlockDropTarget(editor, { clientX, clientY: 145 });
    expect(target?.placement).toBe("before");
    expect(target?.targetRect).toBeUndefined();
  });

  it("starts the child destination at the content indentation threshold", () => {
    expect(resolveBlockDropTarget(editor, { clientX: 124, clientY: 150 })?.placement).toBe("inside");
  });

  function appendBlock(container: Element, id: string, rect: DOMRect): HTMLElement {
    const outer = document.createElement("div");
    outer.className = "bn-block-outer";
    outer.innerHTML = `<div class="bn-block" data-id="${id}"><div class="bn-block-content"><p></p></div></div>`;
    container.append(outer);
    vi.spyOn(outer, "getBoundingClientRect").mockReturnValue(rect);
    const content = outer.querySelector<HTMLElement>(".bn-block-content")!;
    vi.spyOn(content, "getBoundingClientRect").mockReturnValue(rect);
    return content;
  }

  it.each([0, 4, 16, 3.5])("uses one identity and line across a %s px sibling gap", (gap) => {
    const next = appendBlock(editor, "next", new DOMRect(100, 220 + gap, 500, 40));
    const afterParent = resolveBlockDropTarget(editor, { clientX: 350, clientY: 166 });
    hit = next;
    const beforeNext = resolveBlockDropTarget(editor, { clientX: 350, clientY: 222 + gap });
    expect(afterParent).toEqual(beforeNext);
    expect(beforeNext).toEqual({ blockId: "next", placement: "before", line: { left: 100, top: 220 + gap / 2, width: 500 } });
  });

  it("unifies nested siblings without merging the ancestor boundary", () => {
    const group = editor.querySelector(".bn-block-group")!;
    const secondChild = appendBlock(group, "second-child", new DOMRect(124, 224, 476, 40));
    vi.mocked(editor.querySelector(".bn-block-outer")!.getBoundingClientRect).mockReturnValue(new DOMRect(100, 130, 500, 134));
    appendBlock(editor, "root-next", new DOMRect(100, 268, 500, 40));
    hit = group.querySelector("p");
    const afterFirst = resolveBlockDropTarget(editor, { clientX: 350, clientY: 215 });
    hit = secondChild;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 226 })).toEqual(afterFirst);
    expect(afterFirst).toEqual({ blockId: "second-child", placement: "before", line: { left: 124, top: 222, width: 476 } });
    const afterLastChild = resolveBlockDropTarget(editor, { clientX: 350, clientY: 260 });
    hit = editor.querySelector("blockquote");
    const afterParent = resolveBlockDropTarget(editor, { clientX: 350, clientY: 166 });
    expect(afterLastChild).toEqual({ blockId: "second-child", placement: "after", line: { left: 124, top: 264, width: 476 } });
    expect(afterParent).toEqual({ blockId: "root-next", placement: "before", line: { left: 100, top: 266, width: 500 } });
  });
});
