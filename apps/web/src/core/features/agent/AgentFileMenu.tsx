import {ArrowLeft, FileText} from "lucide-react";
import {useEffect, useRef, useState, type ReactNode} from "react";
import type {AgentReferenceInput, AgentResourceScope} from "@research-assistant/core/agent";
import {ActionMenuItem} from "../../components/ActionMenu";
import {AgentResourceList, useAgentResources, useAgentResourceSearch} from "./AgentResourceList";
import type {AgentResourceGateways} from "./agentResourceGateways";

/** The shared + entry reveals file search only after explicit expansion. */
export function AgentFileMenu({open, gateways, scope, onSelect, onClose, children}: {
  readonly open: boolean;
  readonly gateways?: AgentResourceGateways;
  readonly scope?: AgentResourceScope;
  readonly onSelect: (input: AgentReferenceInput) => void;
  readonly onClose: () => void;
  readonly children: ReactNode;
}): React.JSX.Element {
  const [expanded, setExpanded] = useState(false);
  const [query, setQuery] = useState("");
  const entry = useRef<HTMLButtonElement>(null), input = useRef<HTMLInputElement>(null);
  const catalog = useAgentResources(open && expanded, gateways, scope, query);
  const search = useAgentResourceSearch(catalog.resources.filter(resource => resource.group === "files"), query, onSelect, onClose);
  useEffect(() => {if (!open) {setExpanded(false); setQuery("");}}, [open]);
  function expand(): void {
    setExpanded(true);
    requestAnimationFrame(() => input.current?.focus());
  }
  if (!expanded) return <>
    <ActionMenuItem data-tab-action="forward" ref={entry} hideOnClick={false} aria-label="文件" className="writing-agent-composer-add-row" onClick={expand}
      onKeyDown={event => {
        if (!event.nativeEvent.isComposing && (event.key === "Tab" && !event.shiftKey || event.key === "ArrowRight")) {
          event.preventDefault(); event.stopPropagation(); expand();
        }
      }}>
      <FileText aria-hidden="true" size={16}/><span>文件</span><small className="agent-file-menu-hint">Tab 查看更多</small>
    </ActionMenuItem>
    {children}
  </>;
  return <>
    <ActionMenuItem hideOnClick={false} aria-label="返回添加菜单" className="writing-agent-composer-add-row" onClick={() => {
      setExpanded(false); requestAnimationFrame(() => entry.current?.focus());
    }}><ArrowLeft aria-hidden="true" size={16}/><span>文件</span></ActionMenuItem>
    <input data-tab-action="forward" ref={input} aria-label="搜索文件" className="agent-resource-search" type="search" value={query}
      onChange={event => setQuery(event.target.value)} onKeyDown={search.keyDown}/>
    <AgentResourceList {...catalog} groups={["files"]} query={query} onSelect={search.select} activeIndex={search.activeIndex}/>
  </>;
}
