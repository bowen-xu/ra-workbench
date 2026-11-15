// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {readerCommandForKey} from "@research-assistant/core/reader";
import {getDesktopZoomFactor, installDesktopZoom} from "./desktopZoom";
import {selectAppStorage} from "./appStorage";
import {readWorkspaceState,writeWorkspaceState} from "./workspaceState";

const native = vi.hoisted(() => ({desktop: true, setZoom: vi.fn()}));
vi.mock("./platformCore", () => ({isDesktopRuntime: () => native.desktop}));
vi.mock("@tauri-apps/api/webview", () => ({getCurrentWebview: () => ({setZoom: native.setZoom})}));
let stop: (() => void) | undefined;
beforeEach(() => {
  localStorage.clear();selectAppStorage(localStorage);
  native.desktop = true;
  native.setZoom.mockReset().mockResolvedValue(undefined);
  vi.stubGlobal("navigator", {platform: "MacIntel"});
});
it("restores only successfully applied zoom after restarting",async()=>{
  writeWorkspaceState("zoom",130);
  stop=installDesktopZoom({onError:vi.fn()});
  await vi.waitFor(()=>expect(native.setZoom).toHaveBeenCalledWith(1.3));
  native.setZoom.mockRejectedValueOnce(new Error("denied"));
  key("+");await vi.waitFor(()=>expect(native.setZoom).toHaveBeenCalledWith(1.4));
  await Promise.resolve();
  expect(readWorkspaceState("zoom",0,Number)).toBe(130);
  stop();native.setZoom.mockClear();
  stop=installDesktopZoom({onError:vi.fn()});
  await vi.waitFor(()=>expect(native.setZoom).toHaveBeenCalledExactlyOnceWith(1.3));
});
afterEach(() => {stop?.(); stop = undefined; document.body.replaceChildren(); vi.unstubAllGlobals();});
function key(value: string, options: KeyboardEventInit = {}, target: EventTarget = window) {
  const event = new KeyboardEvent("keydown", {key: value, metaKey: true, bubbles: true, cancelable: true, ...options});
  target.dispatchEvent(event);
  return event;
}
async function ready(onError = vi.fn()) {
  stop = installDesktopZoom({onError});
  await vi.waitFor(() => expect(native.setZoom).toHaveBeenCalledWith(1));
  native.setZoom.mockClear();
  return onError;
}

