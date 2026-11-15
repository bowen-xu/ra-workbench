/** @vitest-environment happy-dom */
import {act, StrictMode} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {ApplicationScrollbars, installApplicationScrollbars} from "./ApplicationScrollbars";

interface FakeAnimation {cancel: ReturnType<typeof vi.fn>; onfinish: (() => void) | null}
const alpha = "--ui-scrollbar-alpha";
let dispose: (() => void) | undefined, reduced: boolean;
let animations: {node: HTMLElement; animation: FakeAnimation; frames: unknown; options: unknown}[];
let previousAnimate: PropertyDescriptor | undefined;
beforeEach(() => {
  vi.useFakeTimers({toFake: ["setTimeout", "clearTimeout"]}); reduced = false; animations = [];
  vi.spyOn(window, "matchMedia").mockImplementation(() => ({get matches() {return reduced;}} as MediaQueryList));
  previousAnimate = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "animate");
  Object.defineProperty(HTMLElement.prototype, "animate", {configurable: true, writable: true,
    value: function(this: HTMLElement, frames: unknown, options: unknown) {
      const animation: FakeAnimation = {cancel: vi.fn(), onfinish: null};
      animations.push({node: this, animation, frames, options}); return animation;
    },
  });
});
afterEach(() => {
  dispose?.(); dispose = undefined; document.body.replaceChildren();
  if (previousAnimate) Object.defineProperty(HTMLElement.prototype, "animate", previousAnimate);
  else Reflect.deleteProperty(HTMLElement.prototype, "animate");
  vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers();
});
function viewport(parent: HTMLElement = document.body): HTMLDivElement {
  const node = document.createElement("div"); parent.append(node); return node;
}
function scroll(node: HTMLElement): Event {
  const event = new Event("scroll", {cancelable: true}); node.dispatchEvent(event); return event;
}
function opacity(node: HTMLElement): string {return node.style.getPropertyValue(alpha);}
function scrollbarViewport(parent?: HTMLElement): HTMLElement {
  const node = viewport(parent);
  node.style.overflowX = "auto"; node.style.overflowY = "auto";
  for (const [key, value] of Object.entries({offsetWidth: 100, offsetHeight: 100, clientWidth: 94, clientHeight: 100, scrollHeight: 500, clientLeft: 0})) {
    Object.defineProperty(node, key, {configurable: true, value});
  }
  node.getBoundingClientRect = () => ({left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100} as DOMRect);
  return node;
}
function pointer(node: HTMLElement, x: number, type = "pointermove", y = 50): void {
  node.dispatchEvent(new PointerEvent(type, {bubbles: true, clientX: x, clientY: y, pointerType: "mouse"}));
}

it("holds and strengthens only scrollbar hover; moving through content restarts the delayed fade", () => {
  dispose = installApplicationScrollbars(document); const node = scrollbarViewport();
  pointer(node, 40); expect(opacity(node)).toBe("");
  pointer(node, 97); expect(opacity(node)).toBe("1");
  expect(node.style.getPropertyValue("--ui-scrollbar-hover")).toBe("1");
  scroll(node); vi.advanceTimersByTime(3000); expect(animations).toHaveLength(0);
  pointer(node, 40); expect(node.style.getPropertyValue("--ui-scrollbar-hover")).toBe("");
  vi.advanceTimersByTime(699); expect(animations).toHaveLength(0);
  vi.advanceTimersByTime(1); expect(animations).toHaveLength(1);
});

it("cancels an in-progress fade on scrollbar hover without a fade-in", () => {
  dispose = installApplicationScrollbars(document); const node = scrollbarViewport();
  scroll(node); vi.advanceTimersByTime(700); pointer(node, 97);
  expect(animations[0]!.animation.cancel).toHaveBeenCalledOnce();
  expect(opacity(node)).toBe("1");
  vi.advanceTimersByTime(3000); expect(animations).toHaveLength(1);
});

it("retains the thumb during native dragging outside its gutter, then fades after release", () => {
  dispose = installApplicationScrollbars(document); const node = scrollbarViewport();
  pointer(node, 97, "pointerdown"); pointer(node, 40);
  scroll(node); vi.advanceTimersByTime(3000); expect(animations).toHaveLength(0);
  pointer(node, 40, "pointerup"); vi.advanceTimersByTime(700); expect(animations).toHaveLength(1);
});

it("does not hold an ancestor scrollbar when the pointer is within nested scroll content", () => {
  dispose = installApplicationScrollbars(document); const outer = scrollbarViewport(), inner = scrollbarViewport(outer);
  pointer(inner, 97); vi.advanceTimersByTime(3000);
  expect(opacity(inner)).toBe("1"); expect(opacity(outer)).toBe("");
  pointer(inner, 40); vi.advanceTimersByTime(700);
  expect(animations.map(item => item.node)).toEqual([inner]);
});

it("releases the hover hold when the pointer leaves the document or the window loses focus", () => {
  dispose = installApplicationScrollbars(document); const node = scrollbarViewport();
  pointer(node, 97); pointer(node, 110, "pointerout"); vi.advanceTimersByTime(700);
  expect(animations).toHaveLength(1);
  pointer(node, 97, "pointerdown"); window.dispatchEvent(new Event("blur")); vi.advanceTimersByTime(700);
  expect(animations).toHaveLength(2);
});

