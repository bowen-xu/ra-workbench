// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { PdfCharacterSelectionController } from "./pdfCharacterSelectionController";
import { PdfPageCharacterStore } from "./pdfPageCharacterModel";
import * as selectionModel from "./pdfPageCharacterSelection";

const controllers: PdfCharacterSelectionController[] = [];
afterEach(() => {
  controllers.splice(0).forEach(controller => controller.destroy());
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function harness() {
  const container = document.createElement("div"); container.className = "pdf-viewer-container";
  const viewer = document.createElement("div"); viewer.className = "pdfViewer";
  const page = document.createElement("div"); page.className = "page"; page.dataset.pageNumber = "1";
  page.getBoundingClientRect = () => new DOMRect(0, 0, 600, 800);
  const layer = document.createElement("div"); layer.className = "textLayer";
  const text = document.createElement("span"); text.textContent = "alpha beta gamma";
  layer.append(text);
  const highlight = document.createElement("button"); highlight.dataset.pdfTextSelection = "true";
  page.append(layer, highlight); viewer.append(page); container.append(viewer); document.body.append(container);
  const store = new PdfPageCharacterStore();
  store.setPageCharacters(0, Array.from(text.textContent).map((value, index) => ({
    value, rect: { x: .1 + index * .01, y: .1, width: .01, height: .02 }, rotation: 0,
  })));
  const start = vi.fn(); const complete = vi.fn(); const clicked = vi.fn(); const change = vi.fn();
  highlight.addEventListener("click", clicked);
  const controller = new PdfCharacterSelectionController({
    viewer, pageStore: store, onSelectionStart: start, onSelectionChange: change, onSelectionComplete: complete,
  });
  controllers.push(controller);
  vi.spyOn(document, "elementFromPoint").mockImplementation(() => highlight);
  const pointer = (target: Element, type: string, index: number, extra: PointerEventInit = {}) => {
    const event = new PointerEvent(type, {
      bubbles: true, cancelable: true, isPrimary: true, pointerId: 1, pointerType: "mouse", button: 0,
      clientX: 60 + index * 6, clientY: 88, ...extra,
    });
    target.dispatchEvent(event); return event;
  };
  const down = (target: Element, index: number, detail = 1, extra: PointerEventInit = {}) => {
    pointer(target, "pointerdown", index, extra);
    target.dispatchEvent(new MouseEvent("mousedown", {
      bubbles: true, cancelable: true, button: 0, detail, clientX: 60 + index * 6, clientY: 88, ...extra,
    }));
  };
  const click = (detail = 1) => highlight.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail }));
  return { viewer, page, store, controller, text, highlight, start, complete, change, clicked, down, pointer, click };
}

function frames() {
  const pending = new Map<number, FrameRequestCallback>(); let next = 0;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { pending.set(++next, callback); return next; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { pending.delete(id); });
  return () => { const callbacks = [...pending.values()]; pending.clear(); callbacks.forEach(callback => callback(0)); };
}

describe("PDF drag work scheduling", () => {
  it("coalesces hit testing as well as preview work, skips unchanged endpoints, and builds full text only on release", () => {
    const flush = frames(), h = harness();
    const sync = vi.spyOn(h.store, "syncPage");
    const full = vi.spyOn(selectionModel, "createPdfMultiPageCharacterSelection");
    h.down(h.highlight, 0);
    const initialHits = sync.mock.calls.length;
    for (let index = 1; index <= 10; index++) h.pointer(h.highlight, "pointermove", index);
    expect(sync).toHaveBeenCalledTimes(initialHits); expect(h.change).not.toHaveBeenCalled();
    flush();
    expect(sync).toHaveBeenCalledTimes(initialHits + 1); expect(h.change).toHaveBeenCalledOnce();
    expect(full).not.toHaveBeenCalled(); expect(h.complete).not.toHaveBeenCalled();
    h.pointer(h.highlight, "pointermove", 10); flush();
    expect(h.change).toHaveBeenCalledOnce();
    // A pending move must not override a newer release coordinate.
    h.pointer(h.highlight, "pointermove", 14);
    h.pointer(h.highlight, "pointerup", 5); flush();
    expect(full).toHaveBeenCalledOnce();
    expect(h.complete.mock.calls[0]?.[0].text).toBe("alpha");
    expect(h.change.mock.calls.at(-1)?.[0]).toBeUndefined();
  });

  it("does not publish a queued preview after cancellation or destruction", () => {
    const flush = frames(), h = harness(); h.down(h.highlight, 0);
    h.pointer(h.highlight, "pointermove", 10); h.pointer(h.highlight, "pointercancel", 10);
    const count = h.change.mock.calls.length; flush();
    expect(h.change).toHaveBeenCalledTimes(count); expect(h.complete).not.toHaveBeenCalled();
    h.down(h.highlight, 0); h.pointer(h.highlight, "pointermove", 10); h.controller.destroy();
    const destroyedCount = h.change.mock.calls.length; flush();
    expect(h.change).toHaveBeenCalledTimes(destroyedCount);
  });

  it("retains the latest pending in-page endpoint when released outside the PDF", () => {
    const flush = frames(), h = harness(); h.down(h.highlight, 0);
    h.pointer(h.highlight, "pointermove", 16);
    vi.mocked(document.elementFromPoint).mockReturnValue(null);
    h.pointer(h.viewer, "pointerup", 16, { clientX: 900 }); flush();
    expect(h.complete.mock.calls[0]?.[0].text).toBe("alpha beta gamma");
  });

  it("keeps live geometry identical to committed geometry for forward and reverse two-page ranges", () => {
    const h = harness(); h.down(h.highlight, 0);
    const second = document.createElement("div"); second.dataset.pageNumber = "2";
    second.getBoundingClientRect = () => new DOMRect(0, 810, 600, 800);
    h.store.setPageCharacters(1, Array.from("second page").map((value, index) => ({ value,
      rect: { x: .1 + index * .01, y: .1, width: .01, height: .02 }, rotation: 0 })));
    const firstModel = h.store.syncPage(h.page), secondModel = h.store.syncPage(second);
    if (!firstModel || !secondModel) throw new Error("Missing character models");
    const firstPoint = { index: 2, offset: 0 as const }, secondPoint = { index: 5, offset: 1 as const };
    for (const [anchor, focus, a, f] of [[firstModel, secondModel, firstPoint, secondPoint],
      [secondModel, firstModel, secondPoint, firstPoint], [firstModel, firstModel, firstPoint, secondPoint]] as const) {
      const live = selectionModel.createPdfLiveCharacterSegments(anchor, a, focus, f);
      const complete = selectionModel.createPdfMultiPageCharacterSelection(anchor, a, focus, f);
      expect(live).toEqual(complete?.segments.map(segment => ({ pageIndex: segment.pageIndex, rects: segment.rects })));
    }
  });
});

