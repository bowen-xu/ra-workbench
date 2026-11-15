import { Sparkles } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { ActionMenu, ActionMenuItem } from "../../components/ActionMenu";
import { agentSkillCatalogGateway } from "../../services/agentGateway";
import { notify } from "../../services/notifications";
import type { AgentSkillCatalog, AgentSkillSummary } from "@research-assistant/core/agent";
import type { AgentSkillCatalogGateway } from "@research-assistant/data/agent";
import { useAgentMountedGuard, useAgentRequestGuard } from "./useAgentMountedGuard";
import "./AgentSkillControl.css";

interface Props {
  readonly disabled?: boolean;
  readonly iconOnly?: boolean;
  readonly gateway?: AgentSkillCatalogGateway;
  /** Optional per-run projection of the currently enabled source identities. */
  readonly onSelectionChange?: (sourceIds: readonly string[]) => void;
}

/** Shared application-skill control; bodies remain native and are never shown here. */
export function AgentSkillControl({ disabled = false, iconOnly = false, onSelectionChange, gateway }: Props): React.JSX.Element {
  const [open, setOpen] = useState(false);
  return <ActionMenu keepMounted className="agent-skill-control" contentClassName="agent-skill-menu" label="配置 Agent 技能" menuLabel="Agent 技能" trigger={<><Sparkles aria-hidden="true" size={15} />{!iconOnly && <span>技能</span>}</>} triggerClassName={`agent-skill-trigger${iconOnly ? " agent-skill-trigger-icon" : ""}`} open={open} setOpen={setOpen} disabled={disabled}>
    <AgentSkillMenuItems active={open} disabled={disabled} {...(gateway ? { gateway } : {})} {...(onSelectionChange ? { onSelectionChange } : {})} />
  </ActionMenu>;
}

/** Shares catalog loading and versioned actions with menus owned by a host. */
export function AgentSkillMenuItems({ active: menuActive, disabled = false, onSelectionChange, gateway, itemClassName = "" }: Pick<Props, "disabled" | "gateway" | "onSelectionChange"> & {
  readonly active: boolean;
  readonly itemClassName?: string;
}): React.JSX.Element {
  const [catalog, setCatalog] = useState<AgentSkillCatalog | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [saving, setSaving] = useState<string | null>(null);
  const selectionChangeRef = useRef(onSelectionChange);
  const beginCatalogRequest = useAgentRequestGuard();
  const mountedRef = useAgentMountedGuard();

  useEffect(() => {
    selectionChangeRef.current = onSelectionChange;
  }, [onSelectionChange]);

  useEffect(() => {
    // A workflow host with a selection callback needs the enabled set before
    // its first run, even when the user never opens the menu. Other hosts keep
    // the existing lazy catalog load until the menu is opened.
    const active = beginCatalogRequest();
    if (!menuActive && selectionChangeRef.current === undefined) return;
    setLoading(true);
    setLoadFailed(false);
    void (gateway ?? agentSkillCatalogGateway).list()
      .then((next) => {
        if (!active()) return;
        setCatalog(next);
        selectionChangeRef.current?.(next.skills.filter((skill) => skill.enabled).map((skill) => skill.sourceId));
      })
      .catch((error: unknown) => {
        if (!active()) return;
        // Workflow hosts prefetch the selection in the background. Keep that
        // path quiet when the desktop gateway is unavailable; an explicit menu
        // open still reports the actionable error to the user.
        setLoadFailed(true);
        if (menuActive) notify.error(`无法读取 Agent 技能：${String(error)}`);
      })
      .finally(() => {
        if (active()) setLoading(false);
      });
  }, [beginCatalogRequest, gateway, menuActive, onSelectionChange !== undefined]);

  async function toggle(skill: AgentSkillSummary): Promise<void> {
    if (disabled || saving !== null || loading) return;
    setSaving(skill.sourceId);
    try {
      const next = await (gateway ?? agentSkillCatalogGateway).setEnabled(skill.sourceId, skill.revision, !skill.enabled);
      if (!mountedRef.current) return;
      setCatalog(next);
      selectionChangeRef.current?.(next.skills.filter((entry) => entry.enabled).map((entry) => entry.sourceId));
      notify.success(`${skill.name} 已${skill.enabled ? "停用" : "启用"}`);
    } catch (error) {
      if (mountedRef.current) notify.error(`技能状态未更新：${String(error)}`);
    } finally { if (mountedRef.current) setSaving(null); }
  }

  return <>
    {loading ? <span className="agent-skill-state">正在读取…</span> : loadFailed ? <span className="agent-skill-state">无法读取技能，请重新打开菜单重试。</span> : catalog === null || (catalog.skills.length === 0 && catalog.errors.length === 0) ? <span className="agent-skill-state">当前没有可用技能。</span> : <>
      {catalog.skills.map((skill) => <ActionMenuItem className={`agent-skill-item ${itemClassName}`} disabled={disabled || saving !== null} key={skill.sourceId} onClick={() => void toggle(skill)}>
        <Sparkles aria-hidden="true" size={16} />
        <span><strong>{skill.name}</strong><small>{skill.enabled ? "已启用 · 点击停用" : "已停用 · 点击启用"}</small></span>
      </ActionMenuItem>)}
      {catalog.errors.map((error) => <span className="agent-skill-error" key={error.sourceId}>{error.sourceId} · {error.code}</span>)}
    </>}
  </>;
}
