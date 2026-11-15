/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {useWritingAgentSize} from "./useWritingAgentSize";

let root: Root, host: HTMLElement, panel: HTMLElement;
const capture = vi.fn(), release = vi.fn();
function Harness({rightEdgeEnabled = true}: {rightEdgeEnabled?: boolean}) {
  const size = useWritingAgentSize("standard", rightEdgeEnabled);
  return <section {...size.edgeResizeProps} style={{border: "1px solid black"}}
    data-width={size.widthPreset} data-height={size.heightPreset} data-resizing={size.widthResizing}>
    <div className="scroll-content">Messages and native scrollbar</div>
  </section>;
}
beforeEach(async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); capture.mockClear(); release.mockClear();
  vi.spyOn(Storage.prototype, "getItem").mockReturnValue(null);
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {});
  Object.defineProperty(window, "innerWidth", {configurable: true, value: 1280});
  Object.defineProperty(window, "innerHeight", {configurable: true, value: 800});
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<Harness />)); panel = host.firstElementChild as HTMLElement;
  Object.defineProperty(panel, "offsetWidth", {value: 640});
  panel.getBoundingClientRect = () => ({left: 100, right: 740, top: 100, bottom: 500, width: 640, height: 400} as DOMRect);
  panel.setPointerCapture = capture; panel.releasePointerCapture = release;
});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
async function pointer(target: Element, type: string, x: number, y = 250, button = 0): Promise<PointerEvent> {
  const event = new PointerEvent(type, {bubbles: true, cancelable: true, pointerId: 1, clientX: x, clientY: y, button});
  await act(async () => {target.dispatchEvent(event);}); return event;
}
it("resizes from the actual right border and continues through pointer capture", async () => {
  await pointer(panel, "pointermove", 739.5); expect(panel.dataset.resizeEdge).toBe("width");
  expect((await pointer(panel, "pointerdown", 739.5)).defaultPrevented).toBe(true);
  expect(capture).toHaveBeenCalledWith(1); expect(panel.dataset.resizing).toBe("true");
  await pointer(panel, "pointermove", 900); expect(panel.dataset.width).toBe("comfortable");
  await pointer(panel, "pointerup", 900); expect(release).toHaveBeenCalledWith(1);
  expect(panel.dataset.resizing).toBe("false");
});
it("leaves the previous 10px interior band and bubbled scrollbar/content input alone", async () => {
  await pointer(panel, "pointerdown", 735); await pointer(panel, "pointermove", 900);
  expect(panel.dataset.width).toBe("standard"); expect(capture).not.toHaveBeenCalled();
  const content = panel.firstElementChild!;
  expect((await pointer(content, "pointerdown", 739.5)).defaultPrevented).toBe(false);
  await pointer(content, "pointermove", 900);
  expect(panel.dataset.width).toBe("standard"); expect(capture).not.toHaveBeenCalled();
});
it("uses the same presets for the left border and the upper corners", async () => {
  await pointer(panel, "pointerdown", 100.5); await pointer(panel, "pointermove", -50);
  expect(panel.dataset.width).toBe("comfortable"); await pointer(panel, "pointerup", -50);
  await pointer(panel, "pointermove", 739.5, 105); expect(panel.dataset.resizeEdge).toBe("right");
  await pointer(panel, "pointerdown", 739.5, 105); await pointer(panel, "pointermove", 900, 0);
  expect(panel.dataset.height).toBe("comfortable"); await pointer(panel, "pointercancel", 900, 0);
  expect(panel.dataset.resizing).toBe("false");
});
it("does not resize from secondary clicks and clears the resize cursor on entry to content", async () => {
  await pointer(panel, "pointerdown", 739.5, 250, 2); expect(capture).not.toHaveBeenCalled();
  await pointer(panel, "pointermove", 739.5); expect(panel.dataset.resizeEdge).toBe("width");
  await pointer(panel.firstElementChild!, "pointermove", 739.5); expect(panel.dataset.resizeEdge).toBeUndefined();
});
it("disables the right edge and upper-right corner when docked right while keeping the left edge usable", async () => {
  await act(async () => root.render(<Harness rightEdgeEnabled={false} />));
  await pointer(panel, "pointermove", 739.5); expect(panel.dataset.resizeEdge).toBeUndefined();
  await pointer(panel, "pointerdown", 739.5); await pointer(panel, "pointermove", 900);
  await pointer(panel, "pointerdown", 739.5, 105); await pointer(panel, "pointermove", 900, 0);
  expect(panel.dataset.width).toBe("standard"); expect(panel.dataset.height).toBe("standard");
  expect(capture).not.toHaveBeenCalled();
  await pointer(panel, "pointerdown", 100.5); await pointer(panel, "pointermove", -50);
  expect(panel.dataset.width).toBe("comfortable"); await pointer(panel, "pointerup", -50);
  await act(async () => root.render(<Harness />));
  await pointer(panel, "pointerdown", 739.5); expect(capture).toHaveBeenCalledTimes(2);
});
it("stops a captured right-edge resize if that edge becomes disabled", async () => {
  await pointer(panel, "pointerdown", 739.5);
  await act(async () => root.render(<Harness rightEdgeEnabled={false} />));
  await pointer(panel, "pointermove", 900);
  expect(panel.dataset.width).toBe("standard"); expect(panel.dataset.resizing).toBe("false");
  expect(release).toHaveBeenCalledWith(1);
});
