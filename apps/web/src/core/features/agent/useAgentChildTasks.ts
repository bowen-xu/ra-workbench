import { useCallback, useEffect, useRef, useState } from "react";
import type { AgentChildTaskSummary } from "@research-assistant/core/agent";
import { agentSubtaskGateway } from "../../services/agentGateway";
import type { AgentSubtaskGateway } from "@research-assistant/data/agent";
import { useAgentRequestGuard } from "./useAgentMountedGuard";

/** Reads durable child state and keeps parent task cards current while a child runs. */
export function useAgentChildTasks(conversationId: string | null | undefined, refreshKey?: unknown, onError?: (reason: unknown) => void,
  gateway: Pick<AgentSubtaskGateway, "list" | "subscribe"> = agentSubtaskGateway): {
  readonly tasks: readonly AgentChildTaskSummary[];
  readonly reload: () => Promise<readonly AgentChildTaskSummary[]>;
} {
  const [tasks, setTasks] = useState<readonly AgentChildTaskSummary[]>([]);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const beginRequest = useAgentRequestGuard();
  const reload = useCallback(async (): Promise<readonly AgentChildTaskSummary[]> => {
    const isCurrent = beginRequest();
    if (!conversationId) { setTasks([]); return []; }
    try {
      const next = await gateway.list(conversationId);
      if (isCurrent()) setTasks(next);
      return next;
    } catch (reason) {
      if (isCurrent()) {
        setTasks([]);
        onErrorRef.current?.(reason);
      }
      throw reason;
    }
  }, [beginRequest, conversationId, gateway]);

  useEffect(() => {
    if (!conversationId) return;
    let active = true, unsubscribe: (() => void) | undefined;
    const refresh = (): void => {
      if (active && document.visibilityState !== "hidden") void reload().catch(() => undefined);
    };
    // Subscribe before the initial query; a single recheck closes async setup's gap.
    if (gateway.subscribe) void gateway.subscribe(conversationId, refresh).then(stop => {
      if (!active) stop();
      else { unsubscribe = stop; refresh(); }
    }).catch(reason => { if (active) onErrorRef.current?.(reason); });
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      active = false; unsubscribe?.();
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [conversationId, gateway, reload]);

  useEffect(() => {
    void reload().catch(() => undefined);
  }, [conversationId, refreshKey, reload]);

  return { tasks, reload };
}
