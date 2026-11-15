// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AppUpdateControl } from "./AppUpdateControl";
import { INITIAL_UPDATE_STATUS, type UpdateGateway, type UpdateStatus } from "../services/updateProtocol";
import { notify } from "../services/notifications";

let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => { host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });
function fixture() {
  let current = INITIAL_UPDATE_STATUS;
  let listener = (_: UpdateStatus) => {};
  const publish = (change: Partial<UpdateStatus>) => { current = {...current, ...change, revision: current.revision + 1}; listener(current); return current; };
  const gateway: UpdateGateway = {
    status: async () => current, subscribe: async callback => { listener = callback; return () => {}; },
    check: vi.fn(async () => publish({phase: "available", version: "1.2.3"})),
    download: vi.fn(() => { publish({phase: "downloading", total: 100, downloaded: 24}); return new Promise<UpdateStatus>(() => {}); }),
    pause: vi.fn(async paused => publish({phase: paused ? "paused" : "downloading"})),
    install: vi.fn(async () => publish({phase: "installing"})),
  };
  const button = () => host.querySelector<HTMLButtonElement>("button")!;
  return {gateway, publish, button, click: async () => { await act(async () => button().click()); }};
}
it("uses the same button for download, ring progress, pause and continue; includes the target version in its hint", async () => {
  const f = fixture(); await act(async () => root.render(<AppUpdateControl gateway={f.gateway} />));
  expect(f.gateway.check).not.toHaveBeenCalled(); await f.click();
  expect(f.button().getAttribute("aria-label")).toBe("下载更新");
  await act(async () => f.button().focus());
  expect(document.body.textContent).toContain("下载更新 1.2.3");
  await f.click();
  expect(f.button().disabled).toBe(false);
  expect(f.button().getAttribute("aria-label")).toBe("暂停下载");
  expect(host.querySelector('[role="progressbar"]')?.getAttribute("aria-valuenow")).toBe("24");
  expect(host.querySelector("svg.update-progress")).not.toBeNull();
  await f.click(); expect(f.button().getAttribute("aria-label")).toBe("继续下载");
  await f.click(); expect(f.button().getAttribute("aria-label")).toBe("暂停下载");
  expect(f.gateway.download).toHaveBeenCalledOnce();
});
it("notifies when already latest and does not start downloading", async () => {
  const f = fixture(), info = vi.spyOn(notify, "info").mockReturnValue("toast");
  vi.mocked(f.gateway.check).mockImplementation(async () => f.publish({phase: "upToDate", message: "当前已是最新版本。"}));
  await act(async () => root.render(<AppUpdateControl gateway={f.gateway} />)); await f.click();
  expect(info).toHaveBeenCalledExactlyOnceWith("当前已是最新版本。");
  expect(f.gateway.download).not.toHaveBeenCalled();
  expect(f.button().getAttribute("aria-label")).toBe("检查更新");
});
