import {getStructuredResourceGateway} from "../../../services/structuredResourceGateway";
import { decodeAgentContextResource, agentContextKey, type AgentUserInput, type AgentContextResource, type AgentPickerResource, type AgentContextSelection } from "@research-assistant/core/agent";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import type { AgentResourceGateways } from "../agentResourceGateways";
import { agentMcpGateway, agentSkillCatalogGateway } from "../../../services/agentGateway";
import type {AgentExtensionFacets} from "../../../contracts/agentExtensions";
function pickerResource(item: AgentContextSelection, paperTitle?: string, sourceUrl?: string): AgentPickerResource {
  return {group:"files",label:item.label,source:paperTitle??"",input:{type:"mention",name:item.label,target:{kind:"file",sourceId:"context:"+JSON.stringify(item.resource),resourceId:item.resource.resourceId,...(sourceUrl ? {sourceUrl} : {})}}};
}
export function contextComposerResources(gateway:AgentContextGateway, existing?:AgentResourceGateways, opened:readonly AgentContextSelection[]=[], extensions?: AgentExtensionFacets):AgentResourceGateways {
  const expand = (items: readonly AgentContextSelection[]) => items.flatMap(item => item.resource.kind === "paperGroup"
    ? item.resource.parts.filter(part => part.resource.kind === "paper" || part.resource.kind === "note" || part.resource.kind === "structuredObject")
      .map(part => (part.resource.kind === "note" || part.resource.kind === "structuredObject")
        ? pickerResource({resource:part.resource,label:part.label},item.label, part.resource.kind === "structuredObject" ? `source://library/${item.resource.resourceId}/${part.resource.resourceId}` : undefined)
        : pickerResource({resource:part.resource,label:`${item.label} · ${part.label}`})) : [pickerResource(item)]);
  const tools = existing?.tools ?? (extensions === undefined ? agentMcpGateway : extensions.mcp?.catalog);
  const skills = existing?.skills ?? (extensions === undefined ? agentSkillCatalogGateway : extensions.skills?.catalog);
  const readSkillDocument = existing?.readSkillDocument ?? extensions?.skills?.readDocument;
  return {...existing, ...(tools ? {tools} : {}), ...(skills ? {skills} : {}), ...(readSkillDocument ? {readSkillDocument} : {}),
    openedResources:expand(opened),
    refreshOpenedResources: async () => expand(await Promise.all(opened.map(async item => {
      if (item.resource.kind !== "paperGroup" || !gateway.catalog) return item;
      const rows = await gateway.catalog(item.label);
      return rows.find(row => row.resource.kind === "paperGroup" && row.resource.resourceId === item.resource.resourceId) ?? item;
    }))),
    conversationResources:existing?.conversationResources ?? (async(query)=> Promise.all((await gateway.search(query)).map(async item=>pickerResource(item, undefined, item.resource.kind === "structuredObject" ? (await getStructuredResourceGateway().get(item.resource.resourceId)).sourceUri : undefined))))};
}
export function turnContextResources(input:readonly AgentUserInput[] = []):readonly AgentContextResource[] {
  const resources=input.flatMap(item=>item.type==="mention"&&item.target.kind==="file"&&item.target.sourceId.startsWith("context:")?[decodeAgentContextResource(JSON.parse(item.target.sourceId.slice(8)))]:[]);
  return [...new Map(resources.map(resource=>[agentContextKey(resource),resource])).values()];
}
