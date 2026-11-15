// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { desktopDropClientPosition } from "./desktopDragDrop";

const zoom = vi.hoisted(() => ({ factor: 1 }));
vi.mock("./desktopZoom", () => ({ getDesktopZoomFactor: () => zoom.factor }));
afterEach(() => { zoom.factor = 1; vi.unstubAllGlobals(); });

it.each([
  ["MacIntel", 1, 1, 400, 200], ["MacIntel", 2, 1, 400, 200],
  ["MacIntel", 2, 1.25, 320, 160], ["MacIntel", 2, .5, 800, 400],
  ["Linux x86_64", 2, 1, 400, 200], ["Linux x86_64", 2, 1.25, 320, 160],
  ["Win32", 1, 1, 400, 200], ["Win32", 2, 1, 200, 100],
  // WebView2 devicePixelRatio includes native scaling and page zoom.
  ["Win32", 2.5, 1.25, 160, 80],
])("normalizes %s coordinates at DPR %s and zoom %s", (platform, dpr, factor, x, y) => {
  vi.stubGlobal("navigator", { platform }); vi.stubGlobal("devicePixelRatio", dpr); zoom.factor = factor;
  expect(desktopDropClientPosition({ x: 400, y: 200 })).toEqual({ x, y });
});
