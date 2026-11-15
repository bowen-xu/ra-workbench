// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { getAppRuntime, selectAppRuntime } from "./appRuntime";
import { listen } from "./platformEvents";
const native = vi.hoisted(() => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: native.listen }));
const previous = getAppRuntime();
afterEach(() => { selectAppRuntime(previous); vi.clearAllMocks(); });

it("uses the startup-selected HTTP event port in native preview", async () => {
  const port = { listen: vi.fn(async () => () => {}) };
  selectAppRuntime({ mode: "preview", storage: previous.storage, gateways: {}, events: port, dispose() {} });
  const handler = vi.fn(); await listen("research-writing-file-delivery", handler);
  expect(port.listen).toHaveBeenCalledWith("research-writing-file-delivery", handler);
  expect(native.listen).not.toHaveBeenCalled();
});
it("retains the native production event subscription", async () => {
  selectAppRuntime({ mode: "desktop", storage: previous.storage, gateways: {}, dispose() {} });
  const handler = vi.fn(); await listen("research-note-text-flush", handler);
  expect(native.listen).toHaveBeenCalledWith("research-note-text-flush", handler);
});
it("fails closed when a mock preview did not supply the event port", async () => {
  selectAppRuntime({ mode: "preview", storage: previous.storage, gateways: {}, dispose() {} });
  await expect(listen("unconfigured", vi.fn())).rejects.toThrow("预览未覆盖平台事件");
  expect(native.listen).not.toHaveBeenCalled();
});
