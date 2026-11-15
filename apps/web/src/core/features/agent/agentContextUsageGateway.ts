import type { AgentModelIdentity } from "@research-assistant/core/agent";
import { invoke } from "../../runtime/platformCore";
import type { AgentContextUsage } from "../../contracts/agentContext";

type ObjectValue = Record<string, unknown>;
function object(value: unknown): ObjectValue { return value !== null && typeof value === "object" ? value as ObjectValue : {}; }
function tokens(value: unknown): number | undefined { return typeof value === "number" && Number.isSafeInteger(value) && value >= 0 ? value : undefined; }
export function decodeContextUsage(value: unknown, conversationId: string): AgentContextUsage | undefined {
  const response = object(value), observation = object(response.observation), sample = object(observation.sample);
  if (response.schemaVersion !== 1 || response.conversationId !== conversationId) throw new Error("上下文用量身份无效");
  const requested = object(sample.model), actual = object(sample.actualModel);
  const sameModel = requested.providerId === actual.providerId && requested.modelId === actual.modelId && requested.revision === actual.revision;
  const input = tokens(sample.inputTokens), output = tokens(sample.outputTokens);
  const last = sameModel && input !== undefined && output !== undefined ? tokens(input + output) : tokens(sample.estimatedTokens);
  const current = tokens(response.currentTokens) ?? last;
  if (current === undefined) return undefined;
  const budget = object(sample.budget), rawWindow = tokens(budget.context_window_tokens);
  const percent = budget.effective_percent === undefined ? 100 : tokens(budget.effective_percent);
  const window = rawWindow !== undefined && percent !== undefined && percent > 0 && percent <= 100
    ? Number(BigInt(rawWindow) * BigInt(percent) / 100n) : undefined;
  return {inputTokens: current, ...(window ? {contextWindowTokens: window} : {}),
    ...(response.status === "stale" ? {measurement: "stale" as const} : response.status === "estimated" ? {measurement: "estimated" as const} : {})};
}
export async function readNativeContextUsage(conversationId: string, model: AgentModelIdentity, signal: AbortSignal): Promise<AgentContextUsage | undefined> {
  const result: unknown = await invoke("agent_read_context_usage", {request: {conversationId, model}});
  if (signal.aborted) return undefined;
  return decodeContextUsage(result, conversationId);
}
