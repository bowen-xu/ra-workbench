import { Popover } from "../../../components/overlay/OverlayPopup";
import { PopoverDisclosure, PopoverProvider, usePopoverStore, useStoreState } from "@ariakit/react";
import { useEffect, useRef } from "react";
import { ChevronDown, ChevronUp, Plus, X } from "lucide-react";
import { agentContextKey, type AgentContextSelection } from "@research-assistant/core/agent";
import { ControlHint } from "../../../components/ControlHint";
import { notify } from "../../../services/notifications";

/** Browsing recommendations never changes selection; only explicit +/× actions do. */
export function AgentContextRecommendations({primary, selected, suggestions, disabled, onChange, error}: {
  readonly primary: AgentContextSelection;
  readonly selected: readonly AgentContextSelection[];
  readonly suggestions: readonly AgentContextSelection[];
  readonly disabled: boolean;
  readonly error?: string;
  readonly onChange: (item: AgentContextSelection, remove: boolean) => Promise<void>;
}): React.JSX.Element {
  // ControlHint also provides a popover store; disclosures must use this explicit owner.
  const popover = usePopoverStore({placement: "top-end"});
  const expanded = useStoreState(popover, "open");
  const anchor = useRef<HTMLDivElement>(null);
  const primaryKey = agentContextKey(primary.resource);
  const selectedKeys = new Set(selected.map(item => agentContextKey(item.resource)));
  const added = selectedKeys.has(primaryKey);
  const candidates = [...new Map([...suggestions, ...selected].filter(item => agentContextKey(item.resource) !== primaryKey)
    .map(item => [agentContextKey(item.resource), item])).values()];
  // Group selected candidates nearest the disclosure without sorting by click order.
  const others = candidates.filter(item => selectedKeys.has(agentContextKey(item.resource)));
  const pending = candidates.filter(item => !selectedKeys.has(agentContextKey(item.resource)));
  const rows = [...pending, ...others];
  const hasRows = rows.length > 0;
  useEffect(() => {if (!hasRows) popover.hide();}, [hasRows, popover]);
  useEffect(() => {if (error) notify.error(error);}, [error]);
  return <PopoverProvider store={popover}>
    <ControlHint content={primary.label} wrap preferSidePlacement>
      <div ref={anchor} className="agent-context-recommendation agent-context-recommendation-row" role="group" data-added={added} data-expanded={expanded} data-has-others={rows.length > 0} data-has-count={others.length > 0} aria-label={`推荐持续上下文 ${primary.label}`}>
        <button className="agent-context-recommendation-toggle" type="button" disabled={disabled} aria-label={`${added ? "移除" : "添加"}推荐上下文 ${primary.label}`} onClick={() => void onChange(primary, added)}>{added ? <X aria-hidden="true"/> : <Plus aria-hidden="true"/>}</button>
        <PopoverDisclosure store={popover} disabled={!hasRows} className="agent-context-recommendation-disclosure" render={<button type="button" aria-label={`推荐上下文 ${primary.label}`}/>}>{rows.length > 0 && <span className="agent-context-recommendation-indicator" data-has-count={others.length > 0}>
          {others.length > 0 && <span className="agent-context-recommendation-count" aria-label={`另有 ${others.length} 项持续上下文`}>{others.length}</span>}
          <ChevronUp className="agent-context-recommendation-up" aria-hidden="true"/>
          <ChevronDown className="agent-context-recommendation-down" aria-hidden="true"/>
        </span>}<span className="agent-context-recommendation-label">{primary.label}</span>
          {/* Intrinsic grid sizing includes candidate names only while open, without layout reads. */}
          {expanded && rows.map(item => <span key={agentContextKey(item.resource)} aria-hidden="true" className="agent-context-recommendation-sizer" data-added={selectedKeys.has(agentContextKey(item.resource))}>{item.label}</span>)}
        </PopoverDisclosure>
      </div>
    </ControlHint>
    {/* Both parts follow the intrinsic width of the entire joined frame. */}
    {hasRows && <Popover store={popover} className="agent-context-recommendations-popover" aria-label="推荐上下文" portal fixed modal={false} gutter={0} sameWidth fitViewport flip={false} getAnchorRect={() => anchor.current?.getBoundingClientRect() ?? null}>
      <ul>{rows.map(item => {
        const key = agentContextKey(item.resource), added = selectedKeys.has(key);
        return <ControlHint key={key} content={item.label} preferSidePlacement><li className="agent-context-recommendation-row" tabIndex={-1} data-context-key={key} data-added={added}>
          <button className="agent-context-recommendation-toggle" type="button" disabled={disabled} aria-label={`${added ? "移除持续上下文" : "添加持续上下文"} ${item.label}`} onClick={() => void onChange(item, added)}>
            {added ? <X aria-hidden="true"/> : <Plus aria-hidden="true"/>}
          </button>
          <span className="agent-context-recommendation-label">{item.label}</span>
        </li></ControlHint>;
      })}</ul>
    </Popover>}
  </PopoverProvider>;
}
