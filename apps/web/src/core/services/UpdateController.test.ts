import { describe, expect, it, vi } from "vitest";
import { UpdateController } from "./UpdateController";
import { INITIAL_UPDATE_STATUS, type UpdateGateway, type UpdateStatus } from "./updateProtocol";

function fixture() {
  let current = INITIAL_UPDATE_STATUS;
  const listeners = new Set<(status: UpdateStatus) => void>();
  const publish = (change: Partial<UpdateStatus>) => {
    current = { ...current, ...change, revision: current.revision + 1 };
    listeners.forEach(listener => listener(current));
    return current;
  };
  let finishDownload!: (status: UpdateStatus) => void;
  const gateway: UpdateGateway = {
    status: vi.fn(async () => current),
    subscribe: async listener => { listeners.add(listener); return () => listeners.delete(listener); },
    check: vi.fn(async () => publish({phase: "available", version: "1.2.3"})),
    download: vi.fn(() => {
      publish({phase: "downloading", total: 100, downloaded: 0});
      return new Promise<UpdateStatus>(resolve => { finishDownload = resolve; });
    }),
    pause: vi.fn(async paused => publish({phase: paused ? "paused" : "downloading"})),
    install: vi.fn(async () => publish({phase: "installing"})),
  };
  const prepare = vi.fn(async () => true), report = vi.fn();
  const controller = new UpdateController(gateway, prepare, report);
  return {controller, gateway, prepare, report, publish, finish: () => finishDownload(current)};
}
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };

describe("sidebar updater session", () => {
  it("checks only on demand, prevents double download, pauses and continues the same task, then installs automatically once", async () => {
    const f = fixture(); await f.controller.connect();
    expect(f.gateway.check).not.toHaveBeenCalled();
    await f.controller.activate();
    expect(f.gateway.download).not.toHaveBeenCalled();
    await f.controller.activate();
    await f.controller.activate();
    expect(f.gateway.download).toHaveBeenCalledOnce();
    expect(f.gateway.pause).toHaveBeenLastCalledWith(true);
    expect(f.controller.snapshot().phase).toBe("paused");
    f.publish({downloaded: 42});
    await f.controller.activate();
    expect(f.gateway.pause).toHaveBeenLastCalledWith(false);
    expect(f.controller.snapshot().downloaded).toBe(42);
    const completed = f.publish({phase: "downloaded", downloaded: 100, readyToInstall: true});
    // The command response duplicates an event, and must not install twice.
    f.controller.accept(completed); f.finish(); await flush();
    expect(f.prepare).toHaveBeenCalledOnce();
    expect(f.gateway.install).toHaveBeenCalledOnce();
  });
  it("shows the latest-version message once for every explicit check", async () => {
    const f = fixture(); await f.controller.connect();
    vi.mocked(f.gateway.check).mockImplementation(async () => f.publish({phase: "upToDate", message: "当前已是最新版本。"}));
    await f.controller.activate(); await f.controller.activate();
    expect(f.report.mock.calls).toEqual([["当前已是最新版本。", false], ["当前已是最新版本。", false]]);
    expect(f.gateway.download).not.toHaveBeenCalled();
  });
  it("keeps verified bytes available when editor saving fails and retries installation without downloading", async () => {
    const f = fixture(); await f.controller.connect();
    f.prepare.mockResolvedValueOnce(false);
    f.publish({phase: "downloaded", version: "1.2.3", readyToInstall: true}); await flush();
    expect(f.gateway.install).not.toHaveBeenCalled();
    expect(f.controller.snapshot().phase).toBe("failed");
    expect(f.report).toHaveBeenCalledWith(expect.stringContaining("尚未安全保存"), true);
    await f.controller.activate();
    expect(f.gateway.install).toHaveBeenCalledOnce();
    expect(f.gateway.download).not.toHaveBeenCalled();
  });
  it("never installs a failed or unverified download; reports the diagnostic code once", async () => {
    const f = fixture(); await f.controller.connect();
    await f.controller.activate(); await f.controller.activate();
    const failed = f.publish({phase: "failed", message: "签名无效（update_download_failed）"});
    f.controller.accept(failed); f.finish(); await flush();
    expect(f.report).toHaveBeenCalledOnce();
    expect(f.report).toHaveBeenCalledWith(failed.message, true);
    expect(f.gateway.install).not.toHaveBeenCalled();
    await f.controller.activate();
    expect(f.gateway.download).toHaveBeenCalledTimes(2);
  });
  it("ignores stale snapshots and duplicate listeners while reconnecting", async () => {
    const f = fixture();
    let release!: (value: UpdateStatus) => void;
    vi.mocked(f.gateway.status).mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
    const connecting = f.controller.connect(); await flush();
    const latest = f.publish({phase: "paused", downloaded: 50});
    release(INITIAL_UPDATE_STATUS); const stop = await connecting;
    expect(f.controller.snapshot()).toEqual(latest);
    stop(); await f.controller.connect();
    f.publish({phase: "downloaded", readyToInstall: true}); await flush();
    expect(f.gateway.install).toHaveBeenCalledOnce();
  });
  it("recovers from a rejected check with an actionable error and a subsequent successful check", async () => {
    const f = fixture(); await f.controller.connect();
    vi.mocked(f.gateway.check).mockRejectedValueOnce(new Error("IPC unavailable"));
    await f.controller.activate();
    expect(f.controller.snapshot().phase).toBe("failed");
    expect(f.report).toHaveBeenCalledWith(expect.stringContaining("update_client_failed"), true);
    await f.controller.activate();
    expect(f.controller.snapshot().phase).toBe("available");
  });
});
