/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentMathViewport } from "./AgentMathViewport";

afterEach(() => vi.restoreAllMocks());

it("remeasures overflow after viewport or font changes and releases observation on unmount", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const disconnect = vi.fn(), observe = vi.fn();
  let resize = () => {}, fontsReady!: () => void;
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resize = callback; }
    observe = observe; disconnect = disconnect;
  });
  const previousFonts = Object.getOwnPropertyDescriptor(document, "fonts");
  Object.defineProperty(document, "fonts", { configurable: true, value: { ready: new Promise<void>(resolve => { fontsReady = resolve; }) } });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AgentMathViewport display={false}><span className="katex">x</span></AgentMathViewport>));
    const viewport = host.firstElementChild!;
    let width = 100, contentWidth = 80;
    vi.spyOn(viewport, "getBoundingClientRect").mockImplementation(() => new DOMRect(0, 0, width, 20));
    vi.spyOn(viewport.firstElementChild!, "getBoundingClientRect").mockImplementation(() => new DOMRect(0, 0, contentWidth, 20));
    await act(async () => resize());
    expect(viewport.hasAttribute("tabindex")).toBe(false);
    expect(observe.mock.calls.map(([element]) => element)).toEqual([viewport, viewport.firstElementChild]);
    contentWidth = 240;
    await act(async () => { fontsReady(); });
    expect(viewport.getAttribute("data-overflow")).toBe("true");
    expect(viewport.getAttribute("tabindex")).toBe("0");
    expect(viewport.getAttribute("aria-label")).toBe("公式，可左右滚动");
    width = 300;
    await act(async () => resize());
    expect(viewport.getAttribute("data-overflow")).toBe("false");
    expect(viewport.hasAttribute("role")).toBe(false);
    await act(async () => root.unmount());
    expect(disconnect).toHaveBeenCalledOnce();
    await act(async () => resize());
  } finally {
    await act(async () => root.unmount()); host.remove();
    if (previousFonts) Object.defineProperty(document, "fonts", previousFonts);
    else Reflect.deleteProperty(document, "fonts");
    vi.unstubAllGlobals();
  }
});

it.each([false, true])("uses whole-formula geometry and remains stable across resize (display=%s)", async display => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  let container = 14.25, intrinsic = 14.25;
  const callbacks: ResizeObserverCallback[] = [];
  class Observer {
    constructor(callback: ResizeObserverCallback) { callbacks.push(callback); }
    observe() {}
    disconnect() {}
    unobserve() {}
  }
  vi.stubGlobal("ResizeObserver", Observer);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
    return new DOMRect(0, 0, this.classList.contains("katex") ? intrinsic : container, 20);
  });
  // Real KaTeX glyph/script ink can protrude past its inline box. This is not
  // a formula wider than the text container, even with integer rounding.
  vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockReturnValue(18);
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(14);
  try {
    await act(async () => root.render(<AgentMathViewport display={display}><span className="katex">formula</span></AgentMathViewport>));
    const viewport = host.firstElementChild!;
    const resize = async () => act(async () => {
      for (const callback of callbacks) callback([], {} as ResizeObserver);
    });
    expect(viewport.getAttribute("data-overflow")).toBe("false");
    for (let i = 0; i < 4; i++) await resize();
    expect(viewport.hasAttribute("role")).toBe(false);
    intrinsic = 900; container = 320; await resize();
    expect(viewport.getAttribute("data-overflow")).toBe("true");
    expect(viewport.getAttribute("role")).toBe("region");
    expect(viewport.getAttribute("tabindex")).toBe("0");
    // Reflow to a full-width scroll row must not switch overflow off.
    for (let i = 0; i < 4; i++) await resize();
    expect(viewport.getAttribute("data-overflow")).toBe("true");
    container = 1000; await resize();
    expect(viewport.getAttribute("data-overflow")).toBe("false");
    expect(viewport.hasAttribute("tabindex")).toBe(false);
  } finally {
    await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();
  }
});
