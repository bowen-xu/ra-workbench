// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isBlockMarqueeStart } from "./blockMarqueeSelection";
import { ownsEditorHoverEvent } from "./editorSurface";

describe("retained editor background ownership", () => {
  let pane: HTMLElement;
  let retained: HTMLElement;
  let surface: HTMLElement;
  let editor: HTMLElement;

  function bounds(element: Element, x: number, y: number, width: number, height: number): void {
    const rect = new DOMRect(x, y, width, height);
    vi.spyOn(element, "getBoundingClientRect").mockReturnValue(rect);
    vi.spyOn(element, "getClientRects").mockReturnValue([rect] as unknown as DOMRectList);
  }
  function down(target: HTMLElement, x = 40, y = 140, root = editor): boolean {
    const event = new PointerEvent("pointerdown", { bubbles: true, cancelable: true,
      isPrimary: true, pointerType: "mouse", button: 0, buttons: 1, clientX: x, clientY: y });
    target.dispatchEvent(event);
    return isBlockMarqueeStart(root, event);
  }
  function hover(target: HTMLElement, root = editor): boolean {
    const event = new MouseEvent("mousemove", { bubbles: true });
    target.dispatchEvent(event);
    return ownsEditorHoverEvent(root, event);
  }
  function anotherEditor(host: HTMLElement): HTMLElement {
    const sibling = document.createElement("div");
    sibling.className = "block-editor";
    sibling.innerHTML = '<div class="bn-editor"><div class="bn-block" data-id="other"><div class="bn-block-content">Other note</div></div></div>';
    host.append(sibling);
    bounds(sibling, 100, 100, 600, 500);
    bounds(sibling.querySelector(".bn-editor")!, 100, 100, 600, 500);
    bounds(sibling.querySelector(".bn-block-content")!, 150, 120, 500, 26);
    return sibling;
  }

  beforeEach(() => {
    document.body.innerHTML = '<main data-block-editor-scroll-container><header>Page header</header><div class="retained-note-editor"><div class="editor-diff-highlight"><div class="block-editor"><div class="bn-editor"><div class="bn-block" data-id="a"><div class="bn-block-content">First block</div></div><div class="bn-block" data-id="b"><div class="bn-block-content">Second block</div></div></div></div></div></div><section>Relationships</section><button>Page action</button></main>';
    pane = document.querySelector("main")!;
    retained = pane.querySelector(".retained-note-editor")!;
    surface = retained.querySelector(".block-editor")!;
    editor = surface.querySelector(".bn-editor")!;
    bounds(pane, 20, 50, 760, 850);
    bounds(retained, 20, 100, 760, 700);
    bounds(retained.querySelector(".editor-diff-highlight")!, 20, 100, 760, 700);
    bounds(surface, 100, 100, 600, 500);
    bounds(editor, 100, 100, 600, 500);
    const rows = editor.querySelectorAll(".bn-block-content");
    bounds(rows[0]!, 150, 120, 500, 26);
    bounds(rows[1]!, 150, 180, 500, 26);
  });
  afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });

  it("starts in both outer side margins of the retained layout wrapper", () => {
    expect(down(retained, 40)).toBe(true);
    expect(down(retained, 760)).toBe(true);
  });
  it("keeps wrapper ownership through the shared diff presentation layer", () => {
    const diff = retained.querySelector<HTMLElement>(".editor-diff-highlight")!;
    expect(down(diff, 40)).toBe(true);
    expect(down(diff, 760)).toBe(true);
  });
  it("starts below the last block on either wrapper or scroll pane background", () => {
    expect(down(retained, 400, 750)).toBe(true);
    expect(down(pane, 400, 850)).toBe(true);
  });
  it("does not count hidden retained tabs as editors sharing the pane margins", () => {
    const hidden = document.createElement("div"); hidden.hidden = true; pane.append(hidden);
    anotherEditor(hidden);
    expect(down(pane, 40)).toBe(true);
    expect(down(pane, 760)).toBe(true);
  });
  it("does not count inert retained tabs as editors sharing the pane margins", () => {
    const inactive = document.createElement("div"); inactive.setAttribute("inert", ""); pane.append(inactive);
    anotherEditor(inactive);
    expect(down(pane, 40)).toBe(true);
    expect(down(pane, 760)).toBe(true);
  });
  it("never starts a gesture for the inactive editor itself", () => {
    for (const attribute of ["hidden", "inert"]) {
      retained.setAttribute(attribute, "");
      expect(down(surface, 120)).toBe(false);
      expect(down(pane, 400, 850)).toBe(false);
      retained.removeAttribute(attribute);
    }
  });
  it("leaves headers, relationship sections and page controls outside editor ownership", () => {
    for (const tag of ["header", "section", "button"]) {
      const target = pane.querySelector<HTMLElement>(tag)!;
      expect(down(target)).toBe(false);
      expect(hover(target)).toBe(false);
    }
  });
  it("keeps outer pane margins unclaimed when two visible editors share the pane", () => {
    const other = anotherEditor(pane);
    expect(down(pane)).toBe(false);
    expect(down(other, 120)).toBe(false);
    expect(hover(pane)).toBe(false);
    expect(hover(other)).toBe(false);
  });
  it("does not count an editor in a nested scroll pane as another owner of this pane", () => {
    const nested = document.createElement("aside");
    nested.dataset.blockEditorScrollContainer = "";
    pane.append(nested);
    const other = anotherEditor(nested);
    expect(down(pane)).toBe(true);
    expect(hover(pane)).toBe(true);
    expect(down(nested)).toBe(false);
    expect(hover(other)).toBe(false);
  });
  it("uses the same wrapper background ownership for side-button hover", () => {
    const inactive = document.createElement("div");
    inactive.hidden = true; inactive.setAttribute("inert", ""); pane.append(inactive);
    anotherEditor(inactive);
    expect(hover(retained)).toBe(true);
    expect(hover(retained.querySelector<HTMLElement>(".editor-diff-highlight")!)).toBe(true);
    expect(hover(pane)).toBe(true);
  });
  it("never claims hover for a hidden or inert editor", () => {
    for (const attribute of ["hidden", "inert"]) {
      retained.setAttribute(attribute, "");
      expect(hover(surface)).toBe(false);
      expect(hover(pane)).toBe(false);
      retained.removeAttribute(attribute);
    }
  });
  it("preserves native content and controls even within an owned layout wrapper", () => {
    expect(down(editor.querySelector<HTMLElement>(".bn-block-content")!, 180)).toBe(false);
    const button = document.createElement("button"); retained.append(button);
    expect(down(button)).toBe(false);
    const input = document.createElement("input"); retained.append(input);
    expect(down(input)).toBe(false);
  });
});
