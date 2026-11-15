import { FileText, Sparkles, Wrench } from "lucide-react";
import { useEffect, useState, type KeyboardEvent } from "react";
import type { AgentPickerResource, AgentReferenceInput, AgentResourceScope } from "@research-assistant/core/agent";
import { ActionMenuItem } from "../../components/ActionMenu";
import { agentHostResourceGateway } from "../../services/agentGateway";
import {useAgentExtensions} from "../../extensions/AgentExtensionProvider";
import type { AgentResourceGateways } from "./agentResourceGateways";
import "./AgentResourceList.css";

/** All resource entry points and hosts use this catalog. Selection never changes administration. */
export function useAgentResources(active: boolean, gateways?: AgentResourceGateways, scope?: AgentResourceScope, searchQuery?: string) {
  const extensions = useAgentExtensions();
  const [resources, setResources] = useState<readonly AgentPickerResource[]>([]);
  const [status, setStatus] = useState("");
  const scopeKey = scope ? JSON.stringify(scope) : "";
  useEffect(() => {
    if (!active) return;
    let live = true;
    setResources([]);
    setStatus("正在读取资源…");
    const host = searchQuery === undefined && gateways?.refreshOpenedResources ? gateways.refreshOpenedResources()
      : searchQuery === undefined && gateways?.openedResources !== undefined ? Promise.resolve(gateways.openedResources)
      : gateways?.conversationResources ? gateways.conversationResources(searchQuery ?? "")
      : scope ? (gateways ? gateways.hostResources?.(scope) ?? Promise.resolve([]) : agentHostResourceGateway(scope)) : Promise.resolve([]);
    if (searchQuery !== undefined) {
      void host.then(files => {if (live) {setResources(files.filter(resource => resource.group === "files")); setStatus("");}},
        () => {if (live) setStatus("文件读取失败，请重新打开重试。");});
      return () => {live = false;};
    }
    const skillsReader = gateways === undefined ? extensions.skills?.catalog : gateways.skills;
    const mcpReader = gateways === undefined ? extensions.mcp?.catalog : gateways.tools;
    void Promise.allSettled([skillsReader?.list(), mcpReader?.list(), host]).then(([skills, tools, files]) => {
      if (!live) return;
      const entries: AgentPickerResource[] = [];
      if (skills.status === "fulfilled" && skills.value) {
        const catalog = skills.value;
        entries.push(...catalog.skills.filter(skill => skill.enabled).map(skill => ({group: "skills" as const, label: skill.name, source: "应用技能", input: {type: "skill" as const, name: skill.name, identity: {environment_id: catalog.environmentId, source_id: skill.sourceId, revision: skill.revision}}})));
      }
      if (tools.status === "fulfilled" && tools.value) entries.push(...tools.value.map(server => ({group: "tools" as const, label: server.serverId, source: server.approved ? "已授权服务器" : "需在设置中授权", disabled: !server.enabled || !server.credentialConfigured || !server.approved, input: {type: "mention" as const, name: server.serverId, target: {kind: "tool" as const, serverId: server.serverId, environmentId: server.environmentId, revision: server.revision}}})));
      if (files.status === "fulfilled") entries.push(...files.value);
      const order = {files: 0, tools: 1, skills: 2};
      setResources(entries.sort((a, b) => order[a.group] - order[b.group]));
      setStatus(([skills, tools, files].some(result => result.status === "rejected") || (skills.status === "fulfilled" && skills.value && skills.value.errors.length > 0)) ? "部分资源读取失败，请重新打开重试。" : "");
    });
    return () => {live = false;};
  }, [active, gateways, scopeKey, searchQuery, extensions]);
  return {resources, status};
}
export function filterAgentResources(resources: readonly AgentPickerResource[], query: string) {
  const value = query.toLocaleLowerCase();
  return resources.filter(resource => `${resource.label} ${resource.source} ${resource.input.type === "skill" ? resource.input.identity.source_id : ""}`.toLocaleLowerCase().includes(value));
}
export function AgentResourceList({resources, status, query = "", onSelect, onActivate, activeIndex, groups = ["files", "tools", "skills"]}: {
  readonly groups?: readonly ("files"|"tools"|"skills")[];
  readonly resources: readonly AgentPickerResource[]; readonly status: string; readonly query?: string;
  readonly onSelect: (input: AgentReferenceInput) => void; readonly activeIndex?: number;
  readonly onActivate?: (input: AgentReferenceInput) => void;
}): React.JSX.Element {
  const filtered = filterAgentResources(resources, query);
  let index = 0;
  return <>{status && <p className="agent-resource-status" role="status">{status}</p>}
    {groups.map(group => {
      const items = filtered.filter(resource => resource.group === group);
      const Icon = group === "files" ? FileText : group === "tools" ? Wrench : Sparkles;
      return <section className={`agent-resource-group agent-resource-group-${group}`} key={group}>
        <div className="writing-agent-composer-add-section-title">{group === "files" ? "文件" : group === "tools" ? "工具" : "技能"}</div>
        {items.length === 0 ? <p className="agent-resource-status">{query ? "没有匹配项" : "暂无可用资源"}</p> : items.map(resource => {
          const selected = index++ === activeIndex;
          const caption = [resource.source, resource.input.type === "skill" ? resource.input.identity.source_id : ""].filter(Boolean).join(" · ");
          return <ActionMenuItem data-tab-action="forward" key={JSON.stringify(resource.input)} className={`agent-resource-item${selected ? " is-selected" : ""}`} disabled={resource.disabled ?? false} onMouseDown={activeIndex === undefined ? undefined : event => event.preventDefault()} onClick={() => onSelect(resource.input)}
            onFocus={() => onActivate?.(resource.input)} onPointerMove={() => {if (!resource.disabled) onActivate?.(resource.input);}}
            onKeyDown={event => {
              if (event.key === "Tab" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault(); event.stopPropagation(); onSelect(resource.input);
              }
            }}>
            <Icon aria-hidden="true" size={16} /><span><strong>{resource.label}</strong>{caption && <small>{caption}</small>}</span>
          </ActionMenuItem>;
        })}
      </section>;
    })}</>;
}

/** Search keeps focus while arrows and Enter select an enabled exact identity. */
export function useAgentResourceSearch(resources: readonly AgentPickerResource[], query: string, onSelect: (input: AgentReferenceInput) => void, close: () => void) {
  const [index, setIndex] = useState(0);
  useEffect(() => setIndex(0), [query]);
  const filtered = filterAgentResources(resources, query);
  const candidates = filtered.filter(resource => !resource.disabled);
  function select(input: AgentReferenceInput): void {onSelect(input); close();}
  function keyDown(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {event.preventDefault(); event.stopPropagation(); close();}
    else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault(); event.stopPropagation();
      setIndex(old => (old + (event.key === "ArrowDown" ? 1 : -1) + Math.max(1, candidates.length)) % Math.max(1, candidates.length));
    } else if ((event.key === "Enter" || event.key === "Tab" && !event.shiftKey) && candidates.length) {
      event.preventDefault(); event.stopPropagation();
      const candidate = candidates[Math.min(index, candidates.length - 1)];
      if (candidate) select(candidate.input);
    } else if (event.key === "Tab") {
      event.preventDefault(); event.stopPropagation();
    }
  }
  return {select, keyDown,
    activeIndex: filtered.indexOf(candidates[Math.min(index, Math.max(0, candidates.length - 1))]!)};
}
