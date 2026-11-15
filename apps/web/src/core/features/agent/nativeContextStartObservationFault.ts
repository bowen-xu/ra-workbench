import type {AgentContextGateway} from "../../services/agentContextGateway";

/** Lose one real start response, then simulate an unavailable original owner. */
export function nativeContextStartObservationFault(gateway: AgentContextGateway, enabled: () => boolean): AgentContextGateway {
  const source = gateway.guardedStart;
  if (!source) return gateway;
  let bound: string | undefined;
  const guardedStart = {...source,
    async start(request: Parameters<typeof source.start>[0], epoch: string) {
      const response = await source.start(request, epoch);
      if (enabled() && bound === undefined) {
        bound = JSON.stringify([request, epoch]);
        throw {code:"AGENT_PREVIEW_SYNTHETIC_START_RESPONSE_LOST"};
      }
      return response;
    },
    async read(request: Parameters<typeof source.read>[0], epoch: string) {
      const response = await source.read(request, epoch);
      if (!enabled()) {bound = undefined; return response;}
      if (bound !== JSON.stringify([request, epoch]) || response.status.type !== "started") return response;
      const {runId, ...result} = response.status.result; void runId;
      return {...response, status:{type:"startedUnobserved" as const, result}};
    },
  };
  return new Proxy(gateway, {get(target, property, receiver) {
    return property === "guardedStart" ? guardedStart : Reflect.get(target, property, receiver);
  }});
}
