// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { captureBlockDropGeometry, resolveBlockDropTarget } from "./blockDropTarget";

interface Row { readonly outer: HTMLElement; readonly content: HTMLElement }

describe("block drop ownership beside review snapshots", () => {
  let editor: HTMLElement, children: HTMLElement, parent: Row, current: Row, old: Row, incoming: Row;
  let hit: Element | null;

  function appendRow(container: HTMLElement, id: string | undefined, top: number, left = 124): Row {
    const outer = document.createElement("div");
    outer.className = "bn-block-outer";
    const block = document.createElement("div");
    block.className = "bn-block";
    if (id) { outer.dataset.id = id; block.dataset.id = id; }
    const content = document.createElement("div");
    content.className = "bn-block-content";
    content.innerHTML = `<span>${id ?? "snapshot"}</span>`;
    block.append(content); outer.append(block); container.append(outer);
    const bounds = new DOMRect(left, top, 600 - left, 40);
    vi.spyOn(outer, "getBoundingClientRect").mockReturnValue(bounds);
    vi.spyOn(content, "getBoundingClientRect").mockReturnValue(bounds);
    return { outer, content };
  }

  function appendSnapshot(role: "old" | "incoming", top: number): Row {
    const widget = document.createElement("div");
    widget.className = `note-diff-deleted-blocks${role === "incoming" ? " hbs-note-incoming" : ""}`;
    widget.contentEditable = "false";
    const group = document.createElement("div");
    group.className = "bn-block-group";
    widget.append(group); children.append(widget);
    return appendRow(group, undefined, top);
  }

  beforeEach(() => {
    document.body.innerHTML = '<div class="block-editor"><div class="bn-editor"><div class="bn-block-group"></div></div></div>';
    editor = document.querySelector<HTMLElement>(".bn-editor")!;
    vi.spyOn(editor, "getBoundingClientRect").mockReturnValue(new DOMRect(50, 100, 600, 500));
    parent = appendRow(editor.firstElementChild as HTMLElement, "parent", 130, 100);
    vi.mocked(parent.outer.getBoundingClientRect).mockReturnValue(new DOMRect(100, 130, 500, 230));
    children = document.createElement("div");
    children.className = "bn-block-group";
    parent.content.parentElement!.append(children);
    old = appendSnapshot("old", 220);
    incoming = appendSnapshot("incoming", 260);
    current = appendRow(children, "current", 320);
    hit = editor;
    vi.spyOn(document, "elementFromPoint").mockImplementation(() => hit);
  });
  afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

  it.each(["old", "incoming"] as const)("never treats an ID-less nested %s row as its live parent", role => {
    const snapshot = role === "old" ? old : incoming;
    hit = snapshot.content.firstElementChild;
    const pointer = { clientX: 350, clientY: role === "old" ? 240 : 280 };
    expect(resolveBlockDropTarget(editor, pointer)).toBeUndefined();
    expect(resolveBlockDropTarget(editor, pointer, captureBlockDropGeometry(editor))).toBeUndefined();
  });

  it("captures only current native rows, keeping review snapshots out of the drag geometry", () => {
    const geometry = captureBlockDropGeometry(editor);
    expect(geometry.rows).toEqual([parent.content, current.content]);
  });

  it("ignores snapshots when resolving whitespace to the nearest current row", () => {
    const pointer = { clientX: 350, clientY: 250 };
    const expected = { blockId: "current", placement: "before", line: { left: 124, top: 320, width: 476 } };
    expect(resolveBlockDropTarget(editor, pointer)).toEqual(expected);
    expect(resolveBlockDropTarget(editor, pointer, captureBlockDropGeometry(editor))).toEqual(expected);
  });

  it("retains a real read-only atom as a valid current drop target", () => {
    current.content.contentEditable = "false";
    current.content.dataset.contentType = "formulaBlock";
    hit = current.content.firstElementChild;
    expect(captureBlockDropGeometry(editor).rows).toContain(current.content);
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 340 })).toEqual({
      blockId: "current", placement: "inside", line: { left: 148, top: 360, width: 452 },
      targetRect: { left: 124, top: 320, width: 476, height: 40 },
    });
  });

  it("excludes an embedded foreign editor from direct hits, captured rows and fallback geometry", () => {
    const foreign = document.createElement("div");
    foreign.className = "bn-editor";
    children.append(foreign);
    const other = appendRow(foreign, "foreign", 230);
    hit = other.content.firstElementChild;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 250 })).toBeUndefined();
    const geometry = captureBlockDropGeometry(editor);
    expect(geometry.rows).toEqual([parent.content, current.content]);
    hit = editor;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 250 }, {
      ...geometry, rows: [other.content, ...geometry.rows],
    })?.blockId).toBe("current");
  });

  it("does not canonicalize sibling boundaries through an ID-less snapshot", () => {
    children.replaceChildren();
    const first = appendRow(children, "first", 180);
    appendRow(children, undefined, 224);
    const second = appendRow(children, "second", 268);
    hit = first.content.firstElementChild;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 216 })).toEqual({
      blockId: "first", placement: "after", line: { left: 124, top: 220, width: 476 },
    });
    hit = second.content.firstElementChild;
    expect(resolveBlockDropTarget(editor, { clientX: 350, clientY: 272 })).toEqual({
      blockId: "second", placement: "before", line: { left: 124, top: 268, width: 476 },
    });
  });
});
