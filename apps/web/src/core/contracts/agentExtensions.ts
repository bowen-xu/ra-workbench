import type {AgentResourceScope, AgentSkillCatalog, AgentSkillIdentity} from "@research-assistant/core/agent";

/** Distinct services, not permissions or interchangeable resource kinds. */
export type AgentExtensionContract = "core.agent.skills" | "core.agent.tools" | "core.agent.mcp";

export interface AgentSkillCatalogReader {
  list(): Promise<AgentSkillCatalog>;
}
export interface AgentMcpServerSummary {
  readonly serverId: string;
  readonly environmentId: string;
  readonly revision: string;
  readonly enabled: boolean;
  readonly credentialConfigured: boolean;
  readonly approved: boolean;
}
export interface AgentMcpCatalogReader {
  list(): Promise<readonly AgentMcpServerSummary[]>;
}
export interface AgentSkillFacet {
  readonly provider: string;
  readonly catalog: AgentSkillCatalogReader;
  /** Reading instructions does not grant Tool/MCP execution. */
  readonly readDocument?: (identity: AgentSkillIdentity, scope?: AgentResourceScope) => Promise<string>;
}
export interface AgentMcpFacet {
  readonly provider: string;
  /** Discovery summaries only; Core retains approval, credentials and clients. */
  readonly catalog: AgentMcpCatalogReader;
}
/** Web consumes catalogs/instructions. Executable Tool definitions/handlers use
 * the native research_agent_core ToolRegistration contract and Agent loop. */
export interface AgentExtensionFacets {
  readonly owner: string;
  readonly skills?: AgentSkillFacet;
  readonly mcp?: AgentMcpFacet;
}
