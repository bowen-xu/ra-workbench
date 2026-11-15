import type { AgentModelIdentity } from "@research-assistant/core/agent";

/** Observed or estimated model-context use, independent of its UI host. */
export interface AgentContextUsage {
  readonly contextWindowTokens?: number;
  readonly inputTokens: number;
  readonly measurement?: "estimated" | "stale";
}

export type AgentContextUsageReader = (
  conversationId: string,
  model: AgentModelIdentity,
  signal: AbortSignal,
) => Promise<AgentContextUsage | undefined>;
