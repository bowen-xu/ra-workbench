import { withoutCatalogMembers } from "./contextCatalogSelection";
import { agentContextKey, compactAgentContextSelection, decodeAgentContextSelection, selectAgentContext, type AgentContextSelection, type AgentContextState } from "@research-assistant/core/agent";

export interface ContextRecommendationChoice {
  readonly item: AgentContextSelection;
  readonly selected: boolean;
}
export function recommendationDraftKey(id: string): string {return `research-assistant:context-recommendations:${id}`;}
export function recommendationSubmissionKey(id: string): string {return `research-assistant:context-recommendations-submission:${id}`;}
export function decodeRecommendationChoices(value: unknown): readonly ContextRecommendationChoice[] {
  if (!Array.isArray(value) || value.length > 32) throw new Error("推荐选择无效");
  const choices = value.map(row => {
    if (!row || typeof row !== "object" || Object.keys(row).some(key => !["item", "selected"].includes(key)) || typeof row.selected !== "boolean") throw new Error("推荐选择无效");
    return {item: compactAgentContextSelection(decodeAgentContextSelection(row.item)), selected: row.selected};
  });
  if (new Set(choices.map(row => agentContextKey(row.item.resource))).size !== choices.length) throw new Error("推荐身份重复");
  return choices;
}
export function applyRecommendationChoices(context: AgentContextState, choices: readonly ContextRecommendationChoice[]): AgentContextState {
  return choices.reduce((state, row) => row.selected ? selectAgentContext(withoutCatalogMembers(state, row.item), row.item, "persistent") : withoutCatalogMembers(state, row.item), context);
}
