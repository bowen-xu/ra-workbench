import { Network, Plus, Target } from "lucide-react";
import { useState } from "react";

import { ActionMenu, ActionMenuItem } from "../../../public/overlay";
import type { WritingAgentContextScope } from "./WritingAgentContextControl";
import { AgentResourceList, useAgentResources } from "../../../public/agentContext/AgentResourceList";
import type { AgentReferenceInput, AgentResourceScope } from "@research-assistant/core/agent";
import type { AgentResourceGateways } from "../../../public/agentContext/agentResourceGateways";
import { AgentFileMenu } from "../../../public/agentPresentation/AgentFileMenu";

interface WritingAgentComposerAddMenuProps {
  readonly activePath?: string;
  readonly contextScope: WritingAgentContextScope;
  readonly disabled: boolean;
  readonly goalMode: boolean;
  readonly includeConceptRetrieval: boolean;
  readonly onContextScopeChange: (value: WritingAgentContextScope) => void;
  readonly onGoalModeChange: (value: boolean) => void;
  readonly onIncludeConceptRetrievalChange: (value: boolean) => void;
  readonly resourceGateways?: AgentResourceGateways;
  readonly resourceScope?: AgentResourceScope;
  readonly onReferenceSelect?: (reference: AgentReferenceInput) => void;
}

export function WritingAgentComposerAddMenu({
  disabled, goalMode, includeConceptRetrieval, onGoalModeChange, onIncludeConceptRetrievalChange, resourceGateways, resourceScope, onReferenceSelect,
}: WritingAgentComposerAddMenuProps): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const resources = useAgentResources(open, resourceGateways, resourceScope);
  const choices = resources.resources.filter(resource=>resource.group!=="files");
  function selectReference(input: AgentReferenceInput): void {
    onReferenceSelect?.(input);
    setOpen(false);
  }
  return <ActionMenu
    keepMounted
    className={`writing-agent-composer-add${goalMode || includeConceptRetrieval ? " has-active-option" : ""}`}
    contentClassName="writing-agent-composer-add-menu"
    disabled={disabled}
    label="添加上下文和工具"
    hint
    portal
    placement="top-start"
    open={open}
    setOpen={setOpen}
    trigger={<Plus aria-hidden="true" size={18} />}
    triggerClassName="writing-agent-composer-add-trigger"
  >
    <AgentFileMenu open={open} {...(resourceGateways ? {gateways:resourceGateways} : {})} {...(resourceScope ? {scope:resourceScope} : {})} onSelect={selectReference} onClose={() => setOpen(false)}>
    <ActionMenuItem aria-label="目标模式" className="writing-agent-composer-add-row" disabled={disabled} onClick={() => onGoalModeChange(!goalMode)}>
      <Target aria-hidden="true" size={16} />
      <span>目标模式</span>
      {goalMode && <span className="writing-agent-composer-add-state">已启用</span>}
    </ActionMenuItem>
    <ActionMenuItem aria-label="概念网络检索" className="writing-agent-composer-add-row" disabled={disabled} onClick={() => onIncludeConceptRetrievalChange(!includeConceptRetrieval)}>
      <Network aria-hidden="true" size={16} />
      <span>概念网络检索</span>
      {includeConceptRetrieval && <span className="writing-agent-composer-add-state">已启用</span>}
    </ActionMenuItem>
    <AgentResourceList resources={choices} status={resources.status} groups={["tools","skills"]} onSelect={selectReference} />
    </AgentFileMenu>
  </ActionMenu>;
}
