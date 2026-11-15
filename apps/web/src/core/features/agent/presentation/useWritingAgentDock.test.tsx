/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useWritingAgentDock } from "./useWritingAgentDock";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
const storage = new Map<string, string>();
const key = "research-assistant.writing-agent.panel-dock";
let clock = 0, frameId = 0;
const frames = new Map<number, FrameRequestCallback>();
function Harness({ open = true }: { open?: boolean }) {
  const dock = useWritingAgentDock(open);
  return <section ref={dock.panelRef} data-dock={dock.dock} data-dragging={dock.dragging} data-offset={dock.dragOffset}>
    <textarea defaultValue="保留草稿" /><button {...dock.handleProps}>拖动</button>
  </section>;
}
beforeEach(async () => {
  storage.clear();
  clock = 0; frameId = 0; frames.clear();
  vi.spyOn(performance, "now").mockImplementation(() => clock);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frames.set(++frameId, callback); return frameId; });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => { frames.delete(id); });
  Object.defineProperty(window, "localStorage", { configurable: true, value: {
    getItem: (name: string) => storage.get(name) ?? null,
    setItem: (name: string, value: string) => storage.set(name, value),
  } });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<Harness />));
  const panel = host.querySelector("section")!;
  Object.defineProperty(panel, "offsetParent", { configurable: true, value: host });
  host.getBoundingClientRect = () => ({ left: 0, width: 1000 } as DOMRect);
  panel.getBoundingClientRect = () => ({ left: panel.dataset.dock === "right" ? 582 : 300, width: 400 } as DOMRect);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });
