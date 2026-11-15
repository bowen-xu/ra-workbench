// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BrowserConnectorGateway } from "../../services/browserConnectorGateway";
import { BrowserConnectorSettings } from "./BrowserConnectorSettings";

const platform = vi.hoisted(() => ({ desktop: true, error: vi.fn() }));
vi.mock("../../runtime/platformCore", () => ({ isDesktopRuntime: () => platform.desktop }));
vi.mock("../../services/browserConnectorGateway", () => ({ getBrowserConnectorGateway: vi.fn() }));
vi.mock("../../services/notifications", () => ({ notify: { error: platform.error } }));
const status = { endpoint: "http://127.0.0.1:23120", available: true, message: "已准备接收文献" };

describe("Chrome connector automatic connection settings", () => {
  let host: HTMLDivElement, root: Root;
  const gateway = { getStatus: vi.fn() } satisfies BrowserConnectorGateway;
  const button = () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.getAttribute("aria-label")?.includes("刷新"))!;
  beforeEach(() => {
    vi.clearAllMocks(); platform.desktop = true;
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    gateway.getStatus.mockResolvedValue(status);
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

  it("shows automatic connection and offers no credential or authorization controls", async () => {
    await act(async () => root.render(<BrowserConnectorSettings gateway={gateway} />));
    expect(host.textContent).toContain("无需填写配对码");
    expect(host.textContent).toContain(status.endpoint);
    expect(host.querySelector("input")).toBeNull();
    expect(host.querySelector('[aria-label="Chrome 文献采集说明"]')).not.toBeNull();
    expect(button().querySelector("svg")).not.toBeNull();
    expect(button().closest(".control-hint-anchor")).not.toBeNull();
    await act(async () => button().click());
    expect(gateway.getStatus).toHaveBeenCalledTimes(2);
  });

  it("retains the occupied-port message for unrelated software and still permits refresh", async () => {
    gateway.getStatus.mockResolvedValue({ ...status, available: false, message: "浏览器采集端口可能被其他软件占用" });
    await act(async () => root.render(<BrowserConnectorSettings gateway={gateway} />));
    expect(host.querySelector('[role="status"]')?.textContent).toContain("其他软件占用");
    expect(button().disabled).toBe(false);
  });

  it("reports refresh failure through shared notifications without native diagnostics", async () => {
    await act(async () => root.render(<BrowserConnectorSettings gateway={gateway} />));
    gateway.getStatus.mockRejectedValue(new Error("private-native-diagnostic"));
    await act(async () => button().click());
    expect(platform.error).toHaveBeenCalledWith("无法读取 Chrome 插件连接状态，请刷新重试。");
    expect(host.textContent).not.toContain("private-native-diagnostic");
    expect(host.querySelector('[role="alert"]')).toBeNull();
  });

  it("disables native refresh in browser and preview modes", async () => {
    platform.desktop = false;
    gateway.getStatus.mockResolvedValue({ endpoint: "", available: false, message: "请使用桌面应用" });
    await act(async () => root.render(<BrowserConnectorSettings gateway={gateway} />));
    expect(button().disabled).toBe(true);
    expect(host.textContent).toContain("浏览器预览不能连接");
  });
});