it("zooms the entire WebView for Cmd +/=/- and resets to actual size for Cmd 0", async () => {
  await ready();
  for (const value of ["=", "+", "-", "0"]) expect(key(value).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(native.setZoom.mock.calls).toEqual([[1.1], [1.2], [1.1], [1]]));
});
it("exposes only confirmed native page zoom to file hit testing", async () => {
  const onError = await ready();
  let complete: (() => void) | undefined;
  native.setZoom.mockImplementationOnce(() => new Promise<void>(resolve => { complete = resolve; }));
  key("=");
  await vi.waitFor(() => expect(complete).toBeTypeOf("function"));
  expect(getDesktopZoomFactor()).toBe(1);
  complete?.();
  await vi.waitFor(() => expect(getDesktopZoomFactor()).toBe(1.1));
  native.setZoom.mockRejectedValueOnce(new Error("zoom denied"));
  key("=");
  await vi.waitFor(() => expect(onError).toHaveBeenCalled());
  expect(getDesktopZoomFactor()).toBe(1.1);
  key("0");
  await vi.waitFor(() => expect(getDesktopZoomFactor()).toBe(1));
});
it("captures zoom before focused editors and PDF handlers without consuming ordinary typing", async () => {
  await ready();
  const input = document.createElement("textarea"), local = vi.fn(), pdf = vi.fn();
  document.body.append(input); input.focus(); input.addEventListener("keydown", local);
  document.addEventListener("keydown", pdf);
  try {
    key("=", {}, input);
    expect(local).not.toHaveBeenCalled(); expect(pdf).not.toHaveBeenCalled();
    expect(key("a", {}, input).defaultPrevented).toBe(false);
    expect(local).toHaveBeenCalledOnce(); expect(pdf).toHaveBeenCalledOnce();
  } finally {document.removeEventListener("keydown", pdf);}
});
it("routes shifted zoom chords to PDF commands while unshifted chords zoom the WebView", async () => {
  await ready();
  const viewer = document.createElement("div"), commands: string[] = [];
  document.body.append(viewer);
  const pdf = (event: KeyboardEvent) => {
    const command = readerCommandForKey({key: event.key, code: event.code, platform: "macos",
      metaKey: event.metaKey, shiftKey: event.shiftKey});
    if (command) {commands.push(command); event.preventDefault();}
  };
  document.addEventListener("keydown", pdf);
  try {
    key("=", {code: "Equal"}, viewer);
    for (const [value, code] of [["+", "Equal"], ["_", "Minus"], [")", "Digit0"]] as const) {
      expect(key(value, {code, shiftKey: true}, viewer).defaultPrevented).toBe(true);
    }
    expect(commands).toEqual(["reader.zoom.in", "reader.zoom.out", "reader.zoom.fitWidth"]);
    expect(key("+", {code: "Equal", shiftKey: true}).defaultPrevented).toBe(false);
    await vi.waitFor(() => expect(native.setZoom).toHaveBeenCalledExactlyOnceWith(1.1));
  } finally {document.removeEventListener("keydown", pdf);}
});
it("uses Ctrl on Windows/Linux and ignores composition or unrelated modifier combinations", async () => {
  vi.stubGlobal("navigator", {platform: "Win32"});
  await ready();
  for (const options of [{}, {ctrlKey: true}, {metaKey: false, ctrlKey: true, altKey: true},
    {metaKey: false, ctrlKey: true, isComposing: true}, {metaKey: false, ctrlKey: true, keyCode: 229}]) {
    expect(key("=", options).defaultPrevented).toBe(false);
  }
  expect(key("=", {metaKey: false, ctrlKey: true}).defaultPrevented).toBe(true);
  await vi.waitFor(() => expect(native.setZoom).toHaveBeenCalledExactlyOnceWith(1.1));
});
it("keeps repeat input within 50–200 percent and resets from either bound", async () => {
  await ready();
  for (let i = 0; i < 30; i++) key("=");
  key("0");
  for (let i = 0; i < 30; i++) key("-");
  key("0");
  await vi.waitFor(() => expect(native.setZoom).toHaveBeenCalledTimes(17));
  const scales = native.setZoom.mock.calls.map(([scale]) => scale);
  expect(Math.max(...scales)).toBe(2); expect(Math.min(...scales)).toBe(.5); expect(scales.at(-1)).toBe(1);
});
it("serializes native requests and reports a failure before accepting another zoom step", async () => {
  const onError = await ready();
  native.setZoom.mockRejectedValueOnce(new Error("permission denied"));
  key("=");
  await vi.waitFor(() => expect(onError).toHaveBeenCalledWith(expect.stringContaining("desktop_zoom_failed")));
  expect(onError.mock.calls[0]?.[0]).toContain("permission denied");
  key("=");
  await vi.waitFor(() => expect(native.setZoom.mock.calls).toEqual([[1.1], [1.1]]));
});
it("does not let a delayed zoom overwrite a subsequent reset", async () => {
  await ready();
  let complete!: () => void;
  native.setZoom.mockImplementationOnce(() => new Promise<void>(resolve => {complete = resolve;}));
  key("="); key("="); key("0");
  await vi.waitFor(() => expect(native.setZoom).toHaveBeenCalledExactlyOnceWith(1.1));
  complete();
  await vi.waitFor(() => expect(native.setZoom.mock.calls).toEqual([[1.1], [1.2], [1]]));
});
it("leaves browser/preview zoom and wheel gestures to their original owners", async () => {
  native.desktop = false;
  stop = installDesktopZoom({onError: vi.fn()});
  expect(key("=").defaultPrevented).toBe(false);
  expect(native.setZoom).not.toHaveBeenCalled();
  stop(); native.desktop = true;
  await ready();
  const wheel = new WheelEvent("wheel", {ctrlKey: true, deltaY: -20, cancelable: true});
  window.dispatchEvent(wheel);
  expect(wheel.defaultPrevented).toBe(false); expect(native.setZoom).not.toHaveBeenCalled();
});
it("removes its keyboard listener and skips queued work when the App is disposed", async () => {
  await ready();
  key("="); stop?.();
  await Promise.resolve();
  expect(key("=").defaultPrevented).toBe(false);
  expect(native.setZoom).not.toHaveBeenCalled();
});
