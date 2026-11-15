import type {AgentExtensionFacets, AgentMcpFacet, AgentSkillFacet} from "../core/contracts/agentExtensions";
import {coreAgentExtensionFacets} from "../core/extensions/AgentExtensionProvider";
import type {BuiltinCapabilityBinding} from "./pluginComposition";

/** Runtime owner adapters, separate from declarations and invocation selection.
 * They borrow existing providers; removing a view does not revoke or dispose it. */
export interface BuiltinAgentExtensionProvider {
  readonly owner: string;
  readonly skills?: AgentSkillFacet;
  readonly mcp?: AgentMcpFacet;
}
const coreProviders: readonly BuiltinAgentExtensionProvider[] = [coreAgentExtensionFacets];

/** Consume the already validated C01 resolution, without another registry,
 * execution path, discovery pass or implicit last-provider-wins rule. */
export function bindBuiltinAgentExtensions(selected: readonly {readonly id: string}[], bindings: readonly BuiltinCapabilityBinding[], providers: readonly BuiltinAgentExtensionProvider[] = coreProviders): ReadonlyMap<string, AgentExtensionFacets> {
  const adapters = new Map<string, BuiltinAgentExtensionProvider>();
  for (const provider of providers) {
    if (adapters.has(provider.owner)) throw new Error(`agent_extension_provider_duplicate: ${provider.owner}`);
    if (provider.skills && provider.skills.provider !== provider.owner || provider.mcp && provider.mcp.provider !== provider.owner) throw new Error(`agent_extension_provider_identity_mismatch: ${provider.owner}`);
    adapters.set(provider.owner, provider);
  }
  const bound = new Map<string, AgentExtensionFacets>();
  for (const plugin of selected) {
    const toolBinding = bindings.find(binding => binding.pluginId === plugin.id && binding.capabilityId === "core.agent.tools");
    // The current native assembly has one Core ToolRegistry. A declaration
    // cannot invent a replacement executable provider on the Web side.
    if (toolBinding && toolBinding.provider !== "core") throw new Error(`agent_tool_provider_requires_native_binding: ${plugin.id}`);
    const skillBinding = bindings.find(binding => binding.pluginId === plugin.id && binding.capabilityId === "core.agent.skills");
    const mcpBinding = bindings.find(binding => binding.pluginId === plugin.id && binding.capabilityId === "core.agent.mcp");
    const skills = skillBinding ? adapters.get(skillBinding.provider)?.skills : undefined;
    const mcp = mcpBinding ? adapters.get(mcpBinding.provider)?.mcp : undefined;
    if (skillBinding && !skills || mcpBinding && !mcp) throw new Error(`agent_extension_provider_unavailable: ${plugin.id}`);
    bound.set(plugin.id, Object.freeze({owner: plugin.id, ...(skills ? {skills} : {}), ...(mcp ? {mcp} : {})}));
  }
  return bound;
}
