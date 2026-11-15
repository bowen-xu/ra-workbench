import {createContext, useContext, type PropsWithChildren} from "react";
import type {AgentExtensionFacets} from "../contracts/agentExtensions";
import {agentMcpGateway, agentSkillCatalogGateway} from "../services/agentGateway";
import {readAgentSkillDocument} from "../services/agentSkillDocumentGateway";

/** Existing Core owners also serve global chat and single-document editors
 * when no management plugin is mounted. No discovery or connection starts here. */
export const coreAgentExtensionFacets: AgentExtensionFacets = Object.freeze({owner: "core",
  skills: Object.freeze({provider: "core", catalog: Object.freeze({list: () => agentSkillCatalogGateway.list()}), readDocument: readAgentSkillDocument}),
  mcp: Object.freeze({provider: "core", catalog: Object.freeze({list: () => agentMcpGateway.list()})}),
});
const AgentExtensions = createContext<AgentExtensionFacets>(coreAgentExtensionFacets);

/** App-only binding; consumers cannot select an owner or create providers. */
export function AgentExtensionProvider({value, children}: PropsWithChildren<{readonly value: AgentExtensionFacets}>) {
  return <AgentExtensions.Provider value={value}>{children}</AgentExtensions.Provider>;
}

export function useAgentExtensions(): AgentExtensionFacets { return useContext(AgentExtensions); }
