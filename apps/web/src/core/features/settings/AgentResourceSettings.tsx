import { notify } from "../../services/notifications";
import { useEffect, useRef, useState } from "react";
import { ArrowLeft, ChevronDown, FolderPlus, GitBranch, KeyRound, Plug, Settings2, Trash2 } from "lucide-react";
import type { AgentSkillCatalog } from "@research-assistant/core/agent";
import { agentSkillCatalogGateway, agentMcpGateway, type AgentMcpServerSummary } from "../../services/agentGateway";
import { agentResourceManagementGateway, type AgentSkillImportPreview, type AgentExecutionHost } from "../../services/agentResourceManagementGateway";
import { ActionMenu, ActionMenuItem } from "../../components/ActionMenu";
import { SettingField, SettingGroup, SettingRow } from "./SettingsLayout";
import { McpServerForm } from "./McpServerForm";
import { appendMcpServer, type McpServerDraft } from "./mcpServerDraft";
import "./AgentResourceSettings.css";

type Page = "list" | "git" | "mcp" | "config" | "credentials";
/** Global administration remains separate from per-message resource selection. */
export function AgentResourceSettings({ gateway = agentResourceManagementGateway, skills = agentSkillCatalogGateway, tools = agentMcpGateway, active = true }: {
  readonly gateway?: typeof agentResourceManagementGateway; readonly skills?: typeof agentSkillCatalogGateway; readonly tools?: typeof agentMcpGateway;
  readonly active?: boolean;
}): React.JSX.Element {
  const [page, setPage] = useState<Page>("list");
  const [catalog, setCatalog] = useState<AgentSkillCatalog | null>(null), [status, setStatus] = useState(""), [busy, setBusy] = useState(false);
  const [url, setUrl] = useState(""), [ref, setRef] = useState(""), [directory, setDirectory] = useState("");
  const [preview, setPreview] = useState<AgentSkillImportPreview | null>(null), [selected, setSelected] = useState<readonly string[]>([]), [updates, setUpdates] = useState<readonly string[]>([]);
  const [mcp, setMcp] = useState("schema_version = 1\n"), [originalMcp, setOriginalMcp] = useState("schema_version = 1\n");
  const [handle, setHandle] = useState("mcp:"), [secret, setSecret] = useState("");
  const [hosts, setHosts] = useState<readonly AgentExecutionHost[]>([]), [servers, setServers] = useState<readonly AgentMcpServerSummary[]>([]);
  const [mcpRefresh, setMcpRefresh] = useState(0);
  const initialRead = useRef<{gateway: typeof gateway; skills: typeof skills} | undefined>(undefined);
  useEffect(() => { if (!active) return;
    let live = true; void tools.list().then(value => { if (live) setServers(value); }).catch(() => { if (live) setStatus("无法读取 MCP 服务器，请重试。"); }); return () => { live = false; }; }, [active, tools, mcpRefresh]);
  useEffect(() => { if (!active) return;
    let live = true; void gateway.executionHosts().then(value => {if (live) setHosts(value);}).catch(() => undefined); return () => {live = false;}; }, [active, gateway]);
  useEffect(() => { if (!active) return;
    const loaded = initialRead.current?.gateway === gateway && initialRead.current.skills === skills;
    let live = true; void Promise.all([skills.list(), loaded ? Promise.resolve(undefined) : gateway.readMcp()]).then(([value, text]) => { if (live) { setCatalog(value); if (text !== undefined) {setMcp(text); setOriginalMcp(text);} initialRead.current = {gateway, skills}; } }).catch(() => { if (live) setStatus("无法读取 Agent 资源，请在桌面应用中重试。"); }); return () => { live = false; }; }, [active, gateway, skills]);
  async function action(work: () => Promise<void | string>, success: string): Promise<void> {
    if (busy) return; setBusy(true); setStatus("");
    try { const message = (await work()) ?? success; if (message.includes("取消")) notify.info(message); else notify.success(message); } catch (error) { const message = typeof error === "string" ? error : error instanceof Error ? error.message : "操作失败，输入仍保留，请重试。"; if (page === "list") notify.error(message); else setStatus(message); } finally { setBusy(false); }
  }
  async function scan(source: Parameters<typeof gateway.scan>[0]): Promise<void> {
    if (preview) await gateway.cancel(preview.importId);
    const value = await gateway.scan(source); setPreview(value); setSelected(value.candidates.filter(item => !item.existingRevision).map(item => item.sourceId)); setUpdates([]); setPage("list");
  }
  const importLocal = (): void => { void action(async () => { const path = await gateway.chooseSkillDirectory(); if (path === null) return "已取消目录选择"; await scan({ kind: "local", directory: path }); }, "技能目录已扫描"); };
  async function saveMcp(text: string): Promise<void> {
    await gateway.saveMcp(text, originalMcp); const saved = await gateway.readMcp(); setMcp(saved); setOriginalMcp(saved); setMcpRefresh(value => value + 1);
  }
  async function addServer(draft: McpServerDraft): Promise<void> {
    await action(async () => { if (servers.some(server => server.serverId === draft.name.trim())) throw new Error("服务器名称已存在，请使用其他名称。"); await saveMcp(appendMcpServer(originalMcp, draft)); setPage("list"); }, "MCP 配置已保存，请批准需要连接的服务器");
  }
  const title = { list: "Agent 资源管理", git: "从 Git 导入技能", mcp: "连接自定义 MCP", config: "编辑 MCP 配置", credentials: "凭据管理" }[page];
  return <section className="settings-card agent-resource-settings" aria-label="Agent 资源管理">
    {page !== "list" && <button className="settings-back" type="button" onClick={() => setPage("list")}><ArrowLeft size={16} />返回</button>}
    <header className="settings-resource-heading"><div><h2>{title}</h2><p>{page === "list" ? "管理技能包、工具服务器与聊天脚本授权。" : page === "mcp" ? "保存连接配置后，再批准服务器供 Agent 使用。" : "配置保存在本机，保存后生效。"}</p></div>
      {page === "list" && <div className="settings-heading-actions"><button type="button" disabled={busy} aria-label="导入本地目录" onClick={importLocal}>浏览目录</button>
        <ActionMenu className="settings-add-menu" label="添加资源" placement="bottom-end" portal contentClassName="settings-command-menu" triggerClassName="settings-add-button" disabled={busy} trigger={<>添加 <ChevronDown size={16} /></>}>
          <ActionMenuItem onClick={importLocal}><FolderPlus size={17} />导入本地目录</ActionMenuItem>
          <ActionMenuItem onClick={() => setPage("git")}><GitBranch size={17} />从 Git 导入技能</ActionMenuItem>
          <ActionMenuItem onClick={() => setPage("mcp")}><Plug size={17} />添加 MCP 服务器</ActionMenuItem>
          <ActionMenuItem onClick={() => setPage("credentials")}><KeyRound size={17} />管理凭据</ActionMenuItem>
        </ActionMenu></div>}
    </header>
    {status && <p role="status" className="settings-operation-status">{status}</p>}
    {page === "git" && <form className="settings-detail-form" onSubmit={event => { event.preventDefault(); void action(() => scan({ kind: "git", url, ref: ref || null, subdirectory: directory || null }), "Git 技能目录已扫描"); }}>
      <SettingGroup label="Git 技能来源"><SettingField label="HTTPS Git 地址"><input aria-label="HTTPS Git 地址" autoFocus type="url" required placeholder="https://github.com/owner/repository.git" value={url} onChange={event => setUrl(event.target.value)} /></SettingField>
        <SettingField label="Ref（留空使用 HEAD）"><input aria-label="Ref（留空使用 HEAD）" value={ref} onChange={event => setRef(event.target.value)} placeholder="分支、标签或提交" /></SettingField>
        <SettingField label="子目录（可选）"><input aria-label="子目录（可选）" value={directory} onChange={event => setDirectory(event.target.value)} /></SettingField></SettingGroup>
      <footer className="settings-form-footer"><button className="settings-primary" disabled={busy} type="submit">扫描 Git 技能</button></footer>
    </form>}
    {page === "mcp" && <McpServerForm busy={busy} onSave={addServer} />}
    {page === "config" && <form className="settings-detail-form" onSubmit={event => { event.preventDefault(); void action(async () => { await saveMcp(mcp); setPage("list"); }, "MCP 配置已保存，请批准需要连接的服务器"); }}><SettingGroup label="MCP 配置文件"><SettingField label="服务器配置（TOML）" description="保留已有配置；保存时核对原始版本，避免覆盖其他窗口的修改。"><textarea aria-label="MCP 服务器配置" value={mcp} onChange={event => setMcp(event.target.value)} rows={14} /></SettingField></SettingGroup><footer className="settings-form-footer"><button className="settings-primary" disabled={busy} type="submit">保存服务器配置</button></footer></form>}
    {page === "credentials" && <form className="settings-detail-form" onSubmit={event => { event.preventDefault(); void action(async () => { await gateway.credential(handle, secret); setSecret(""); setMcpRefresh(value => value + 1); }, "凭据已保存"); }}><SettingGroup label="本机凭据"><SettingField label="凭据句柄"><input aria-label="凭据句柄" required value={handle} onChange={event => setHandle(event.target.value)} autoComplete="off" /></SettingField><SettingField label="凭据" description="仅写入本机凭据库，不保存在 MCP 配置正文中。"><input aria-label="凭据" type="password" required value={secret} onChange={event => setSecret(event.target.value)} autoComplete="new-password" /></SettingField></SettingGroup><footer className="settings-form-footer"><button disabled={busy} type="button" onClick={() => void action(async () => { await gateway.credential(handle, null); setSecret(""); setMcpRefresh(value => value + 1); }, "凭据已删除")}>删除凭据</button><button className="settings-primary" disabled={busy} type="submit">保存凭据</button></footer></form>}
    {page === "list" && <>
      {preview && <section className="settings-section" aria-label="选择导入技能"><h3>选择导入技能</h3>{preview.resolvedCommit && <p>Commit: <code>{preview.resolvedCommit}</code></p>}<SettingGroup label="待导入技能">{preview.candidates.map(item => <SettingRow key={item.sourceId} title={item.name} description={item.sourceId}><label><input type="checkbox" checked={selected.includes(item.sourceId)} onChange={event => setSelected(old => event.target.checked ? [...old, item.sourceId] : old.filter(id => id !== item.sourceId))} />导入</label>{item.existingRevision && <label><input type="checkbox" checked={updates.includes(item.sourceId)} onChange={event => setUpdates(old => event.target.checked ? [...old, item.sourceId] : old.filter(id => id !== item.sourceId))} />更新现有技能</label>}</SettingRow>)}</SettingGroup><footer className="settings-form-footer"><button disabled={busy} type="button" onClick={() => void action(async () => { await gateway.cancel(preview.importId); setPreview(null); }, "已取消导入")}>取消</button><button className="settings-primary" disabled={busy || !selected.length || preview.candidates.some(item => selected.includes(item.sourceId) && !!item.existingRevision && !updates.includes(item.sourceId))} type="button" onClick={() => void action(async () => { await gateway.install(preview.importId, preview.candidates.filter(item => selected.includes(item.sourceId)).map(item => ({ sourceId: item.sourceId, expectedExistingRevision: item.existingRevision }))); setPreview(null); setCatalog(await skills.list()); }, "技能已安装")}>安装所选技能</button></footer></section>}
      <section className="settings-section"><h3>技能</h3><SettingGroup label="已安装技能">{catalog?.skills.map(skill => <SettingRow key={skill.sourceId} title={skill.name} description={skill.description}><button className="resource-enable-switch" role="switch" aria-checked={skill.enabled} aria-label={`启用技能：${skill.name}`} disabled={busy} onClick={() => void action(async () => setCatalog(await skills.setEnabled(skill.sourceId, skill.revision, !skill.enabled)), "技能状态已更新")} type="button" /><button className="settings-icon-button" aria-label={`删除技能：${skill.name}`} disabled={busy} onClick={() => void action(async () => { await gateway.remove(skill.sourceId, skill.revision); setCatalog(await skills.list()); }, "技能已删除")} type="button"><Trash2 size={17} /></button></SettingRow>)}{catalog?.skills.length === 0 && <p className="settings-empty">尚未安装技能</p>}</SettingGroup>{catalog?.errors.map(error => <p role="alert" key={error.sourceId}>{error.sourceId} · {error.code}</p>)}</section>
      <section className="settings-section"><div className="settings-section-toolbar"><h3>MCP 服务器</h3><button className="settings-icon-button" type="button" aria-label="编辑 MCP 配置" onClick={() => setPage("config")}><Settings2 size={17} /></button></div><SettingGroup label="MCP 服务器">{servers.map(server => <SettingRow key={server.serverId} title={server.serverId} description={!server.enabled ? "已禁用" : !server.credentialConfigured ? "未配置凭据" : server.approved ? "已批准" : "等待批准"}><button className="resource-enable-switch" type="button" role="switch" aria-label={`批准 MCP 服务器：${server.serverId}`} aria-checked={server.approved} disabled={busy || !server.enabled || !server.credentialConfigured} onClick={() => void action(async () => { if (server.approved) await tools.revoke(server.serverId); else await tools.install(servers.filter(item => item.enabled && item.credentialConfigured && (item.approved || item.serverId === server.serverId)).map(item => item.serverId)); setMcpRefresh(value => value + 1); }, "MCP 授权已更新")} /></SettingRow>)}{servers.length === 0 && <p className="settings-empty">暂无 MCP 服务器，可通过“添加”连接。</p>}</SettingGroup></section>
      <section className="settings-section"><h3>聊天脚本执行</h3><SettingGroup label="聊天脚本授权">{hosts.map(host => <SettingRow key={host.conversationId} title={host.title} description={host.kind}><button disabled={busy} type="button" onClick={() => void action(async () => { await gateway.authorizeShell(host.conversationId, !host.approved); setHosts(await gateway.executionHosts()); }, "聊天脚本授权已更新")}>{host.approved ? "撤销脚本授权" : "授权脚本执行"}</button></SettingRow>)}{hosts.length === 0 && <p className="settings-empty">暂无需要管理脚本授权的对话。</p>}</SettingGroup></section>
    </>}
  </section>;
}