it("starts hidden and reveals only the actual scroll owner without touching layout or position", () => {
  dispose = installApplicationScrollbars(document);
  const outer = viewport(), inner = viewport(outer);
  outer.scrollTop = 100; inner.scrollLeft = 240;
  expect(opacity(inner)).toBe("");
  expect(scroll(inner).defaultPrevented).toBe(false);
  expect(opacity(inner)).toBe("1"); expect(opacity(outer)).toBe("");
  expect(outer.scrollTop).toBe(100); expect(inner.scrollLeft).toBe(240);
  expect(inner.style.overflow).toBe(""); expect(inner.style.width).toBe("");
  expect(document.documentElement.dataset.appScrollbars).toBe("true");
});

it("waits for 700ms of inactivity then fades the registered alpha on the real element", () => {
  dispose = installApplicationScrollbars(document); const node = viewport(); scroll(node);
  vi.advanceTimersByTime(699); expect(animations).toHaveLength(0); expect(opacity(node)).toBe("1");
  vi.advanceTimersByTime(1);
  expect(animations).toHaveLength(1);
  expect(animations[0]).toMatchObject({node, frames: {[alpha]: ["1", "0"]}, options: {duration: 200, easing: "ease-out", fill: "forwards"}});
  animations[0]!.animation.onfinish?.();
  expect(opacity(node)).toBe(""); expect(animations[0]!.animation.cancel).toHaveBeenCalledOnce();
});

it("renews the idle delay through continuous scrolling", () => {
  dispose = installApplicationScrollbars(document); const node = viewport(); scroll(node);
  vi.advanceTimersByTime(600); scroll(node); vi.advanceTimersByTime(600);
  expect(animations).toHaveLength(0); expect(opacity(node)).toBe("1");
  vi.advanceTimersByTime(100); expect(animations).toHaveLength(1);
});

it("reveals immediately during a fade and ignores a stale animation completion", () => {
  dispose = installApplicationScrollbars(document); const node = viewport(); scroll(node);
  vi.advanceTimersByTime(700);
  const first = animations[0]!.animation, oldFinish = first.onfinish;
  scroll(node); expect(first.cancel).toHaveBeenCalledOnce(); expect(opacity(node)).toBe("1");
  oldFinish?.(); expect(opacity(node)).toBe("1");
  vi.advanceTimersByTime(700); expect(animations).toHaveLength(2);
  oldFinish?.(); expect(opacity(node)).toBe("1");
  animations[1]!.animation.onfinish?.(); expect(opacity(node)).toBe("");
});

it("covers later portal content with independent timers for nested viewports", () => {
  dispose = installApplicationScrollbars(document); const outer = viewport(), inner = viewport(outer);
  scroll(inner); vi.advanceTimersByTime(400);
  const portal = viewport(); scroll(portal); vi.advanceTimersByTime(300);
  expect(animations.map(item => item.node)).toEqual([inner]); expect(opacity(outer)).toBe("");
  animations[0]!.animation.onfinish?.(); expect(opacity(portal)).toBe("1");
  vi.advanceTimersByTime(400); expect(animations[1]!.node).toBe(portal);
});

it("respects reduced motion without retaining a visible thumb", () => {
  dispose = installApplicationScrollbars(document); const node = viewport(); scroll(node);
  reduced = true; vi.advanceTimersByTime(700);
  expect(animations).toHaveLength(0); expect(opacity(node)).toBe("");
});

it("still hides the thumb when animations are unavailable or its node was removed", () => {
  dispose = installApplicationScrollbars(document); const node = viewport(), removed = viewport();
  Object.defineProperty(node, "animate", {value: undefined}); scroll(node); scroll(removed); removed.remove();
  vi.advanceTimersByTime(700);
  expect(animations).toHaveLength(0); expect(opacity(node)).toBe(""); expect(opacity(removed)).toBe("");
});

it("releases listeners, timers, animation effects and all paint state on disposal", () => {
  dispose = installApplicationScrollbars(document); const fading = viewport(), pending = viewport();
  scroll(fading); vi.advanceTimersByTime(700); scroll(pending);
  dispose(); dispose = undefined;
  expect(animations[0]!.animation.cancel).toHaveBeenCalledOnce(); expect(vi.getTimerCount()).toBe(0);
  expect(opacity(fading)).toBe(""); expect(opacity(pending)).toBe("");
  expect(document.documentElement.dataset.appScrollbars).toBeUndefined();
  scroll(pending); expect(opacity(pending)).toBe("");
});

it("has one active owner after StrictMode effect cleanup and remount", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = viewport(), root = createRoot(host);
  try {
    await act(async () => root.render(<StrictMode><ApplicationScrollbars /><div /></StrictMode>));
    const node = host.firstElementChild as HTMLElement; scroll(node);
    expect(vi.getTimerCount()).toBe(1);
    await act(async () => root.unmount());
    expect(vi.getTimerCount()).toBe(0); expect(opacity(node)).toBe("");
    expect(document.documentElement.dataset.appScrollbars).toBeUndefined();
  } finally {host.remove();}
});
