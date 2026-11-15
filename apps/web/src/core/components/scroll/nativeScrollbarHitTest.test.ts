/** @vitest-environment happy-dom */
import {afterEach, expect, it} from "vitest";
import {isOverNativeScrollbar} from "./nativeScrollbarHitTest";

afterEach(() => document.body.replaceChildren());
function scroller(values: Record<string, number> = {}, scale = 1): HTMLElement {
  const node = document.createElement("div"); document.body.append(node);
  node.style.cssText = "overflow-x:auto;overflow-y:auto;border:2px solid black";
  const dimensions = {offsetWidth: 110, offsetHeight: 110, clientWidth: 100, clientHeight: 100, clientLeft: 2, clientTop: 2, scrollWidth: 200, scrollHeight: 200, ...values};
  for (const [key, value] of Object.entries(dimensions)) Object.defineProperty(node, key, {configurable: true, value});
  node.getBoundingClientRect = () => ({left: 10, top: 20, width: dimensions.offsetWidth * scale, height: dimensions.offsetHeight * scale} as DOMRect);
  return node;
}
it("distinguishes the measured gutters from borders, content and outside points", () => {
  const node = scroller();
  expect(isOverNativeScrollbar(node, 115, 50)).toBe(true);
  expect(isOverNativeScrollbar(node, 50, 125)).toBe(true);
  expect(isOverNativeScrollbar(node, 50, 50)).toBe(false);
  expect(isOverNativeScrollbar(node, 119, 50)).toBe(false);
  expect(isOverNativeScrollbar(node, 121, 50)).toBe(false);
});
it("locates a left-side RTL scrollbar using clientLeft rather than a guessed right edge", () => {
  const node = scroller({clientLeft: 8}); node.dir = "rtl";
  expect(isOverNativeScrollbar(node, 15, 50)).toBe(true);
  expect(isOverNativeScrollbar(node, 115, 50)).toBe(false);
});
it("does not treat an empty stable gutter or hidden-overflow surface as a scrollbar", () => {
  const node = scroller({scrollWidth: 100, scrollHeight: 100});
  expect(isOverNativeScrollbar(node, 115, 50)).toBe(false);
  const hidden = scroller(); hidden.style.overflowX = "hidden"; hidden.style.overflowY = "hidden";
  expect(isOverNativeScrollbar(hidden, 115, 50)).toBe(false);
});
it("accounts for scale transforms without expanding the hit area into content", () => {
  const node = scroller({}, 2);
  expect(isOverNativeScrollbar(node, 220, 100)).toBe(true);
  expect(isOverNativeScrollbar(node, 200, 100)).toBe(false);
});
it("ignores unmeasurable and scrollbar-free elements", () => {
  const node = scroller({offsetWidth: 104, offsetHeight: 104});
  expect(isOverNativeScrollbar(node, 112, 50)).toBe(false);
  expect(isOverNativeScrollbar(document.createElement("div"), 0, 0)).toBe(false);
});
