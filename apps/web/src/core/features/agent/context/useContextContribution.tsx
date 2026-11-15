import { useNativeConversationContext } from "./useNativeConversationContext";
import { useEffect, useMemo, useState } from "react";
import { freezeAgentContext, EMPTY_AGENT_CONTEXT, type AgentContextState } from "@research-assistant/core/agent";
import { readConversationContext, subscribeConversationContext } from "./contextStorage";
import { AgentContextControl } from "./AgentContextControl";
import { ContextConfigurationRecovery } from "./ContextConfigurationRecovery";
import { agentContextGateway, type AgentContextGateway } from "../../../services/agentContextGateway";
import { contextComposerResources, turnContextResources } from "./contextComposerResources";
import type { AgentResourceGateways } from "../agentResourceGateways";
import { notify } from "../../../services/notifications";
import { useContextRecommendations } from "./useContextRecommendations";
import { useAgentContextSuggestions } from "./AgentContextProvider";
import { useCaptureAgentNavigation } from "../AgentCanvasNavigationGuard";
import {useAgentExtensions} from "../../../extensions/AgentExtensionProvider";
/** Keyed by conversation by its host; configuration never enters the Agent loop. */
export function useContextContribution(id:string|null,disabled:boolean,existing?:AgentResourceGateways,_manuscriptId?:string,pending=false,gateway:AgentContextGateway=agentContextGateway,
  access: {readonly readOnly?: boolean} = {}) {
  const [context,setContext]=useState(()=>{try{return id?readConversationContext(id):EMPTY_AGENT_CONTEXT;}catch{return EMPTY_AGENT_CONTEXT;}});
  useEffect(()=>id?subscribeConversationContext(id,()=>{try{setContext(readConversationContext(id));}catch{notify.error("无法读取对话上下文，请重试。");}}):undefined,[id]);
  const native = useNativeConversationContext(id,pending,gateway,setContext,() => notify.error("无法恢复对话上下文，请重试。"),access);
  async function change(next:AgentContextState):Promise<void> {if(!id)return;try{await native.save(next);}catch(reason){notify.error("无法保存对话上下文，请重试。");throw reason;}}
  const opened = useAgentContextSuggestions();
  const extensions = useAgentExtensions();
  const captureDirectoryNavigation = useCaptureAgentNavigation();
  const resources=useMemo(()=>contextComposerResources(gateway,existing,opened,extensions),[gateway,existing,opened,extensions]);
  const recommendations = useContextRecommendations(id, context, pending);
  const recovery = id && native.recovery ? <ContextConfigurationRecovery conversationId={id} recovery={native.recovery} detail={native.detail} conflicts={native.conflicts}
    onRetry={()=>void native.retry().catch(()=>notify.error("无法核对对话上下文，请重试。"))}
    onRepair={()=>void native.repair().catch(()=>notify.error("无法修复对话上下文，请重试。"))}
    onResolve={(conflict,action)=>void native.resolve(conflict,action).catch(()=>notify.error("无法处理上下文冲突，请重试。"))}/> : null;
  return {resources,context,change,ready:native.ready,loading:native.loading,saving:native.saving,recovery,flush:()=>native.save(context),controls:id?<AgentContextControl recommendations={recommendations} pending={pending} conversationId={id} state={context} onChange={change} gateway={gateway} disabled={disabled || (!pending && !native.ready) || native.loading || native.saving} captureDirectoryNavigation={captureDirectoryNavigation} onDirectoryChanged={directory=>window.dispatchEvent(new CustomEvent("research-assistant:agent-directory-changed",{detail:{id,directory}}))}/>:null,
    selected:(input:Parameters<typeof turnContextResources>[0],requestId:string)=>[...freezeAgentContext(recommendations.capture(requestId)).map(item=>item.resource),...turnContextResources(input)],
    accept:async(requestId:string)=>{if(!native.ready)throw new Error("CONTEXT_CONFIGURATION_NOT_READY");
      const next=recommendations.prepareCommit(requestId);await native.save(next);recommendations.completeCommit(requestId);},
  };
}