async function frame(elapsed: number) {
  clock += elapsed;
  const callbacks = [...frames.values()]; frames.clear();
  await act(async () => callbacks.forEach(callback => callback(clock)));
}
async function pointer(type: string, x: number, id = 1, settle = true) {
  await act(async () => host.querySelector("button")!.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: id, button: 0, clientX: x })));
  if (settle) await frame(220);
}
it("responds immediately on the same side and only animates the attraction across sides", async () => {
  const panel = host.querySelector("section")!;
  await pointer("pointerdown", 600, 1, false);
  await pointer("pointermove", 700, 1, false);
  expect(panel.dataset.offset).toBe("50");
  expect(frames.size).toBe(0);
  await pointer("pointermove", 780, 1, false);
  expect(panel.dataset.offset).toBe("90"); // Pointer moves now; the attraction hasn't jumped.
  expect(frames.size).toBe(1);
  await frame(55);
  const crossing = Number(panel.dataset.offset);
  expect(crossing).toBeGreaterThan(90); expect(crossing).toBeLessThan(231);
  await pointer("pointermove", 800, 1, false);
  expect(Number(panel.dataset.offset)).toBe(crossing + 10);
  expect(frames.size).toBe(1);
  await frame(165); // Ordinary pointer moves did not restart the 220ms transition.
  expect(panel.dataset.offset).toBe("241");
  expect(frames.size).toBe(0);
});
it("stops a side transition on release, cancel, hiding and unmount", async () => {
  const panel = host.querySelector("section")!;
  for (const reason of ["pointerup", "pointercancel", "lostpointercapture"]) {
    await act(async () => host.querySelector("button")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })));
    await pointer("pointerdown", 600, 1, false); await pointer("pointermove", 780, 1, false);
    expect(frames.size).toBe(1);
    await pointer(reason, 780, 1, false);
    expect(frames.size).toBe(0); expect(panel.dataset.offset).toBe("0");
    await frame(220);
    expect(panel.dataset.offset).toBe("0");
  }
  await pointer("pointerdown", 600, 1, false); await pointer("pointermove", 780, 1, false);
  await act(async () => root.render(<Harness open={false} />));
  expect(frames.size).toBe(0);
  await act(async () => root.render(<Harness />));
  await pointer("pointerdown", 600, 1, false); await pointer("pointermove", 780, 1, false);
  await act(async () => root.unmount());
  expect(frames.size).toBe(0);
});
it("switches attraction immediately when reduced motion is requested", async () => {
  vi.spyOn(window, "matchMedia").mockReturnValue({ matches: true } as MediaQueryList);
  await pointer("pointerdown", 600, 1, false); await pointer("pointermove", 780, 1, false);
  expect(host.querySelector("section")?.dataset.offset).toBe("231");
  expect(frames.size).toBe(0);
});
it("resists toward each side, snaps both ways, and preserves the composer", async () => {
  const panel = host.querySelector("section")!, input = host.querySelector("textarea");
  const handle = host.querySelector("button")!;
  handle.setPointerCapture = vi.fn();
  await pointer("pointerdown", 600);
  expect(handle.setPointerCapture).toHaveBeenCalledWith(1);
  await pointer("pointermove", 750, 2);
  expect(panel.dataset.offset).toBe("0");
  await pointer("pointermove", 700);
  expect(panel.dataset.offset).toBe("50"); // Pointer center 600, attracted to center 500.
  await pointer("pointermove", 780);
  expect(panel.dataset.offset).toBe("231"); // Pointer center 680, attracted to right 782.
  await pointer("pointerup", 900);
  expect(panel.dataset.dock).toBe("right"); expect(panel.dataset.offset).toBe("0");
  expect(storage.get(key)).toBe("right");
  await pointer("pointerdown", 800);
  await pointer("pointermove", 700);
  expect(panel.dataset.offset).toBe("-50");
  await pointer("pointermove", 600);
  expect(panel.dataset.offset).toBe("-241");
  await pointer("pointerup", 600);
  expect(panel.dataset.dock).toBe("center");
  expect(host.querySelector("textarea")).toBe(input); expect(input?.value).toBe("保留草稿");
});
it("keeps attraction through jitter until the opposite switching threshold is crossed", async () => {
  const panel = host.querySelector("section")!;
  // Dock centers 500 and 782: switch right above 655.1 (55%), left below 626.9 (45%).
  await pointer("pointerdown", 600);
  await pointer("pointermove", 740);
  expect(panel.dataset.offset).toBe("70");
  await pointer("pointermove", 750);
  expect(panel.dataset.offset).toBe("75");
  await pointer("pointermove", 755);
  expect(panel.dataset.offset).toBe("77.5");
  await pointer("pointermove", 756);
  expect(panel.dataset.offset).toBe("219");
  for (const x of [740, 742, 740]) {
    await pointer("pointermove", x);
    expect(Number(panel.dataset.offset)).toBeGreaterThan(200);
  }
  await pointer("pointermove", 727);
  expect(panel.dataset.offset).toBe("204.5");
  await pointer("pointermove", 726);
  expect(panel.dataset.offset).toBe("63");
  await pointer("pointerup", 740);
  expect(panel.dataset.dock).toBe("center"); expect(panel.dataset.offset).toBe("0");
});
it("releases to the retained side inside the dead band and clamps motion to the anchors", async () => {
  const panel = host.querySelector("section")!;
  await pointer("pointerdown", 600); await pointer("pointermove", 100);
  expect(panel.dataset.offset).toBe("0");
  await pointer("pointermove", 1600);
  expect(panel.dataset.offset).toBe("282");
  await pointer("pointerup", 741);
  expect(panel.dataset.dock).toBe("right");
  await pointer("pointerdown", 600); await pointer("pointerup", 900);
  await pointer("pointerdown", 800); await pointer("pointermove", 1600);
  expect(panel.dataset.offset).toBe("0");
  await pointer("pointermove", 100);
  expect(panel.dataset.offset).toBe("-282");
  await pointer("pointerup", 659);
  expect(panel.dataset.dock).toBe("center");
});
it("preserves the visible grab position when another drag interrupts a settling animation", async () => {
  const panel = host.querySelector("section")!;
  await pointer("pointerdown", 600); await pointer("pointerup", 900);
  panel.getBoundingClientRect = () => ({ left: 500, width: 400 } as DOMRect);
  await pointer("pointerdown", 800);
  expect(panel.dataset.offset).toBe("-82"); // Visible center 700, persisted right anchor 782.
  await pointer("pointermove", 780);
  expect(panel.dataset.offset).toBe("-51"); // Attraction midpoint of 680 and 782.
  await pointer("pointercancel", 780);
  expect(panel.dataset.dock).toBe("right"); expect(panel.dataset.offset).toBe("0");
});
it("cancels interrupted drags and ignores a click without movement", async () => {
  const panel = host.querySelector("section")!;
  for (const reason of ["pointercancel", "lostpointercapture"]) {
    await pointer("pointerdown", 600); await pointer("pointermove", 900); await pointer(reason, 900);
    expect(panel.dataset.dock).toBe("center"); expect(panel.dataset.offset).toBe("0");
    expect(panel.dataset.dragging).toBe("false");
  }
  await pointer("pointerdown", 600); await pointer("pointerup", 600);
  expect(panel.dataset.dock).toBe("center");
});
it("supports keyboard placement and restores it when remounted", async () => {
  await act(async () => host.querySelector("button")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true })));
  await act(async () => root.render(<Harness key="new-session" />));
  expect(host.querySelector("section")?.dataset.dock).toBe("right");
  await act(async () => host.querySelector("button")!.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true })));
  expect(host.querySelector("section")?.dataset.dock).toBe("center");
});
