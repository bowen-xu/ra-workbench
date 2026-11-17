import {useEffect, useRef, useState} from "react";
import {Box, Download, Search, RefreshCw} from "lucide-react";
import {DialogHeading} from "@ariakit/react";
import {Dialog} from "../../components/overlay/OverlayDialog";
import "./PluginManagementSettings.css";

export interface ManagedPluginView {
  readonly id: string;
  readonly name: string;
  readonly version: string;
  readonly description: string;
  readonly enabled: boolean;
  readonly runtime: "active" | "disabled" | "blocked" | "restart" | "loading";
  readonly pendingRemoval?: boolean;
  readonly origin?: "installed" | "bundled";
  readonly dependencies: readonly string[];
  readonly capabilities: readonly string[];
  readonly diagnostic?: string;
}

/** UI port. Installation and runtime authority remain with the eventual Core backend. */
export interface PluginManagementViewGateway {
  list(): Promise<readonly ManagedPluginView[]>;
  importPackage(): Promise<ManagedPluginView | null>;
  setEnabled(id: string, enabled: boolean): Promise<void>;
  uninstall(id: string): Promise<void>;
  subscribe?(listener: () => void): () => void;
}

const statuses = {active: "运行中", disabled: "已禁用", blocked: "加载受阻", restart: "待重启生效", loading: "正在加载"};

