import type {AgentContextGateway} from "../../services/agentContextGateway";
import type {NativeAgentPreview} from "../../runtime/nativeAgentPreview";

/** Inject one lost start after the native queue admission; all other ports stay unchanged. */
export function nativeContextQueueFault(gateway: AgentContextGateway, preview: NativeAgentPreview,
  take: () => boolean): AgentContextGateway {
  const start = gateway.guardedStart;
  if (!start) throw new Error("CONTEXT_START_UNAVAILABLE");
  const guardedStart = {...start, async start(request: Parameters<typeof start.start>[0], epoch: string) {
    if (!request.queue || !take()) return start.start(request, epoch);
    const response = await preview.invoke("agent_preview_admit_context_queue", {request: structuredClone(request), expectedEpoch: epoch});
    if (!response || typeof response !== "object" || !("schemaVersion" in response) || response.schemaVersion !== 1
      || !("admitted" in response) || response.admitted !== true) throw new Error("CONTEXT_PREVIEW_ADMISSION_INVALID");
    throw {code: "AGENT_PREVIEW_SYNTHETIC_IO_START_ADMITTED"};
  }};
  return new Proxy(gateway, {get(target, property, receiver) {
    return property === "guardedStart" ? guardedStart : Reflect.get(target, property, receiver);
  }});
}
