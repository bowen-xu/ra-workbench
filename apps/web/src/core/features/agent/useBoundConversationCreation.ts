import { useEffect, useRef, useState } from "react";
import type { AgentConversationBinding } from "@research-assistant/core/agent";
import { getAppStorage } from "../../runtime/appStorage";
import { boundCreationScope } from "./boundConversationCreationPending";
import { acquireConversationCreationOwner, type ConversationCreationController, type ConversationCreationPorts, type ConversationCreationSnapshot } from "./conversationCreationController";

/** Subscriptions are views; retained leases own in-flight native operations. */
export function useBoundConversationCreation(binding: AgentConversationBinding | null, ports: ConversationCreationPorts) {
  const storage = getAppStorage(), owner = useRef<ConversationCreationController | null>(null);
  const scope = binding ? boundCreationScope(binding) : null;
  const [owned, setOwned] = useState<{ scope: string; state: ConversationCreationSnapshot } | null>(null);
  useEffect(() => {
    if (!binding || !scope) return;
    const lease = acquireConversationCreationOwner(binding, ports, storage), controller = lease.controller;
    owner.current = controller;
    const sync = () => setOwned({ scope, state: controller.getSnapshot() });
    const detach = controller.subscribe(sync); sync();
    return () => { detach(); lease.release(); if (owner.current === controller) owner.current = null; };
  }, [scope, ports.creation, ports.metadata, storage]);
  async function retained<T>(operation: (value: ConversationCreationController) => Promise<T>): Promise<T | null> {
    if (!binding) return null;
    const lease = acquireConversationCreationOwner(binding, ports, storage);
    try { return await operation(lease.controller); } finally { lease.release(); }
  }
  return { state: owned?.scope === scope ? owned.state : null, getSnapshot: () => owner.current?.getSnapshot() ?? null,
    submit: (id?: string) => retained(value => value.submit(id)), check: () => retained(value => value.check()),
    retryOriginal: () => retained(value => value.retryOriginal()), readCurrent: () => retained(value => value.readCurrent()),
    consumeAdoption: (adopt: () => boolean) => owner.current?.consumeAdoption(adopt) ?? false,
    clearAdopted: () => owner.current?.clearAdopted() ?? false, finishKnown: () => owner.current?.finishKnown() ?? false };
}
