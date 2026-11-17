import {createRoot} from "react-dom/client";
import {SettingsWorkspace} from "../../../core/features/settings/SettingsWorkspace";
import {selectAppRuntime} from "../../../core/runtime/appRuntime";
import type {ManagedPluginView, PluginManagementViewGateway} from "../../../core/features/settings/PluginManagementSettings";

function createPluginSettingsFixture(): PluginManagementViewGateway {
  let plugins: readonly ManagedPluginView[] = [
    {
      id: "literature_library", name: "文献管理", version: "0.1.0",
      description: "整理文献、阅读 PDF，管理来源与研究证据。",
      enabled: true, runtime: "active", dependencies: [],
      capabilities: ["文献目录", "PDF 阅读", "证据管理"],
    },
    {
      id: "note_manager", name: "笔记管理", version: "0.1.0",
      description: "在工作台中组织、搜索和管理笔记。",
      enabled: true, runtime: "active", dependencies: [], capabilities: ["笔记目录", "导入与导出"],
    },
    {
      id: "writing_projects", name: "写作管理", version: "0.1.0",
      description: "组织稿件、引用与编译流程。",
      enabled: false, runtime: "disabled", dependencies: ["literature_library（可选）"],
      capabilities: ["稿件工程", "书目与引用", "编译结果"],
    },
  ];
  return {
    async list() {return plugins;},
    async importPackage() {
      if (plugins.some(plugin => plugin.id === "diagram_demo")) {
        throw new Error("plugin_already_installed：演示插件已导入。");
      }
      const plugin: ManagedPluginView = {
        id: "diagram_demo", name: "绘图示例", version: "0.1.0",
        description: "用于演示本地包导入界面的合成插件。",
        enabled: true, runtime: "restart", dependencies: [], capabilities: ["图形文档"],
      };
      plugins = [...plugins, plugin];
      return plugin;
    },
    async setEnabled(id, enabled) {
      plugins = plugins.map(plugin => plugin.id === id ? {...plugin, enabled, runtime: "restart"} : plugin);
    },
    async uninstall(id) {
      plugins = plugins.filter(plugin => plugin.id !== id);
    },
  };
}

export function mountPluginSettingsPreview(target: HTMLElement): void {
  const values = new Map<string, string>();
  const storage: Storage = {
    get length() {return values.size;},
    clear() {values.clear();},
    key(index) {return [...values.keys()][index] ?? null;},
    getItem(key) {return values.get(key) ?? null;},
    setItem(key, value) {values.set(key, value);},
    removeItem(key) {values.delete(key);},
  };
  storage.setItem("research-assistant:workspace.v1:settings-page", JSON.stringify("plugins"));
  selectAppRuntime({mode: "preview", storage, gateways: {}, dispose() {values.clear();}});
  createRoot(target).render(
    <div style={{height: "100dvh"}}>
      <SettingsWorkspace pluginManagement={createPluginSettingsFixture()}
        pluginManagementPreview onOpenApiSettings={trigger => trigger.focus()}/>
    </div>,
  );
}
