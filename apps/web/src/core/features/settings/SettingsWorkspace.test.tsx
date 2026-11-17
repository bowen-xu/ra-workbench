// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SettingsWorkspace } from "./SettingsWorkspace";
import {selectAppStorage} from "../../runtime/appStorage";

const mounted = vi.hoisted(() => ({ resources: vi.fn(), storage: vi.fn(), capture: vi.fn(), docling: vi.fn() }));
vi.mock("./AgentResourceSettings", () => ({ AgentResourceSettings: ({active}: {active: boolean}) => {
  mounted.resources(); return <label data-resource-active={active}>测试 MCP 草稿<input aria-label="测试 MCP 草稿" defaultValue="原始配置" /></label>;
} }));
vi.mock("./LocalStorageSettings", () => ({ LocalStorageSettings: () => { mounted.storage(); return <p>本地存储内容</p>; } }));
vi.mock("./KeyboardShortcutSettings", () => ({ KeyboardShortcutSettings: () => <p>快捷键内容</p> }));
vi.mock("./BrowserConnectorSettings", () => ({BrowserConnectorSettings: () => {mounted.capture(); return <p>浏览器采集配置</p>;}}));
vi.mock("./DoclingSettings", () => ({DoclingSettings: () => {mounted.docling(); return <p>共享解析配置</p>;}}));

let root: Root, host: HTMLDivElement;
beforeEach(() => {
  localStorage.clear();selectAppStorage(localStorage);
  vi.stubGlobal("__APP_VERSION__", "test"); mounted.resources.mockClear(); mounted.storage.mockClear(); mounted.capture.mockClear(); mounted.docling.mockClear();
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
it("keeps shared service settings available without mounting an unavailable library capture adapter, then restores it", async () => {
  localStorage.setItem("capture-preference", "retained");
  await act(async () => root.render(<SettingsWorkspace browserCaptureAvailable={false} onOpenApiSettings={vi.fn()}/>));
  await click("本地服务");
  expect(mounted.docling).toHaveBeenCalled(); expect(mounted.capture).not.toHaveBeenCalled();
  expect(host.textContent).toContain("browser_capture_unavailable");
  expect(localStorage.getItem("capture-preference")).toBe("retained");
  await act(async () => root.unmount()); root = createRoot(host);
  await act(async () => root.render(<SettingsWorkspace browserCaptureAvailable onOpenApiSettings={vi.fn()}/>));
  expect(mounted.capture).toHaveBeenCalled(); expect(host.textContent).not.toContain("browser_capture_unavailable");
  expect(localStorage.getItem("capture-preference")).toBe("retained");
});
it("restores the selected category and search after remount",async()=>{
  await act(async()=>root.render(<SettingsWorkspace onOpenApiSettings={vi.fn()}/>));
  await click("Agent 资源");
  await input(host.querySelector<HTMLInputElement>('[aria-label="搜索设置"]')!,"MCP");
  await act(async()=>root.unmount());root=createRoot(host);
  await act(async()=>root.render(<SettingsWorkspace onOpenApiSettings={vi.fn()}/>));
  expect(host.querySelector('[aria-current="page"]')?.textContent).toBe("Agent 资源");
  expect(host.querySelector<HTMLInputElement>('[aria-label="搜索设置"]')?.value).toBe("MCP");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function click(text: string): Promise<void> {
  const button = [...host.querySelectorAll("button")].find(item => item.textContent === text);
  expect(button).toBeDefined(); await act(async () => button!.click());
}
async function input(node: HTMLInputElement, value: string): Promise<void> {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(node, value);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

it("starts with general settings, opens the existing API configuration, and defers resource/data panels", async () => {
  const open = vi.fn(); await act(async () => root.render(<SettingsWorkspace onOpenApiSettings={open} />));
  expect(host.querySelector('[aria-current="page"]')?.textContent).toBe("常规");
  expect(host.textContent).toContain("应用版本"); expect(host.textContent).not.toContain("检查并安装更新");
  expect(mounted.resources).not.toHaveBeenCalled(); expect(mounted.storage).not.toHaveBeenCalled();
  await click("管理"); expect(open).toHaveBeenCalledOnce();
});

it("opens local plugin management as a settings category with an honest disconnected state", async () => {
  await act(async () => root.render(<SettingsWorkspace onOpenApiSettings={vi.fn()}/>));
  await click("插件");
  expect(host.querySelector('[aria-current="page"]')?.textContent).toBe("插件");
  expect(host.textContent).toContain("本地插件安装服务待接入");
  expect([...host.querySelectorAll("button")].find(button => button.textContent === "导入本地插件")?.disabled).toBe(true);
});

it("filters categories and preserves an unsaved resource form across category changes", async () => {
  await act(async () => root.render(<SettingsWorkspace onOpenApiSettings={vi.fn()} />));
  const search = host.querySelector<HTMLInputElement>('[aria-label="搜索设置"]')!;
  await input(search, "MCP");
  expect([...host.querySelectorAll("nav button")].map(item => item.textContent)).toEqual(["Agent 资源"]);
  await click("Agent 资源");
  const draft = host.querySelector<HTMLInputElement>('[aria-label="测试 MCP 草稿"]')!;
  await input(draft, "未保存配置"); await input(search, ""); await click("常规"); await click("Agent 资源");
  expect(host.querySelector('[aria-label="测试 MCP 草稿"]')).toBe(draft); expect(draft.value).toBe("未保存配置");
  await input(search, "不存在的分类"); expect(host.textContent).toContain("没有匹配的设置");
});

it("retains category, search and draft when the entire settings workspace is hidden", async () => {
  const open = vi.fn();
  await act(async () => root.render(<SettingsWorkspace active onOpenApiSettings={open}/>));
  await click("Agent 资源");
  const draft = host.querySelector<HTMLInputElement>('[aria-label="测试 MCP 草稿"]')!;
  await input(draft, "未保存草稿");
  await act(async () => root.render(<SettingsWorkspace active={false} onOpenApiSettings={open}/>));
  expect(host.querySelector('[data-resource-active="false"]')).not.toBeNull();
  await act(async () => root.render(<SettingsWorkspace active onOpenApiSettings={open}/>));
  expect(host.querySelector('[aria-current="page"]')?.textContent).toBe("Agent 资源");
  expect(host.querySelector('[aria-label="测试 MCP 草稿"]')).toBe(draft); expect(draft.value).toBe("未保存草稿");
  expect(host.querySelector('[data-resource-active="true"]')).not.toBeNull();
});
