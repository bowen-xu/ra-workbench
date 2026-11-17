// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {PluginManagementSettings, type ManagedPluginView, type PluginManagementViewGateway} from "./PluginManagementSettings";

let root: Root, host: HTMLDivElement;
const plugin: ManagedPluginView = {
  id: "notes", name: "笔记管理", version: "0.1.0", description: "管理工作台笔记",
  enabled: true, runtime: "active", dependencies: [], capabilities: ["笔记目录"],
};
beforeEach(() => {
  Object.assign(globalThis, {IS_REACT_ACT_ENVIRONMENT: true});
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

function fixture(): PluginManagementViewGateway {
  let plugins = [plugin];
  return {
    list: vi.fn(async () => plugins),
    importPackage: vi.fn(async () => {
      const added = {...plugin, id: "writing", name: "写作管理"};
      plugins = [...plugins, added];
      return added;
    }),
    setEnabled: vi.fn(async (id, enabled) => {
      plugins = plugins.map(item => item.id === id ? {...item, enabled, runtime: "restart"} : item);
    }),
    uninstall: vi.fn(async id => {
      plugins = plugins.filter(item => item.id !== id);
    }),
  };
}
async function click(text: string): Promise<void> {
  const button = [...document.querySelectorAll("button")].find(node => node.textContent === text);
  if (!button) throw new Error(`Missing button ${text}`);
  await act(async () => button.click());
}

it("shows the real empty integration state with unavailable mutations", async () => {
  await act(async () => root.render(<PluginManagementSettings/>));
  expect(host.textContent).toContain("本地插件管理需要桌面应用");
  expect(host.querySelector("button")?.disabled).toBe(true);
  expect(host.textContent).not.toContain("笔记管理");
});

it("imports through the supplied picker port and selects the returned package", async () => {
  const gateway = fixture();
  await act(async () => root.render(<PluginManagementSettings gateway={gateway}/>));
  await click("导入本地插件");
  expect(gateway.importPackage).toHaveBeenCalledOnce();
  expect(host.querySelector('[aria-label="插件详情"]')?.textContent).toContain("写作管理");
});

it("keeps the list when package selection is cancelled", async () => {
  const gateway = fixture();
  vi.mocked(gateway.importPackage).mockResolvedValueOnce(null);
  await act(async () => root.render(<PluginManagementSettings gateway={gateway}/>));
  await click("导入本地插件");
  expect(host.querySelector('[aria-label="插件详情"]')?.textContent).toContain("笔记管理");
  expect(host.textContent).not.toContain("已安装");
  expect(host.querySelector('[role="alert"]')).toBeNull();
});

it("preserves the previous state on failed enable and shows the original error", async () => {
  const gateway = fixture();
  vi.mocked(gateway.setEnabled).mockRejectedValueOnce(new Error("plugin_busy: operation-42"));
  await act(async () => root.render(<PluginManagementSettings gateway={gateway}/>));
  await click("禁用");
  expect(host.textContent).toContain("plugin_busy: operation-42");
  expect(host.textContent).toContain("运行中");
  await click("禁用");
  expect(host.textContent).toContain("待重启生效");
});

it("cancels uninstall without a write, then confirms through the same port", async () => {
  const gateway = fixture();
  await act(async () => root.render(<PluginManagementSettings gateway={gateway}/>));
  await click("卸载");
  await click("取消");
  expect(gateway.uninstall).not.toHaveBeenCalled();
  await click("卸载");
  await click("确认卸载");
  expect(gateway.uninstall).toHaveBeenCalledWith("notes");
  expect(host.textContent).toContain("尚未安装插件");
});
