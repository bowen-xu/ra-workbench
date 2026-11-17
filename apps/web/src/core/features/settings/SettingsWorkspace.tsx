import { lazy, Suspense, useState } from "react";
import { Blocks, Box, Database, Keyboard, Search, Settings2, SlidersHorizontal, Unplug } from "lucide-react";
import { isDesktopRuntime } from "../../runtime/platformCore";
import { AgentResourceSettings } from "./AgentResourceSettings";
import { DeveloperModeSettings } from "./DeveloperMode";
import { KeyboardShortcutSettings } from "./KeyboardShortcutSettings";
import { PreviewDataSettings } from "./PreviewDataSettings";
import { SettingGroup, SettingRow } from "./SettingsLayout";
import "./SettingsWorkspace.css";
import { getAppRuntime } from "../../runtime/appRuntime";
import { storageBackend } from "../../runtime/storagePersistence";
import {useWorkspaceState, workspaceString} from "../../runtime/workspaceState";

import {PluginManagementSettings, type PluginManagementViewGateway} from "./PluginManagementSettings";

const DoclingSettings = lazy(async () => ({ default: (await import("./DoclingSettings")).DoclingSettings }));
const BrowserConnectorSettings = lazy(async () => ({ default: (await import("./BrowserConnectorSettings")).BrowserConnectorSettings }));
const LocalStorageSettings = lazy(async () => ({ default: (await import("./LocalStorageSettings")).LocalStorageSettings }));
const pages = [
  { id: "general", label: "常规", description: "AI API 模型 运行环境 版本", icon: Settings2 },
  { id: "shortcuts", label: "快捷键", description: "聊天 对话列表 键盘", icon: Keyboard },
  { id: "plugins", label: "插件", description: "本地插件 安装 导入 启用 禁用 卸载 依赖", icon: Box },
  { id: "resources", label: "Agent 资源", description: "技能 MCP 服务器 脚本 授权 凭据", icon: Blocks },
  { id: "services", label: "本地服务", description: "Docling PDF 解析 Chrome 文献采集 插件", icon: Unplug },
  { id: "storage", label: "存储", description: "本地数据 空间 清理 历史", icon: Database },
  { id: "advanced", label: "高级", description: "开发者模式 预览数据 模拟", icon: SlidersHorizontal },
] as const;
type SettingsPage = typeof pages[number]["id"];

export function SettingsWorkspace({ onOpenApiSettings, active: visible = true, browserCaptureAvailable = false, pluginManagement, pluginManagementPreview = false }: {
  readonly onOpenApiSettings: (trigger: HTMLButtonElement) => void;
  readonly active?: boolean;
  readonly pluginManagement?: PluginManagementViewGateway;
  readonly pluginManagementPreview?: boolean;
  /** App supplies availability; the shared settings tree does not load a plugin. */
  readonly browserCaptureAvailable?: boolean;
}): React.JSX.Element {
  const [active, setActive] = useWorkspaceState<SettingsPage>("settings-page","general",value=>pages.some(page=>page.id===value)?value as SettingsPage:"general");
  const [visited, setVisited] = useState<readonly SettingsPage[]>([active]);
  const [query, setQuery] = useWorkspaceState("settings-query","",workspaceString);
  const matches = pages.filter(page => `${page.label} ${page.description}`.toLowerCase().includes(query.trim().toLowerCase()));
  const choose = (page: SettingsPage): void => {
    setActive(page);
    setVisited(current => current.includes(page) ? current : [...current, page]);
  };
  return <section className="settings-workspace" aria-label="设置">
    <aside className="settings-navigation">
      <h1>设置</h1>
      <label className="settings-search"><Search size={16} aria-hidden="true" />
        <input type="search" aria-label="搜索设置" placeholder="搜索设置" value={query} onChange={event => setQuery(event.target.value)} /></label>
      <nav aria-label="设置分类">{matches.map(page => <button key={page.id} type="button" aria-current={active === page.id ? "page" : undefined}
        onClick={() => choose(page.id)}><page.icon size={18} strokeWidth={1.7} aria-hidden="true" />{page.label}</button>)}</nav>
      {matches.length === 0 && <p className="settings-search-empty">没有匹配的设置</p>}
    </aside>
    <div className="settings-content">
      {visited.map(page => <div className="settings-page" key={page} hidden={active !== page}>
        {page === "general" && <>
          <header className="settings-page-heading"><h2>常规</h2><p>管理应用的基础配置与运行信息。</p></header>
          <section className="settings-section"><h3>AI 与模型</h3>
            <SettingGroup label="AI 与模型"><SettingRow title="AI API 设置" description="管理 Provider、账号、模型目录与功能路由。">
              <button type="button" onClick={event => onOpenApiSettings(event.currentTarget)}>管理</button>
            </SettingRow></SettingGroup>
          </section>
          <section className="settings-section"><h3>关于应用</h3>
            <SettingGroup label="关于应用">
              <SettingRow title="应用版本"><span>{__APP_VERSION__}</span></SettingRow>
              <SettingRow title="运行环境"><span>{isDesktopRuntime() ? "Tauri 2 · 本地桌面" : "浏览器预览"}</span></SettingRow>
              <SettingRow title="数据存储" description={isDesktopRuntime() ? "数据保存在本机应用数据目录。" : "浏览器数据与桌面应用隔离保存。"}>
                <span>{isDesktopRuntime() ? "SQLite" : storageBackend(getAppRuntime().storage)}</span>
              </SettingRow>
            </SettingGroup>
          </section>
        </>}
        {page === "plugins" && <PluginManagementSettings {...(pluginManagement ? {gateway: pluginManagement} : {})} active={visible && active === "plugins"} preview={pluginManagementPreview}/>}
        {page === "shortcuts" && <KeyboardShortcutSettings />}
        {page === "resources" && <AgentResourceSettings active={visible && active === "resources"} />}
        {page === "services" && <><header className="settings-page-heading"><h2>本地服务</h2><p>管理文献解析与浏览器采集连接。</p></header>
          <Suspense fallback={<p role="status">正在读取服务设置…</p>}><DoclingSettings active={visible && active === "services"} onManageModels={() => { setQuery(""); choose("storage"); }} />{browserCaptureAvailable
            ? <BrowserConnectorSettings active={visible && active === "services"} />
            : <section className="settings-section"><h3>浏览器文献采集</h3><p role="status">browser_capture_unavailable：当前未装配文献管理插件，浏览器采集连接暂不可用；原设置与已采集资源保留。</p></section>}</Suspense></>}
        {page === "storage" && <Suspense fallback={<p role="status">正在读取本地存储…</p>}><LocalStorageSettings active={visible && active === "storage"} /></Suspense>}
        {page === "advanced" && <><header className="settings-page-heading"><h2>高级</h2><p>开发与调试选项。</p></header><DeveloperModeSettings /><PreviewDataSettings /></>}
      </div>)}
    </div>
  </section>;
}
