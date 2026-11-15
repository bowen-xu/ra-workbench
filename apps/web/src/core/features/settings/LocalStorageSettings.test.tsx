// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { LocalStorageSettings } from "./LocalStorageSettings";

const gateway = vi.hoisted(() => ({ getReport: vi.fn(), scan: vi.fn(), clean: vi.fn(), cancel: vi.fn() }));
const models = vi.hoisted(() => ({ getModelStorage: vi.fn(), deleteModels: vi.fn() }));
const platform = vi.hoisted(() => ({ desktop: true }));
vi.mock("../../services/doclingGateway", () => ({ getDoclingGateway: () => models }));
vi.mock("../../services/contentStorageGateway", () => ({ getContentStorageGateway: () => gateway }));
vi.mock("../../runtime/appRuntime", () => ({ hasPlatformCapabilities: () => true }));
vi.mock("../../runtime/platformCore", () => ({ isDesktopRuntime: () => platform.desktop }));
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.clearAllMocks();
  platform.desktop = true;
  models.getModelStorage.mockResolvedValue({ totalBytes: 0, downloading: false });
  gateway.getReport.mockResolvedValue({ totalBytes: 300, totalObjectCount: 10, currentReferencedBytes: 120, historyOnlyBytes: 90, pinnedHistoryBytes: 20, pinnedHistoryObjectCount: 1, graceBytes: 60, reclaimableBytes: 30, reclaimableObjectCount: 2, availableBytes: 1024, backfillStatus: "complete", isLowDiskSpace: false });
  gateway.clean.mockResolvedValue({ deletedBytes: 30, deletedObjectCount: 2, prunedRevisionCount: 0 });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});

it("includes model usage in totals and clears it only after confirmed deletion", async () => {
  models.getModelStorage.mockResolvedValue({ totalBytes: 1024, installedVersion: "FP32-v1", downloading: false });
  const confirm = vi.fn(() => false); window.confirm = confirm;
  models.deleteModels.mockImplementation(async () => { models.getModelStorage.mockResolvedValue({ totalBytes: 0, downloading: false }); return { totalBytes: 0, downloading: false }; });
  await act(async () => root.render(<LocalStorageSettings />));
  expect(host.textContent).toContain("1.3 KiB 已使用");
  expect(host.textContent).toContain("Docling 模型");
  expect(host.textContent).toContain("FP32-v1");
  await act(async () => button("删除模型").click()); expect(models.deleteModels).not.toHaveBeenCalled();
  confirm.mockReturnValue(true);
  await act(async () => button("删除模型").click());
  expect(models.deleteModels).toHaveBeenCalledOnce(); expect(host.textContent).toContain("300 B 已使用");
  expect(button("删除模型").disabled).toBe(true);
});

it("disables deletion during downloads", async () => {
  models.getModelStorage.mockResolvedValue({ totalBytes: 1024, downloading: true });
  await act(async () => root.render(<LocalStorageSettings />));
  expect(button("删除模型").disabled).toBe(true);
});
it("preserves model usage after a busy-service rejection", async () => {
  models.getModelStorage.mockResolvedValue({ totalBytes: 1024, downloading: false });
  window.confirm = vi.fn(() => true);
  models.deleteModels.mockRejectedValue({ code: "models_unavailable", message: "其他客户端正在使用模型" });
  await act(async () => root.render(<LocalStorageSettings />));
  await act(async () => button("删除模型").click());
  expect(host.textContent).toContain("1.3 KiB 已使用");
  expect(button("删除模型").disabled).toBe(false);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function button(label: string): HTMLButtonElement { return [...host.querySelectorAll("button")].find(item => item.textContent === label)!; }
it("shows actual usage with grouped actions and keeps retention details in a hint", async () => {
  await act(async () => root.render(<LocalStorageSettings />));
  expect(host.textContent).toContain("300 B 已使用"); expect(host.textContent).toContain("当前可回收 30 B");
  expect(host.querySelector('[role="img"]')).not.toBeNull();
  expect(host.querySelector(".settings-storage-overview")?.textContent).toContain("磁盘可用空间 1 KiB");
  for (const removed of ["查看存储明细", "对象总占用", "固定历史占用", "历史引用索引"]) expect(host.textContent).not.toContain(removed);
  expect(host.textContent).not.toContain("保留 14 天"); expect(host.querySelector('[aria-label="历史版本保留规则"]')).not.toBeNull();
  await act(async () => button("清理").click()); expect(gateway.clean).toHaveBeenCalledWith(false);
});
it("labels browser availability as site quota rather than free disk space", async () => {
  platform.desktop = false;
  await act(async () => root.render(<LocalStorageSettings />));
  expect(host.querySelector(".settings-storage-overview")?.textContent).toContain("站点可用配额 1 KiB");
  expect(host.textContent).not.toContain("磁盘可用空间");
});
it("retains the extra confirmation before cleaning objects in the grace period", async () => {
  const confirm = vi.fn(() => false); vi.stubGlobal("confirm", confirm); window.confirm = confirm;
  await act(async () => root.render(<LocalStorageSettings />));
  await act(async () => host.querySelector<HTMLInputElement>('[role="switch"]')!.click());
  await act(async () => button("清理").click()); expect(confirm).toHaveBeenCalledOnce(); expect(gateway.clean).not.toHaveBeenCalled();
  confirm.mockReturnValue(true); await act(async () => button("清理").click()); expect(gateway.clean).toHaveBeenCalledWith(true);
});
