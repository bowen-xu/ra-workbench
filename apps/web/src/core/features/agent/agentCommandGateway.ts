import { getAppRuntime } from "../../runtime/appRuntime";

export interface AgentCommandUsage {
  readonly inputTokens: number;
  readonly contextWindowTokens: number;
}
export interface AgentCompactResult {
  readonly beforeTokens: number;
  readonly usage: AgentCommandUsage;
}
export interface AgentCommandRequest {
  readonly conversationId: string;
  readonly signal: AbortSignal;
}
export interface AgentCompactRequest extends AgentCommandRequest {
  /** Stable queue input identity for adapter receipt replay, scoped to conversation. */
  readonly requestId?: string;
}
/** Command I/O is separate from resource selection and message submission. */
export interface AgentCommandGateway {
  readUsage(request: AgentCommandRequest): Promise<AgentCommandUsage>;
  compact(request: AgentCompactRequest): Promise<AgentCompactResult>;
}

/** No native compaction port is connected during the UI-only stage. */
export function getAgentCommandGateway(): AgentCommandGateway | undefined {
  return getAppRuntime().gateways.agentCommandGateway;
}
