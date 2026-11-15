import { useEffect, useState } from "react";
import type { AgentConversationProjection, AgentModelIdentity } from "@research-assistant/core/agent";
import { isDesktopRuntime } from "../../runtime/platformCore";
import { isNativeAgentPreview } from "../../runtime/appRuntime";
import { readNativeContextUsage } from "./agentContextUsageGateway";
import type { AgentContextUsage as WritingAgentContextUsage } from "../../contracts/agentContext";

/** Observe native per-sampling-window measurements; never sum billed turn usage. */
export function useAgentContextUsage(projection: AgentConversationProjection | null, reader = isDesktopRuntime() || isNativeAgentPreview() ? readNativeContextUsage : undefined, selectedModel?: Pick<AgentModelIdentity,"providerId"|"modelId">): WritingAgentContextUsage | undefined {
  const turnId = projection?.turnOrder.at(-1), turn = turnId ? projection?.turns.get(turnId) : undefined;
  const historicalModel = turn?.model;
  const model = selectedModel && (selectedModel.providerId !== historicalModel?.providerId || selectedModel.modelId !== historicalModel?.modelId) ? undefined : historicalModel;
  const conversationId = projection?.conversationId;
  const attemptId = turn?.attemptOrder.at(-1), attempt = attemptId ? turn?.attempts.get(attemptId) : undefined;
  const compactions = turn?.itemOrder.flatMap(itemId => {
    const item = turn.items.get(itemId);
    return item?.kind.type === "contextCompaction" ? [[itemId, item.completed]] : [];
  });
  const owner = JSON.stringify([conversationId, model]);
  const sample = JSON.stringify([turnId, attemptId, attempt?.outcome, attempt?.inputTokens, attempt?.outputTokens, turn?.outcome, compactions]);
  const [saved, setSaved] = useState<{owner: string; sample: string; usage: WritingAgentContextUsage | undefined}>();
  useEffect(() => {
    if (!reader || !model || !conversationId) return;
    const controller = new AbortController();
    void reader(conversationId, model, controller.signal).then(usage => {
      if (!controller.signal.aborted) setSaved({owner, sample, usage});
    }).catch(() => { if (!controller.signal.aborted) setSaved({owner, sample, usage: undefined}); });
    return () => controller.abort();
    // Content deltas do not change the sampling measurement or trigger IPC.
  }, [reader, owner, sample]);
  return saved?.owner === owner && saved.sample === sample ? saved.usage : undefined;
}
