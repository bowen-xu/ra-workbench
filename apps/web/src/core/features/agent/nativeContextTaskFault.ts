import type {AgentContextGateway} from "../../services/agentContextGateway";

/** Replace only the observation result for one real, completed preview task. */
export function nativeContextTaskFault(gateway: AgentContextGateway, enabled: () => boolean): AgentContextGateway {
  const source = gateway.taskObservation;
  if (!source) return gateway;
  let bound: string | undefined;
  const taskObservation = {async observe(request: Parameters<typeof source.observe>[0]) {
    const observed = await source.observe(request);
    if (!enabled()) {bound = undefined; return observed;}
    const key = JSON.stringify(request);
    if (bound === undefined) bound = key;
    if (bound !== key || observed.status.type !== "finished") return observed;
    return {...observed, status: {type: "finished" as const, completion: "recoveryRequired" as const,
      diagnostic: {code: "PREVIEW_TASK_RECOVERY", phase: "write", safeCause: "synthetic_observation_failure",
        conversationId: request.conversationId, turnId: request.turnId}}};
  }};
  return new Proxy(gateway, {get(target, property, receiver) {
    return property === "taskObservation" ? taskObservation : Reflect.get(target, property, receiver);
  }});
}
