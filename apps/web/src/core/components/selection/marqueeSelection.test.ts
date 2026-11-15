// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { startMarqueeSelection, type MarqueePoint } from "./marqueeSelection";

let pane: HTMLElement, dispose: (() => void) | undefined, frameId: number;
const frames = new Map<number, FrameRequestCallback>();
const pointer = (type: string, y: number) => new PointerEvent(type, {pointerId: 1, buttons: type === "pointerup" ? 0 : 1,
  clientX: 100, clientY: y, bubbles: true, cancelable: true});
function paint() {
  const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0));
}
beforeEach(() => {
  frameId = 0; frames.clear();
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frames.set(++frameId, callback); return frameId; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { frames.delete(id); });
  pane = document.createElement("div"); document.body.append(pane);
  pane.setPointerCapture = vi.fn(); pane.hasPointerCapture = () => false;
  vi.spyOn(pane, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 500, 500));
});
afterEach(() => { dispose?.(); dispose = undefined; pane.remove(); vi.restoreAllMocks(); });

it("coalesces a burst into one hit test per paint and skips unchanged selection delivery", () => {
  const getIds = vi.fn((_start: MarqueePoint, end: MarqueePoint) => end.clientY < 200 ? ["a"] : ["a", "b"]), onSelect = vi.fn();
  dispose = startMarqueeSelection(pane, pointer("pointerdown", 50), {getIds, onSelect, onActivate: vi.fn(), onFinish: vi.fn()});
  for (let y = 60; y < 100; y++) window.dispatchEvent(pointer("pointermove", y));
  expect(getIds).not.toHaveBeenCalled();
  paint(); expect(getIds).toHaveBeenCalledTimes(1); expect(onSelect).toHaveBeenCalledExactlyOnceWith(["a"]);
  window.dispatchEvent(pointer("pointermove", 150)); paint();
  expect(getIds).toHaveBeenCalledTimes(2); expect(onSelect).toHaveBeenCalledTimes(1);
  paint(); expect(getIds).toHaveBeenCalledTimes(2);
  // Release before the next paint must commit the actual final rectangle.
  window.dispatchEvent(pointer("pointermove", 210)); window.dispatchEvent(pointer("pointerup", 250));
  expect(getIds).toHaveBeenCalledTimes(3); expect(onSelect).toHaveBeenLastCalledWith(["a", "b"]);
  expect(frames.size).toBe(0); expect(document.querySelector(".row-selection-marquee")).toBeNull();
});

it("updates on scrolling without pointer movement and still delivers an empty selection", () => {
  const getIds = vi.fn((start: MarqueePoint) => start.clientY < 0 ? ["scrolled"] : []), onSelect = vi.fn();
  dispose = startMarqueeSelection(pane, pointer("pointerdown", 50), {getIds, onSelect, onActivate: vi.fn(), onFinish: vi.fn()});
  window.dispatchEvent(pointer("pointermove", 100)); paint();
  expect(onSelect).toHaveBeenCalledExactlyOnceWith([]);
  pane.scrollTop = 80; pane.dispatchEvent(new Event("scroll")); paint();
  expect(onSelect).toHaveBeenLastCalledWith(["scrolled"]);
  expect(document.querySelector<HTMLElement>(".row-selection-marquee")!.style.top).toBe("0px");
});

it("leaves no idle frame or layout reads when stationary, and resumes on input", () => {
  const getIds = vi.fn(() => ["a"]);
  dispose = startMarqueeSelection(pane, pointer("pointerdown", 100), {autoScroll: true, getIds, onSelect: vi.fn(), onActivate: vi.fn(), onFinish: vi.fn()});
  window.dispatchEvent(pointer("pointermove", 150)); paint();
  expect(frames.size).toBe(0);
  expect(pane.getBoundingClientRect).toHaveBeenCalledTimes(1);
  window.dispatchEvent(pointer("pointermove", 150));
  expect(frames.size).toBe(0);
  paint(); paint();
  expect(getIds).toHaveBeenCalledTimes(1);
  expect(pane.getBoundingClientRect).toHaveBeenCalledTimes(1);
  window.dispatchEvent(pointer("pointermove", 170)); paint();
  expect(getIds).toHaveBeenCalledTimes(2); expect(frames.size).toBe(0);
});

it("stops requesting edge frames at the scroll limit and retains final release", () => {
  Object.defineProperty(pane, "scrollTop", {get: () => 0, set: () => undefined, configurable: true});
  const onSelect = vi.fn();
  dispose = startMarqueeSelection(pane, pointer("pointerdown", 100), {autoScroll: true,
    getIds: (_start, end) => end.clientY > 400 ? ["edge"] : [], onSelect, onActivate: vi.fn(), onFinish: vi.fn()});
  window.dispatchEvent(pointer("pointermove", 490)); paint();
  expect(frames.size).toBe(0); expect(onSelect).toHaveBeenCalledExactlyOnceWith(["edge"]);
  window.dispatchEvent(pointer("pointerup", 200));
  expect(onSelect).toHaveBeenLastCalledWith([]);
});

it("combines pointer work and edge scrolling into one detection per frame", () => {
  const getIds = vi.fn(() => [String(pane.scrollTop)]), onSelect = vi.fn();
  dispose = startMarqueeSelection(pane, pointer("pointerdown", 100), {autoScroll: true, getIds, onSelect, onActivate: vi.fn(), onFinish: vi.fn()});
  window.dispatchEvent(pointer("pointermove", 490)); paint();
  expect(pane.scrollTop).toBe(12); expect(getIds).toHaveBeenCalledTimes(1);
  // Staying at the edge continues the gesture without another pointer event.
  paint();
  expect(pane.scrollTop).toBe(24); expect(getIds).toHaveBeenCalledTimes(2);
  expect(onSelect).toHaveBeenLastCalledWith(["24"]);
});

it.each(["blur", "pointercancel", "Escape"])("cancels pending work before paint on %s", reason => {
  const getIds = vi.fn(() => ["a"]), onFinish = vi.fn();
  dispose = startMarqueeSelection(pane, pointer("pointerdown", 50), {getIds, onSelect: vi.fn(), onActivate: vi.fn(), onFinish});
  window.dispatchEvent(pointer("pointermove", 100));
  window.dispatchEvent(reason === "Escape" ? new KeyboardEvent("keydown", {key: "Escape"})
    : reason === "pointercancel" ? pointer("pointercancel", 100) : new Event(reason));
  paint();
  expect(getIds).not.toHaveBeenCalled(); expect(onFinish).toHaveBeenCalledExactlyOnceWith(true, true);
  expect(frames.size).toBe(0);
});
