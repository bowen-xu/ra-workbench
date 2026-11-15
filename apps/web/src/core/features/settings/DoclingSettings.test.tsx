/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  DoclingGateway,
  DoclingServiceStatus,
  DoclingSettings as Settings,
} from "../../services/doclingGateway";
import { notify } from "../../services/notifications";
import { DoclingSettings } from "./DoclingSettings";

const gateway = vi.hoisted(() => ({
  getSettings: vi.fn(),
  getModelStatus: vi.fn(),
  checkModelUpdate: vi.fn(),
  downloadModels: vi.fn(),
  getStatus: vi.fn(),
  pingService: vi.fn(),
  saveSettings: vi.fn(),
  connectService: vi.fn(),
  disconnectService: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("../../services/doclingGateway", () => ({
  getDoclingGateway: () => gateway as unknown as DoclingGateway,
}));
vi.mock("../../services/notifications", () => ({
  notify: { error: vi.fn(), success: vi.fn(), warning: vi.fn() },
}));

const settings: Settings = {
  enabled: true,
  modelManifestUrl: "http://127.0.0.1:4174/models/docling/latest.json",
  defaultPort: 5001,
  fallbackStartPort: 5010,
  fallbackEndPort: 5099,
  idleShutdownMs: 1_200_000,
  requestTimeoutMs: 300_000,
  updatedAt: "2026-08-10T00:00:00Z",
};

function button(label: string): HTMLButtonElement {
  const match = [...document.body.querySelectorAll("button")].find(
    (candidate) =>
      candidate.getAttribute("aria-label") === label ||
      candidate.textContent?.includes(label),
  );
  if (match === undefined) throw new Error(`missing button ${label}`);
  return match;
}

describe("Docling daemon settings", () => {
  let container: HTMLDivElement;
  let root: Root;

  it("pauses hidden status polling and preserves loaded settings on return", async () => {
    vi.useFakeTimers();
    try {
      await act(async () => root.render(<DoclingSettings active={false}/>));
      await act(async () => vi.advanceTimersByTime(15_000));
      expect(gateway.getStatus).not.toHaveBeenCalled(); expect(gateway.getSettings).not.toHaveBeenCalled(); expect(gateway.getModelStatus).not.toHaveBeenCalled();
      await act(async () => root.render(<DoclingSettings active/>));
      expect(gateway.getSettings).toHaveBeenCalledOnce();
      await act(async () => root.render(<DoclingSettings active={false}/>));
      const count = gateway.getStatus.mock.calls.length;
      await act(async () => vi.advanceTimersByTime(15_000)); expect(gateway.getStatus).toHaveBeenCalledTimes(count);
      await act(async () => root.render(<DoclingSettings active/>)); expect(gateway.getSettings).toHaveBeenCalledOnce();
    } finally {vi.useRealTimers();}
  });

  beforeEach(() => {
    vi.clearAllMocks();
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    gateway.getSettings.mockResolvedValue(settings);
    gateway.saveSettings.mockImplementation(async (draft) => ({
      ...draft,
      updatedAt: settings.updatedAt,
    }));
    gateway.getModelStatus.mockResolvedValue({
      state: "not_installed",
      downloadedBytes: 0,
      totalBytes: 0,
    });
    gateway.getStatus.mockResolvedValue({
      state: "disconnected",
    } satisfies DoclingServiceStatus);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("shows daemon connection controls without a Docling command field", async () => {
    await act(async () => root.render(<DoclingSettings />));
    await act(async () => undefined);

    expect(button("连接服务").querySelector("svg")).not.toBeNull();
    expect(document.querySelector('[aria-label="断开本客户端"]')).toBeNull();
    expect(document.body.textContent).not.toContain("保存配置");
    expect(document.body.textContent).not.toContain("Docling 命令");
    expect(document.querySelector('[aria-label="Conda 环境"]')).toBeNull();
    expect(document.querySelector('[aria-label="模型更新源"]')).not.toBeNull();
    expect(document.querySelector('[aria-label="解析无响应超时（毫秒）"]')).not.toBeNull();
    expect(button("连接服务").disabled).toBe(false);
  });

  it("checks and downloads independent models and keeps the installed version visible on failure", async () => {
    gateway.getModelStatus.mockResolvedValue({
      state: "ready",
      installedVersion: "models-1",
      downloadedBytes: 0,
      totalBytes: 0,
    });
    gateway.checkModelUpdate.mockResolvedValue({
      state: "ready",
      installedVersion: "models-1",
      availableVersion: "models-2",
      downloadedBytes: 0,
      totalBytes: 0,
    });
    gateway.downloadModels.mockRejectedValue({
      code: "models_unavailable",
      message: "checksum mismatch",
    });
    await act(async () => root.render(<DoclingSettings />));
    expect(document.body.textContent).toContain("models-1");
    await act(async () => button("检查模型更新").click());
    expect(gateway.checkModelUpdate).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("models-2");
    await act(async () => button("下载模型更新").click());
    expect(gateway.downloadModels).toHaveBeenCalledOnce();
    expect(notify.error).toHaveBeenCalledWith(
      "[models_unavailable] checksum mismatch",
    );
    expect(document.body.textContent).toContain("当前版本：models-1");
  });

  it("automatically saves the latest edit before connecting", async () => {
    gateway.connectService.mockResolvedValue({ state: "connected" });
    await act(async () => root.render(<DoclingSettings />));
    const input = document.querySelector<HTMLInputElement>(
      '[aria-label="默认端口"]',
    )!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!;
      setter.call(input, "5023");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => button("连接服务").click());
    expect(gateway.saveSettings).toHaveBeenLastCalledWith(
      expect.objectContaining({ defaultPort: 5023 }),
    );
    expect(gateway.saveSettings.mock.invocationCallOrder[0]).toBeLessThan(
      gateway.connectService.mock.invocationCallOrder[0]!,
    );
  });

  it("shows no-update feedback and changes one button through download and completion", async () => {
    gateway.checkModelUpdate.mockResolvedValue({
      state: "ready",
      installedIdentity: "one",
      availableIdentity: "one",
    });
    await act(async () => root.render(<DoclingSettings />));
    await act(async () => button("检查模型更新").click());
    expect(notify.success).toHaveBeenCalledWith("模型无更新");
    gateway.checkModelUpdate.mockResolvedValue({
      state: "ready",
      installedIdentity: "one",
      availableIdentity: "two",
    });
    await act(async () => button("检查模型更新").click());
    expect(notify.success).toHaveBeenCalledWith("有模型更新，可点击下载");
    let finish!: (value: unknown) => void;
    gateway.downloadModels.mockReturnValue(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    await act(async () => button("下载模型更新").click());
    expect(button("正在下载模型").disabled).toBe(true);
    await act(async () => finish({ state: "ready", installedVersion: "two" }));
    const completed = button("模型更新完成");
    expect(completed.querySelector("svg")).not.toBeNull();
    expect(completed.closest(".control-hint-anchor")).not.toBeNull();
    await act(async () =>
      completed.dispatchEvent(new MouseEvent("mouseover", { bubbles: true })),
    );
    expect(button("检查模型更新")).not.toBeNull();
  });

  it("disconnects only the current client and renders shared daemon counts", async () => {
    gateway.getStatus.mockResolvedValue({
      state: "connected",
      baseUrl: "http://127.0.0.1:5001",
      activeClientCount: 2,
      activeRequestCount: 1,
    } satisfies DoclingServiceStatus);
    gateway.disconnectService.mockResolvedValue({ state: "disconnected" });
    gateway.pingService.mockResolvedValue({
      status: {
        state: "connected",
        baseUrl: "http://127.0.0.1:5001",
        activeClientCount: 3,
        activeRequestCount: 0,
      },
      latencyMs: 12,
    });
    await act(async () => root.render(<DoclingSettings />));
    await act(async () => undefined);

    expect(document.body.textContent).toContain("http://127.0.0.1:5001");
    expect(document.body.textContent).toContain("2 个客户端");
    expect(document.body.textContent).toContain("1 个任务");
    await act(async () => button("Ping").click());

    expect(gateway.pingService).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("最近 Ping：12 ms");
    expect(document.body.textContent).toContain("3 个客户端");
    await act(async () => button("断开本客户端").click());

    expect(gateway.disconnectService).toHaveBeenCalledOnce();
    expect(document.body.textContent).toContain("本客户端未连接");
  });
});
