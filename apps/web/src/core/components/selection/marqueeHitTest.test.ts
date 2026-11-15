// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createMarqueeHitTest } from "./marqueeHitTest";

let pane: HTMLElement, row: HTMLElement, dispose: (() => void) | undefined;
let resize: ResizeObserverCallback;
const getIds = () => {
  const test = createMarqueeHitTest(pane, pane, "[data-id]", element => element.dataset.id);
  dispose = test.dispose; return test.getIds;
};
beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { resize = callback; }
    observe() {} unobserve() {} disconnect() {}
  });
  pane = document.createElement("div"); row = document.createElement("div"); row.dataset.id = "a";
  pane.append(row); document.body.append(pane);
  vi.spyOn(row, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 100, 200, 40));
});
afterEach(() => { dispose?.(); dispose = undefined; pane.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("reuses geometry across pointer moves and adjusts pane scrolling without remeasurement", () => {
  const hit = getIds();
  expect(hit({clientX: 90, clientY: 90}, {clientX: 200, clientY: 120})).toEqual(["a"]);
  expect(hit({clientX: 90, clientY: 90}, {clientX: 210, clientY: 130})).toEqual(["a"]);
  pane.scrollTop = 80; pane.dispatchEvent(new Event("scroll"));
  expect(hit({clientX: 90, clientY: 0}, {clientX: 200, clientY: 40})).toEqual(["a"]);
  expect(row.getBoundingClientRect).toHaveBeenCalledTimes(1);
});

it("remeasures after resize, ancestor scrolling and synchronous content replacement", () => {
  const hit = getIds(), start = {clientX: 90, clientY: 90}, end = {clientX: 200, clientY: 120};
  hit(start, end); resize([], {} as ResizeObserver); hit(start, end);
  expect(row.getBoundingClientRect).toHaveBeenCalledTimes(2);
  window.dispatchEvent(new Event("scroll")); hit(start, end);
  expect(row.getBoundingClientRect).toHaveBeenCalledTimes(3);
  row.dataset.id = "b";
  expect(hit(start, end)).toEqual(["b"]);
  expect(row.getBoundingClientRect).toHaveBeenCalledTimes(4);
  // Selection paint alone is not a geometry change.
  row.setAttribute("aria-selected", "true"); hit(start, end);
  expect(row.getBoundingClientRect).toHaveBeenCalledTimes(4);
  dispose!();
  expect(hit(start, end)).toEqual([]);
});