describe("PDF text gestures over saved highlights", () => {
  it.each(["text", "highlight"] as const)("selects a whole word on %s using the mouse click count", target => {
    const h = harness();
    h.down(h[target], 2, 2); // Real browsers report detail=0 on pointerdown.
    h.pointer(h[target], "pointerup", 2);
    h.click(2);
    expect(h.complete).toHaveBeenCalledOnce();
    expect(h.complete.mock.calls[0]?.[0].text).toBe("alpha");
    expect(h.complete.mock.calls[0]?.[0].segments[0].context).toEqual({ before: "", after: " beta gamma" });
    expect(h.clicked).not.toHaveBeenCalled();
  });

  it("keeps a plain highlight click available without beginning a text selection", () => {
    const h = harness(); h.down(h.highlight, 2); h.pointer(h.highlight, "pointerup", 2); h.click();
    expect(h.start).not.toHaveBeenCalled(); expect(h.complete).not.toHaveBeenCalled();
    expect(h.clicked).toHaveBeenCalledOnce();
  });

  it.each([true, false])("selects through a highlight with move events=%s, suppressing the trailing annotation click", move => {
    const h = harness(); h.down(h.highlight, 0);
    if (move) h.pointer(h.highlight, "pointermove", 10);
    h.pointer(h.highlight, "pointerup", 10); h.click();
    expect(h.start).toHaveBeenCalledOnce();
    expect(h.complete.mock.calls[0]?.[0].text).toBe("alpha beta");
    expect(h.clicked).not.toHaveBeenCalled();
    // A later ordinary click still opens the saved highlight actions.
    h.down(h.highlight, 2); h.pointer(h.highlight, "pointerup", 2); h.click();
    expect(h.clicked).toHaveBeenCalledOnce();
  });

  it("leaves Cmd+click to the annotation host without selecting text", () => {
    const h = harness(); h.down(h.highlight, 2, 1, { metaKey: true });
    h.pointer(h.highlight, "pointerup", 2, { metaKey: true }); h.click();
    expect(h.start).not.toHaveBeenCalled(); expect(h.complete).not.toHaveBeenCalled();
    expect(h.clicked).toHaveBeenCalledOnce();
  });

  it("keeps line/page expansion and keyboard annotation activation", () => {
    const h = harness();
    for (const detail of [3, 4]) {
      h.down(h.highlight, 2, detail); h.pointer(h.highlight, "pointerup", 2); h.click(detail);
      expect(h.complete.mock.calls.at(-1)?.[0].text).toBe("alpha beta gamma");
    }
    expect(h.clicked).not.toHaveBeenCalled();
    h.click(0); expect(h.clicked).toHaveBeenCalledOnce();
  });

  it("ignores other controls, secondary buttons and cancelled drags", () => {
    const h = harness(); const other = document.createElement("button"); h.viewer.append(other);
    h.down(other, 2, 2); expect(h.start).not.toHaveBeenCalled();
    h.pointer(h.highlight, "pointerdown", 2, { button: 2 }); expect(h.start).not.toHaveBeenCalled();
    h.down(h.highlight, 0); h.pointer(h.highlight, "pointermove", 10); h.pointer(h.highlight, "pointercancel", 10);
    expect(h.complete).not.toHaveBeenCalled();
    expect(h.viewer.querySelector("[data-custom-selection-active]")).toBeNull();
  });
});
