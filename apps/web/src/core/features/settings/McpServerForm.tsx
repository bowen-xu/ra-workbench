import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { SettingField, SettingGroup, SettingRow } from "./SettingsLayout";
import type { McpServerDraft } from "./mcpServerDraft";

export function McpServerForm({ busy, onSave }: { readonly busy: boolean; readonly onSave: (draft: McpServerDraft) => Promise<void> }): React.JSX.Element {
  const [draft, setDraft] = useState<McpServerDraft>({ name: "", transport: "stdio", program: "", directory: "", url: "", arguments: [], environment: [], credentialHandle: "" });
  const update = <K extends keyof McpServerDraft>(key: K, value: McpServerDraft[K]): void => setDraft(old => ({ ...old, [key]: value }));
  return <form className="settings-detail-form" onSubmit={event => { event.preventDefault(); void onSave(draft); }}>
    <SettingGroup label="MCP 基础设置">
      <SettingField label="名称"><input aria-label="MCP 服务器名称" autoFocus required value={draft.name} onChange={event => update("name", event.target.value)} placeholder="MCP server name" /></SettingField>
      <SettingRow title="类型"><div className="settings-segmented" aria-label="MCP 类型">{([['stdio', 'STDIO'], ['streamable_http', 'Streamable HTTP']] as const).map(([value, label]) => <button key={value} type="button" aria-pressed={draft.transport === value} onClick={() => update("transport", value)}>{label}</button>)}</div></SettingRow>
    </SettingGroup>
    <SettingGroup label="MCP 连接设置">{draft.transport === "stdio" ? <>
      <SettingField label="启动命令"><input aria-label="启动命令" required placeholder="/absolute/path/to/server" value={draft.program} onChange={event => update("program", event.target.value)} /></SettingField>
      <SettingField label="参数"><div className="settings-repeat-list">{draft.arguments.map((value, index) => <div className="settings-repeat-row" key={index}><input aria-label={`参数 ${index + 1}`} required value={value} onChange={event => update("arguments", draft.arguments.map((item, i) => i === index ? event.target.value : item))} /><button className="settings-icon-button" type="button" aria-label={`删除参数 ${index + 1}`} onClick={() => update("arguments", draft.arguments.filter((_, i) => i !== index))}><Trash2 size={16} /></button></div>)}</div>
        <button className="settings-add-row" type="button" disabled={draft.arguments.length >= 64} onClick={() => update("arguments", [...draft.arguments, ""])}><Plus size={16} />添加参数</button></SettingField>
      <SettingField label="环境变量" description="仅填写非敏感配置；凭据使用下方句柄。不会继承宿主环境。"><div className="settings-repeat-list">{draft.environment.map((item, index) => <div className="settings-repeat-row" key={index}>{(['key', 'value'] as const).map(key => <input key={key} aria-label={`环境变量 ${index + 1} ${key === 'key' ? '名称' : '值'}`} placeholder={key === 'key' ? 'Key' : 'Value'} required value={item[key]} onChange={event => update("environment", draft.environment.map((row, i) => i === index ? { ...row, [key]: event.target.value } : row))} />)}<button className="settings-icon-button" type="button" aria-label={`删除环境变量 ${index + 1}`} onClick={() => update("environment", draft.environment.filter((_, i) => i !== index))}><Trash2 size={16} /></button></div>)}</div>
        <button className="settings-add-row" type="button" disabled={draft.environment.length >= 64} onClick={() => update("environment", [...draft.environment, { key: "", value: "" }])}><Plus size={16} />添加环境变量</button></SettingField>
      <SettingField label="工作目录"><input aria-label="工作目录" required placeholder="/absolute/path/to/workspace" value={draft.directory} onChange={event => update("directory", event.target.value)} /></SettingField>
    </> : <SettingField label="服务地址"><input aria-label="MCP 服务地址" type="url" required placeholder="https://example.com/mcp" value={draft.url} onChange={event => update("url", event.target.value)} /></SettingField>}
      <SettingField label="凭据句柄（可选）"><input aria-label="MCP 凭据句柄" placeholder="mcp:server" value={draft.credentialHandle} onChange={event => update("credentialHandle", event.target.value)} /></SettingField>
    </SettingGroup>
    <footer className="settings-form-footer"><button className="settings-primary" type="submit" disabled={busy}>{busy ? "正在保存…" : "保存"}</button></footer>
  </form>;
}
