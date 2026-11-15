import { useEffect, useRef, useState } from "react";
import type { AgentQueueState } from "@research-assistant/core/agent";
import { AgentInputQueueController, type AgentInputQueueRegistry, type AgentInputQueueState, type AgentQueueGateway } from "@research-assistant/data/agent";
import {captureQueueOwnerIntent} from "./captureQueueOwnerIntent";

/** Reconcile the original queue edit before retrying a history-confirmed transfer. */
export async function reconcileSteeringTransfer<S, Q extends AgentQueueState>(
  controller: AgentInputQueueController<S, Q> | null, inputId: string, retryConfirmation: (inputId: string) => void,
): Promise<void> {
  if (!controller) return;
  if (controller.getPendingRequest()) await controller.retry();
  else await controller.reload();
  const state = controller.getSnapshot();
  if (state.queue && !state.loading && !state.editing && !state.uncertain && !state.error && !controller.getPendingRequest()) retryConfirmation(inputId);
}

/** The view subscribes without owning an unresolved app request. Scope is memoized
 * by each typed host; metadata readiness gates reads, not reattachment. */
export function useAgentInputQueue<S extends { conversationId: string }, Q extends AgentQueueState>(
  gateway: AgentQueueGateway<S, Q>, scope: S | null, ready: boolean, refresh: number,
  registry?: AgentInputQueueRegistry<S, Q>,
) {
  const controller = useRef<AgentInputQueueController<S, Q> | null>(null);
  const [owned, setOwned] = useState<{scope:S; gateway:AgentQueueGateway<S,Q>; registry:AgentInputQueueRegistry<S,Q>|undefined; state:AgentInputQueueState<Q>} | null>(null);
  useEffect(() => {
    setOwned(null);
    if (!scope) return;
    const lease = registry?.acquire(scope);
    const value = registry ? lease?.controller : new AgentInputQueueController(gateway, scope, () => crypto.randomUUID(), captureQueueOwnerIntent);
    if (!value) return;
    controller.current = value;
    const publish = () => setOwned({scope, gateway, registry, state:value.getSnapshot()});
    publish();
    const unsubscribe = value.subscribe(publish);
    return () => {
      unsubscribe();
      if (lease) lease.release(); else value.dispose();
      controller.current = null;
    };
  }, [scope, gateway, registry]);
  useEffect(() => { if (ready) void controller.current?.reload(); }, [scope, ready, refresh, registry]);
  /** Keep a scoped queue owner alive while a queued item is transferred into
   * same-turn Steer and the current host may unmount. */
  function retainTransfer(edit: import("@research-assistant/core/agent").AgentQueueEdit): { apply: () => Promise<boolean>; release: () => void } | null {
    if (!scope) return null;
    let lease = registry?.acquire(scope);
    let value = lease?.controller ?? (!registry ? controller.current : null);
    if (!value) return null;
    let released = false;
    const release = () => { if (!released) { released = true; lease?.release(); } };
    let attempted = false;
    return { release, apply: async () => {
      if (released) {
        lease = registry?.acquire(scope);
        value = lease?.controller ?? (!registry ? controller.current : null);
        released = false;
      }
      if (!value) return false;
      try {
        const current = value.getSnapshot();
        if (attempted && edit.type === "remove" && current.queue && !current.loading && !current.editing
          && !current.uncertain && !current.error && !current.queue.inputs.some(input => input.inputId === edit.inputId)) return true;
        attempted = true;
        return await value.edit(edit);
      }
      // The registry retains an uncertain edit itself. The transfer's lease is
      // needed until dispatch, not until an eventual user retry completes.
      finally { release(); }
    } };
  }
  function acquireOwner(){
    if(!scope)return null;
    if(registry)return registry.acquire(scope);
    return controller.current?{controller:controller.current,release:()=>undefined}:null;
  }
  const state = owned?.scope === scope && owned.gateway === gateway && owned.registry === registry ? owned.state : null;
  return { controller, state, retainTransfer, acquireOwner };
}
