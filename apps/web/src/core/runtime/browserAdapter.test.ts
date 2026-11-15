// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AppRuntime } from "./appRuntime";
import { isWebUrl, openBrowserPage } from "./browserAdapter";
import capabilities from "../../../../desktop/src-tauri/capabilities/main.json";

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn(async () => {}) }));
const runtime = (mode: AppRuntime["mode"], browser?: AppRuntime["browser"]): AppRuntime => ({
  mode, ...(browser ? {browser} : {}), storage: localStorage, gateways: {}, dispose() {},
});
afterEach(() => { vi.clearAllMocks(); vi.restoreAllMocks(); });

it("opens arbitrary HTTP/HTTPS domains through the system port on desktop", async () => {
  await openBrowserPage("https://arxiv.org/abs/1234#page=2", runtime("desktop"));
  await openBrowserPage("http://localhost:9000/paper", runtime("desktop"));
  expect(openUrl).toHaveBeenNthCalledWith(1, "https://arxiv.org/abs/1234#page=2");
  expect(openUrl).toHaveBeenNthCalledWith(2, "http://localhost:9000/paper");
  const permission = capabilities.permissions.find(value => typeof value === "object" && value.identifier === "opener:allow-open-url");
  expect(permission).toMatchObject({ allow: [{ url: "https://*" }, { url: "http://*" }] });
});

it.each(["browser", "preview"] as const)("uses a detached browser tab in %s", async mode => {
  const open = vi.spyOn(window, "open").mockReturnValue(null);
  await openBrowserPage("https://example.org/论文", runtime(mode));
  expect(open).toHaveBeenCalledWith("https://example.org/%E8%AE%BA%E6%96%87", "_blank", "noopener,noreferrer");
  expect(openUrl).not.toHaveBeenCalled();
});

it("supports a replacement browser without changing link consumers", async () => {
  const open = vi.fn(async () => {});
  await openBrowserPage("https://example.net/", runtime("desktop", { open }));
  expect(open).toHaveBeenCalledExactlyOnceWith("https://example.net/");
  expect(openUrl).not.toHaveBeenCalled();
});

it.each(["source://notes/n", "file:///tmp/test", "javascript:alert(1)", "data:text/html,test", "#page=2", "../relative", "mailto:a@example.org", "https://"])("does not treat %s as a browser page", async url => {
  const open = vi.fn(async () => {});
  expect(isWebUrl(url)).toBe(false);
  await expect(openBrowserPage(url, runtime("desktop", { open }))).rejects.toThrow("BROWSER_INVALID_URL");
  expect(open).not.toHaveBeenCalled();
});

it("retains the original opener cause and technical code without echoing URL credentials", async () => {
  const cause = new Error("OS error 5: https://user:password@example.org/a?token=private#secret");
  const failure = await openBrowserPage("https://example.org", runtime("desktop", { open: async () => { throw cause; } }))
    .then(() => { throw new Error("Expected failure"); }, (error: Error) => error);
  expect(failure.cause).toBe(cause);
  expect(failure.message).toContain("[BROWSER_OPEN_FAILED]");
  expect(failure.message).toContain("OS error 5: https://example.org/a");
  expect(failure.message).not.toMatch(/password|private|secret/);
});
