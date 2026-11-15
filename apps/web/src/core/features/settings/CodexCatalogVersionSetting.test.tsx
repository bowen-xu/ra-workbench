/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {CodexCatalogVersionSetting} from "./CodexCatalogVersionSetting";
import {decodeCodexCatalogVersions, type CodexCatalogVersions} from "../../services/codexCatalogVersionGateway";
const notices = vi.hoisted(() => ({error: vi.fn(), success: vi.fn()}));
vi.mock("../../services/notifications", () => ({notify: notices}));
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup(); vi.clearAllMocks();});
const initial: CodexCatalogVersions = {runtimeVersion: "0.146.0", selectedVersion: "0.146.0", availableVersions: ["0.146.0"]};
async function fixture(failed = false, enabled = true) {
  const changed = vi.fn();
  const gateway = {getVersions: vi.fn(async (refresh?: boolean) => {
    if (failed && refresh) throw {code: "codex_version_http", message: "HTTP 403"};
    return refresh ? {...initial, availableVersions: ["0.200.0", "0.146.0"]} : initial;
  }), selectVersion: vi.fn(async (version: string) => ({...initial, availableVersions: ["0.200.0", "0.146.0"], selectedVersion: version}))};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  await act(async () => root.render(<CodexCatalogVersionSetting enabled={enabled} gateway={gateway} onChanged={changed}/>));
  return {host, gateway, changed};
}
it("discovers a newer option without selecting it or changing the runtime", async () => {
  const f = await fixture();
  const select = f.host.querySelector<HTMLButtonElement>('[aria-label="Codex CLI 版本号"]')!;
  expect(select.textContent).toContain("0.146.0");
  await act(async () => f.host.querySelector<HTMLButtonElement>('[aria-label="刷新 Codex CLI 版本号"]')!.click());
  expect(f.gateway.getVersions).toHaveBeenLastCalledWith(true);
  expect(select.textContent).toContain("0.146.0"); expect(f.gateway.selectVersion).not.toHaveBeenCalled();
  expect(f.changed.mock.lastCall?.[0]).toEqual({...initial, availableVersions: ["0.200.0", "0.146.0"]});
  expect(f.host.textContent).toContain("仅供获取模型列表使用，不表示更新版本或使用的实际版本");
});
it("retains the current selection and actionable error code when refresh fails", async () => {
  const f = await fixture(true);
  await act(async () => f.host.querySelector<HTMLButtonElement>('[aria-label="刷新 Codex CLI 版本号"]')!.click());
  expect(f.host.querySelector('[aria-label="Codex CLI 版本号"]')?.textContent).toContain("0.146.0");
  expect(notices.error).toHaveBeenCalledWith("HTTP 403（错误码：codex_version_http）");
});
it("does not contact native or external services in an unavailable browser preview", async () => {
  const f = await fixture(false, false); expect(f.gateway.getVersions).not.toHaveBeenCalled();
  expect(f.host.querySelector<HTMLButtonElement>('[aria-label="刷新 Codex CLI 版本号"]')?.disabled).toBe(true);
});
it("rejects malformed version IPC rather than accepting an arbitrary selector value", () => {
  expect(decodeCodexCatalogVersions(initial)).toEqual(initial);
  for (const bad of [{...initial, selectedVersion: "0.200.0"}, {...initial, availableVersions: ["rust-v0.200.0"]}, {...initial, runtimeVersion: "unknown"}, {...initial, availableVersions: []}]) expect(decodeCodexCatalogVersions(bad)).toBeUndefined();
});
