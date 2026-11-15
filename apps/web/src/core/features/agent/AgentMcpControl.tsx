import { useEffect, useRef, useState } from "react";
import { Wrench } from "lucide-react";
import { agentMcpGateway, type AgentMcpServerSummary } from "../../services/agentGateway";
import { ActionMenu, ActionMenuItem } from "../../components/ActionMenu";
import { notify } from "../../services/notifications";
import { useAgentMountedGuard, useAgentRequestGuard } from "./useAgentMountedGuard";
import "./AgentMcpControl.css";

interface Props {
  readonly disabled?: boolean;
  /** Use the icon-only trigger when the host already provides a dense action row. */
  readonly iconOnly?: boolean;
  readonly gateway?: typeof agentMcpGateway;
}

/** Shared host-owned MCP approval control. Secrets and local paths are never shown. */
export function AgentMcpControl({ disabled = false, iconOnly = false, gateway }: Props): React.JSX.Element {
  const [open, setOpen] = useState(false);
  return <ActionMenu keepMounted className="agent-mcp-control" contentClassName="agent-mcp-menu" label="配置 Agent 工具" menuLabel="Agent 工具" disabled={disabled}
    open={open} setOpen={setOpen} triggerClassName={`agent-mcp-trigger${iconOnly ? " agent-mcp-trigger-icon" : ""}`}
    trigger={<><Wrench aria-hidden="true" size={15} />{!iconOnly && <span>工具</span>}</>}>
    <AgentMcpMenuItems active={open} disabled={disabled} {...(gateway ? { gateway } : {})} />
  </ActionMenu>;
}

/** The same resource actions can be embedded in an existing command menu. */
export function AgentMcpMenuItems({ active: menuActive, disabled = false, gateway, itemClassName = "" }: Pick<Props, "disabled" | "gateway"> & {
  readonly active: boolean;
  readonly itemClassName?: string;
}): React.JSX.Element {
  const [servers, setServers] = useState<AgentMcpServerSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [saving, setSaving] = useState(false);
  const installPending = useRef(false);
  const mountedRef = useAgentMountedGuard();
  const beginListRequest = useAgentRequestGuard();
  useEffect(() => {
    const active = beginListRequest();
    if (!menuActive || saving) return;
    setLoading(true);
    setLoadFailed(false);
    void (gateway ?? agentMcpGateway).list()
      .then((value) => { if (active() && mountedRef.current) setServers(value); })
      .catch((error: unknown) => { if (active() && mountedRef.current) { setLoadFailed(true); notify.error(`无法读取 Agent 工具配置：${String(error)}`); } })
      .finally(() => { if (active() && mountedRef.current) setLoading(false); });
  }, [beginListRequest, gateway, menuActive, saving]);

  async function toggle(server: AgentMcpServerSummary): Promise<void> {
    if (installPending.current || loading || disabled || !server.enabled || !server.credentialConfigured) return;
    installPending.current = true;
    setSaving(true);
    try {
      const resourceGateway = gateway ?? agentMcpGateway;
      if (server.approved) await resourceGateway.revoke(server.serverId);
      else await resourceGateway.install(servers
        .filter((item) => item.enabled && item.credentialConfigured && (item.approved || item.serverId === server.serverId))
        .map((item) => item.serverId));
      if (mountedRef.current) notify.success("Agent 工具配置已更新");
    } catch (error) {
      if (mountedRef.current) notify.error(`Agent 工具配置未更新：${String(error)}`);
    } finally {
      installPending.current = false;
      if (mountedRef.current) setSaving(false);
    }
  }

  return <>
    {loading ? <small className="agent-mcp-state">正在读取…</small> : loadFailed ? <small className="agent-mcp-state">无法读取工具，请重新打开菜单重试。</small> : servers.length === 0 ? <small className="agent-mcp-state">当前没有可用的工具服务器。</small> : servers.map((server) =>
      <ActionMenuItem className={`agent-mcp-item ${itemClassName}`} disabled={disabled || !server.enabled || !server.credentialConfigured || saving} key={server.serverId} onClick={() => void toggle(server)}>
        <Wrench aria-hidden="true" size={16} />
        <span><strong>{server.serverId}</strong><small>{!server.enabled ? "已禁用" : !server.credentialConfigured ? "未配置凭据" : server.approved ? "已批准 · 点击撤销" : "点击批准"}</small></span>
      </ActionMenuItem>)}
  </>;
}
