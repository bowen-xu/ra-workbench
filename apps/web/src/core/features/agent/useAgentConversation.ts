import { useCallback, useEffect, useRef, useState } from "react";
import { AgentConversationController, type AgentConversationSnapshot } from "./conversationController";
import { agentSessionGateway } from "../../services/agentGateway";
import { notify } from "../../services/notifications";
import type { AgentSessionGateway } from "@research-assistant/data/agent";
import { agentHistoryErrorMessage } from "./agentHistoryError";

const projectionCache = new Map<string, AgentConversationSnapshot["projection"]>();
const MAX_CACHED_CONVERSATIONS = 24;
function cacheProjection(id: string, projection: AgentConversationSnapshot["projection"]): void {
  if (!projection) return;
  projectionCache.delete(id);
  projectionCache.set(id, projection);
  while (projectionCache.size > MAX_CACHED_CONVERSATIONS) {
    const oldest = projectionCache.keys().next().value;
    if (oldest === undefined) break;
    projectionCache.delete(oldest);
  }
}
/** Test-only reset; production keeps recent projections across panel remounts. */
export function resetAgentConversationProjectionCacheForTests(): void { projectionCache.clear(); }
const empty: AgentConversationSnapshot = { status: "idle", projection: null, error: null };
export interface AgentConversationView extends AgentConversationSnapshot {
  readonly reconnect: () => void;
}
const emptyView: AgentConversationView = { ...empty, reconnect: () => undefined };
export interface AgentConversationAccess {
  readonly identity: string;
  readonly gateway: AgentSessionGateway;
}
export function useAgentConversation(id: string | null, activation?: object | null, access?: AgentConversationAccess): AgentConversationView {
  const storageIdentity = access?.identity ?? "desktop";
  const gateway = access?.gateway ?? agentSessionGateway;
  const [owned, setOwned] = useState<{ id: string; storageIdentity: string; gateway: AgentSessionGateway; activation: object | null | undefined; snapshot: AgentConversationSnapshot } | null>(null);
  const controllerRef = useRef<AgentConversationController | null>(null);
  useEffect(() => {
    if (!id) { controllerRef.current = null; setOwned(null); return; }
    const identity = JSON.stringify([storageIdentity, id.toLowerCase()]);
    const cached = projectionCache.get(identity) ?? undefined;
    const controller = new AgentConversationController(gateway, cached);
    controllerRef.current = controller;
    const unsubscribe = controller.subscribe(() => {
      const next = controller.getSnapshot();
      cacheProjection(identity, next.projection);
      setOwned({ id, storageIdentity, gateway, activation, snapshot: next });
      if (next.status === "error") notify.error(agentHistoryErrorMessage(next.error, id));
    });
    void controller.open(id);
    return () => {
      unsubscribe();
      if (controllerRef.current === controller) controllerRef.current = null;
      controller.dispose();
    };
  }, [id, activation, storageIdentity, gateway]);
  // A repeated selection starts a fresh read; its previous ready state must not finish navigation.
  const reconnect = useCallback(() => { void controllerRef.current?.resume(); }, []);
  return owned?.id === id && owned.activation === activation && owned.storageIdentity === storageIdentity && owned.gateway === gateway
    ? { ...owned.snapshot, reconnect } : emptyView;
}