export function PluginManagementSettings({gateway, active = true, preview = false}: {
  readonly gateway?: PluginManagementViewGateway;
  readonly active?: boolean;
  readonly preview?: boolean;
}): React.JSX.Element {
  const [plugins, setPlugins] = useState<readonly ManagedPluginView[]>([]);
  const [selectedId, select] = useState<string>();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(Boolean(gateway));
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [removing, setRemoving] = useState<ManagedPluginView>();
  const readGeneration = useRef(0);

  useEffect(() => {
    if (!active || !gateway) return;
    let disposed = false;
    const refresh = (): void => {
      const generation = ++readGeneration.current;
      setLoading(true);
      void gateway.list()
      .then(value => {
        if (disposed || generation !== readGeneration.current) return;
        setPlugins(value);
        setError(undefined);
      })
      .catch(reason => {
        if (!disposed && generation === readGeneration.current) setError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => {
        if (!disposed && generation === readGeneration.current) setLoading(false);
      });
    };
    refresh();
    const unsubscribe = gateway.subscribe?.(refresh);
    return () => {disposed = true; readGeneration.current += 1; unsubscribe?.();};
  }, [gateway, active]);

  const visible = plugins.filter(plugin =>
    `${plugin.name} ${plugin.id}`.toLowerCase().includes(query.trim().toLowerCase())
    && (filter === "all" || (filter === "enabled" ? plugin.enabled : !plugin.enabled)));
  const selected = visible.find(plugin => plugin.id === selectedId) ?? visible[0];

  async function run(operation: (service: PluginManagementViewGateway) => Promise<void>): Promise<void> {
    if (!gateway || busy) return;
    setBusy(true);
    setError(undefined);
    setNotice(undefined);
    try {
      await operation(gateway);
      const generation = ++readGeneration.current;
      const plugins = await gateway.list();
      if (generation === readGeneration.current) setPlugins(plugins);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
      setLoading(false);
    }
  }

  function importPackage(): void {
    void run(async service => {
      const plugin = await service.importPackage();
      if (!plugin) return;
      select(plugin.id);
      setQuery("");
      setFilter("all");
      setNotice(`${plugin.name} 已安装。${plugin.enabled ? "重启后生效。" : "启用后将在下次启动时加载。"}`);
    });
  }

  return <>
    <header className="settings-page-heading plugin-settings-heading">
      <div><h2>插件</h2><p>管理本地插件、启用状态与依赖。</p></div>
      <button type="button" className="primary-button" disabled={!gateway || busy || loading} onClick={importPackage}>
        <Download size={16} aria-hidden="true"/>导入本地插件
      </button>
    </header>
    {preview && <p className="plugin-settings-message" role="note">交互预览 · 使用合成插件数据</p>}
    {!gateway && <p className="plugin-settings-message" role="status">本地插件管理需要桌面应用。</p>}
    {error && <p className="plugin-settings-error" role="alert">{error}</p>}
    {notice && <p className="plugin-settings-message" role="status">{notice}</p>}
    <div className="plugin-settings-toolbar">
      <label className="plugin-settings-search">
        <Search size={16} aria-hidden="true"/>
        <input aria-label="搜索已安装插件" placeholder="搜索已安装插件" value={query}
          onChange={event => setQuery(event.target.value)}/>
      </label>
      <label>显示
        <select aria-label="筛选插件状态" value={filter} onChange={event => setFilter(event.target.value)}>
          <option value="all">全部插件</option>
          <option value="enabled">已启用</option>
          <option value="disabled">已禁用</option>
        </select>
      </label>
      <button type="button" aria-label="刷新插件列表" disabled={!gateway || busy || loading}
        onClick={() => void run(async () => {setNotice("插件列表已刷新。");})}>
        <RefreshCw size={15} aria-hidden="true"/>
      </button>
    </div>
    {loading ? <p role="status" className="settings-empty">正在读取插件…</p> :
      <div className="plugin-settings-layout" aria-busy={busy}>
        <div className="plugin-settings-list" role="group" aria-label="已安装插件">
          {visible.map(plugin =>
            <button key={plugin.id} type="button" aria-pressed={selected?.id === plugin.id}
              className="plugin-settings-item" onClick={() => select(plugin.id)}>
              <Box size={22} aria-hidden="true"/>
              <span><strong>{plugin.name}</strong><small>{plugin.version} · {statuses[plugin.runtime]}</small></span>
            </button>)}
          {visible.length === 0 &&
            <p className="settings-empty">{!gateway ? "接入安装服务后显示已安装插件" :
              plugins.length ? "没有匹配的插件" : "尚未安装插件"}</p>}
        </div>
        {selected && <section className="plugin-settings-detail" aria-label="插件详情">
          <header>
            <Box size={30} aria-hidden="true"/>
            <div><h3>{selected.name}</h3><p>{selected.id}</p></div>
          </header>
          <p>{selected.description}</p>
          <div className="plugin-settings-actions">
            <button type="button" disabled={!gateway || busy || loading || selected.pendingRemoval} onClick={() => void run(async service => {
              await service.setEnabled(selected.id, !selected.enabled);
              setNotice(`${selected.name} 的启用设置已保存，重启后生效。`);
            })}>{selected.enabled ? "禁用" : "启用"}</button>
            <button type="button" disabled={!gateway || busy || loading || selected.pendingRemoval} onClick={() => {
              setError(undefined);
              setRemoving(selected);
            }}>卸载</button>
            {busy && <span role="status">正在处理…</span>}
          </div>
          <dl>
            <div><dt>版本</dt><dd>{selected.version}</dd></div>
            {selected.origin && <div><dt>安装来源</dt><dd>{selected.origin === "bundled" ? "随应用或开发配置提供" : "本地安装"}</dd></div>}
            <div><dt>启动设置</dt><dd>{selected.enabled ? "已启用" : "已禁用"}</dd></div>
            <div><dt>运行状态</dt><dd>{statuses[selected.runtime]}</dd></div>
            <div><dt>依赖插件</dt><dd>{selected.dependencies.join("、") || "无"}</dd></div>
            <div><dt>提供能力</dt><dd>{selected.capabilities.join("、") || "无"}</dd></div>
          </dl>
          {selected.runtime === "restart" && <p className="plugin-settings-message">{selected.pendingRemoval ? "已安排卸载，重启工作台后完成。" : "重启工作台后应用此更改。"}</p>}
          {selected.diagnostic && <p role="alert" className="plugin-settings-error">{selected.diagnostic}</p>}
        </section>}
      </div>}
    {removing && <Dialog open onClose={() => {if (!busy) setRemoving(undefined);}}
      backdrop={<div className="plugin-settings-backdrop"/>}
      className="plugin-settings-dialog" aria-describedby="plugin-uninstall-description">
      <DialogHeading>卸载 {removing.name}？</DialogHeading>
      <p id="plugin-uninstall-description">移除插件程序包。笔记、文献与稿件资料按原存储规则管理。更改将在重启后生效。</p>
      {error && <p role="alert">{error}</p>}
      <footer>
        <button type="button" disabled={busy} onClick={() => setRemoving(undefined)}>取消</button>
        <button type="button" disabled={busy} onClick={() => void run(async service => {
          await service.uninstall(removing.id);
          setRemoving(undefined);
          setNotice(`${removing.name} 已安排卸载，重启后生效。`);
        })}>{busy ? "正在卸载…" : "确认卸载"}</button>
      </footer>
    </Dialog>}
  </>;
}
