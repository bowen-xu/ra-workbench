import type {AgentContextGateway} from "../../services/agentContextGateway";
import type {NativeAgentPreview} from "../../runtime/nativeAgentPreview";

/** Run the real queued turn, then simulate an old database and a lost response. */
export function nativeContextLegacyQueueFault(gateway: AgentContextGateway, preview: NativeAgentPreview,
  take: () => false | "running" | "settled", finished: () => void = () => undefined): AgentContextGateway {
  const source = gateway.guardedStart;
  if (!source) throw new Error("CONTEXT_START_UNAVAILABLE");
  const guardedStart = {...source, async start(request: Parameters<typeof source.start>[0], epoch: string) {
    const mode = request.queue && take();
    if (!mode) return source.start(request, epoch);
    try {
      const response = await source.start(request, epoch);
      if (!response.runId) throw new Error("CONTEXT_PREVIEW_RUN_ID_UNAVAILABLE");
      const fixture = await preview.invoke(mode === "running" ? "agent_preview_forget_running_context_queue_admission" : "agent_preview_forget_settled_context_queue_admission", {
        request: structuredClone(request), expectedEpoch: epoch, runId: response.runId,
      });
      if (!fixture || typeof fixture !== "object" || !("schemaVersion" in fixture) || fixture.schemaVersion !== 1
        || !("prepared" in fixture) || fixture.prepared !== true) throw new Error("CONTEXT_PREVIEW_LEGACY_FIXTURE_INVALID");
      throw {code: "AGENT_PREVIEW_SYNTHETIC_LEGACY_START_RESPONSE_LOST"};
    } finally {finished();}
  }};
  return new Proxy(gateway, {get(target, property, receiver) {
    return property === "guardedStart" ? guardedStart : Reflect.get(target, property, receiver);
  }});
}
