import { beforeEach, describe, expect, it, vi } from "vitest";

import { getBrowserConnectorGateway, listenBrowserCaptures } from "./browserConnectorGateway";

const native = vi.hoisted(() => ({ desktop: true, invoke: vi.fn(), listen: vi.fn() }));
vi.mock("../runtime/platformCore", () => ({
  isDesktopRuntime: () => native.desktop,
  invoke: native.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: native.listen }));

beforeEach(() => { vi.clearAllMocks(); native.desktop = true; });

describe("Chrome connector platform boundary", () => {
  it("never accesses native status or events in browser and preview modes", async () => {
    native.desktop = false;
    const gateway = getBrowserConnectorGateway();
    expect((await gateway.getStatus()).available).toBe(false);
    const stop = await listenBrowserCaptures(vi.fn()); stop();
    expect(native.invoke).not.toHaveBeenCalled();
    expect(native.listen).not.toHaveBeenCalled();
  });

  it("only reads receiver status and validates its response", async () => {
    const gateway = getBrowserConnectorGateway();
    native.invoke.mockResolvedValue({ endpoint: "http://127.0.0.1:23120", available: true, message: "可连接" });
    expect((await gateway.getStatus()).available).toBe(true);
    expect(native.invoke).toHaveBeenCalledExactlyOnceWith("get_browser_connector_status");
    native.invoke.mockResolvedValue({ available: "yes" });
    await expect(gateway.getStatus()).rejects.toThrow("连接状态");
  });

  it("forwards only recognized capture events and preserves unsubscribe ownership", async () => {
    let handler: ((event: { payload: unknown }) => void) | undefined;
    const unlisten = vi.fn(), onCapture = vi.fn();
    native.listen.mockImplementation(async (_name, callback: typeof handler) => { handler = callback; return unlisten; });
    const stop = await listenBrowserCaptures(onCapture);
    for (const payload of [null, {}, { kind: "unknown" }, { kind: "item" }, { kind: "attachment" }, { kind: "tags", extra: "ignored" }]) {
      handler?.({ payload });
    }
    expect(onCapture.mock.calls).toEqual([[{ kind: "item" }], [{ kind: "attachment" }], [{ kind: "tags" }]]);
    expect(native.listen).toHaveBeenCalledWith("research-assistant:browser-capture", expect.any(Function));
    stop(); expect(unlisten).toHaveBeenCalledOnce();
  });
});
