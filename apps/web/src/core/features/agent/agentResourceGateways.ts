import type {AgentMcpCatalogReader, AgentSkillCatalogReader} from "../../contracts/agentExtensions";
import type { AgentPickerResource, AgentResourceScope, AgentSkillIdentity } from "@research-assistant/core/agent";

export interface AgentResourceGateways {
  readonly systemDocuments?: import("@research-assistant/core/agent").SystemDocumentGateway;
  /** Legacy UI group name: these are MCP server summaries, not ToolRegistry. */
  readonly tools?: AgentMcpCatalogReader;
  readonly skills?: AgentSkillCatalogReader;
  readonly hostResources?: (scope: AgentResourceScope) => Promise<readonly AgentPickerResource[]>;
  readonly openedResources?: readonly AgentPickerResource[];
  readonly refreshOpenedResources?: () => Promise<readonly AgentPickerResource[]>;
  readonly conversationResources?: (query: string) => Promise<readonly AgentPickerResource[]>;
  readonly readSkillDocument?: (identity: AgentSkillIdentity, scope?: AgentResourceScope) => Promise<string>;
}
