import { agentContextGroupParts, agentContextKey, isAgentContextGroup, type AgentContextSelection, type AgentContextState } from "@research-assistant/core/agent";

/** Old individual selections remain readable; editing a group replaces only its members. */
export function selectedCatalogItem(item: AgentContextSelection, state: AgentContextState): AgentContextSelection | undefined {
  const existing = state.persistent.find(row => agentContextKey(row.resource) === agentContextKey(item.resource));
  if (!isAgentContextGroup(item.resource)) return existing;
  const saved = existing && isAgentContextGroup(existing.resource) ? existing.resource : undefined;
  const members = new Map(state.persistent.filter(row => !isAgentContextGroup(row.resource))
    .map(row => [agentContextKey(row.resource), row]));
  if (saved && !item.resource.parts.some(part => members.has(agentContextKey(part.resource)))) return existing;
  const standaloneAccess = (part: AgentContextSelection): "readOnly" | "readWrite" =>
    (part.resource.kind === "structuredObject" || part.resource.kind === "templateCatalog") &&
    part.resource.access === "readWrite" ? "readWrite" : "readOnly";
  const raiseCeiling = saved?.groupAccess === "readOnly" && item.resource.parts.some(part => {
    const selected = members.get(agentContextKey(part.resource));
    return selected && standaloneAccess(selected) === "readWrite";
  });
  const current = saved ? agentContextGroupParts(item.resource, saved, !raiseCeiling)
    : item.resource.parts.map(part => ({...part, selected: false, access: "readOnly" as const}));
  // Separate exact grants are additive to a group. Project their effective authority
  // without changing the durable representation merely by opening the dialog.
  const parts = current.map(part => {
    const standalone = members.get(agentContextKey(part.resource));
    if (!standalone) return part;
    const access = standaloneAccess(standalone);
    return {...part, selected: true,
      access: access === "readWrite" || part.selected && part.access === "readWrite" ? "readWrite" as const : "readOnly" as const};
  });
  const missing = saved?.parts.filter(part => !parts.some(value =>
    agentContextKey(value.resource) === agentContextKey(part.resource))) ?? [];
  if (!parts.some(part => part.selected) && !saved?.newMemberAccess) return undefined;
  return {...item, resource: {
    kind: item.resource.kind, resourceId: item.resource.resourceId, parts: [...parts, ...missing],
    ...(saved?.groupAccess ? {groupAccess: raiseCeiling ? "readWrite" : saved.groupAccess} : {}),
    ...(saved?.newMemberAccess ? {newMemberAccess: raiseCeiling ? "readOnly" : saved.newMemberAccess} : {}),
  }};
}
export function withoutCatalogMembers(state: AgentContextState, item: AgentContextSelection): AgentContextState {
  const keys = new Set([agentContextKey(item.resource), ...(isAgentContextGroup(item.resource) ? item.resource.parts.map(part => agentContextKey(part.resource)) : [])]);
  return {...state, persistent: state.persistent.filter(row => !keys.has(agentContextKey(row.resource))), temporary: state.temporary.filter(row => !keys.has(agentContextKey(row.resource)))};
}
