import { Plus } from "lucide-react";
import { useState } from "react";
import { ActionMenu } from "../../components/ActionMenu";
import type { AgentResourceScope, AgentReferenceInput } from "@research-assistant/core/agent";
import type { AgentResourceGateways } from "./agentResourceGateways";
import { AgentResourceList, useAgentResources } from "./AgentResourceList";
import {AgentFileMenu} from "./AgentFileMenu";
/** The plus menu shares the resource catalog and insertion path with @. */
export function AgentResourcePickerMenu({scope, gateways, onSelect, disabled=false}: {readonly scope?: AgentResourceScope; readonly gateways?: AgentResourceGateways; readonly onSelect: (input: AgentReferenceInput) => void; readonly disabled?: boolean}) {
  const [open, setOpen] = useState(false);
  const catalog = useAgentResources(open, gateways, scope);
  function select(input: AgentReferenceInput): void {onSelect(input); setOpen(false);}
  return <ActionMenu keepMounted hint portal label="添加上下文和工具" className="agent-resource-picker-menu" contentClassName="writing-agent-composer-add-menu" placement="top-start" disabled={disabled} open={open} setOpen={setOpen} trigger={<Plus size={18} />}>
    <AgentFileMenu open={open} {...(gateways ? {gateways} : {})} {...(scope ? {scope} : {})} onSelect={select} onClose={() => setOpen(false)}>
      <AgentResourceList {...catalog} groups={["tools", "skills"]} onSelect={select}/>
    </AgentFileMenu>
  </ActionMenu>;
}
