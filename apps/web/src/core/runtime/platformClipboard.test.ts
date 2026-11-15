// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {readClipboardText} from "./platformClipboard";
const mocks=vi.hoisted(() => ({desktop:false,native:vi.fn()}));
vi.mock("./platformCore", () => ({isDesktopRuntime:() => mocks.desktop}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({readText:mocks.native}));
afterEach(() => {mocks.desktop=false; mocks.native.mockReset(); vi.unstubAllGlobals();});
it("uses the native text-only adapter on desktop and browser clipboard in preview",async () => {
  const browser=vi.fn().mockResolvedValue("browser");
  vi.stubGlobal("navigator",{clipboard:{readText:browser}});
  mocks.native.mockResolvedValue("desktop"); mocks.desktop=true;
  expect(await readClipboardText()).toBe("desktop"); expect(browser).not.toHaveBeenCalled();
  mocks.desktop=false; expect(await readClipboardText()).toBe("browser"); expect(mocks.native).toHaveBeenCalledOnce();
});
it("keeps an explicit failure when the browser cannot read text",async () => {
  vi.stubGlobal("navigator",{});
  await expect(readClipboardText()).rejects.toThrow("CLIPBOARD_TEXT_UNAVAILABLE");
});
