/** Read the App-bound facets. Registration, approval and connection ownership
 * stay in App/Core; Native ToolRegistration remains the executable Tool API. */
export {useAgentExtensions} from "../extensions/AgentExtensionProvider";
export type {AgentExtensionContract, AgentExtensionFacets, AgentSkillFacet, AgentMcpFacet, AgentSkillCatalogReader, AgentMcpCatalogReader, AgentMcpServerSummary} from "../contracts/agentExtensions";
