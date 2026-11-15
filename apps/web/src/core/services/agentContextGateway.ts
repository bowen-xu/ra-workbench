import { listen } from "../runtime/platformEvents";
import { decodeAgentContextState, decodeAgentContextPreview, decodeAgentContextConversations, decodeAgentContextResources, decodeAgentStartResponse, decodeAgentSteerResponse, type AgentWritingSteerRequest, type AgentSteerResponse, type AgentContextPrepareRequest, type AgentContextReceipt, type AgentContextPreview, type AgentContextConversation, type AgentContextSelection, type AgentContextState, type AgentStartResponse } from "@research-assistant/core/agent";
import {createAgentContextQueueGateway, createAgentContextPrepareProvenGateway, createAgentContextPrepareStatusGateway, createAgentContextStartGateway, createAgentSteeringHandoffGateway, createAgentContextConfigurationSaveGateway, createAgentContextConfigurationSeedGateway,
  type AgentSessionGateway, type AgentContextQueueGateway, type AgentQueueEditInspection, type AgentContextPrepareProvenGateway,
  type AgentContextPrepareStatusGateway, type AgentContextStartGateway, type AgentTaskObservationGateway, type AgentSteeringHandoffGateway, type AgentContextConfigurationSaveGateway, type AgentContextConfigurationSeedGateway} from "@research-assistant/data/agent";
import type {AgentContextQueueEditRequest} from "@research-assistant/core/agent";
import { agentSessionGateway, agentTurnGateway, agentTaskObservationGateway } from "./agentGateway";
import { getAppRuntime, runtimeAgentGateway } from "../runtime/appRuntime";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import { NATIVE_PREVIEW_OWNER_CAPABILITY, resolveOrRecoverPreviewOwnerIntent } from "../runtime/nativePreviewOwnerIntent";
import type {contextDirectoryGateway} from "../features/agent/context/contextDirectoryGateway";
export interface AgentContextConfiguration { readonly revision: number; readonly state: AgentContextState }
export type AgentContextSteerRequest = Omit<AgentWritingSteerRequest, "manuscriptId">;
export interface AgentContextGateway {
    /** Stable owner identity survives wrappers that replace unrelated gateway ports. */
    readonly steeringOwner?: object;
    readonly directories?: Pick<typeof contextDirectoryGateway, "read" | "set">;
    subscribeConfiguration?(conversationId:string, changed:()=>void): Promise<()=>void>;
    readConfiguration?(conversationId: string, initializeInternal?: boolean): Promise<AgentContextConfiguration | null>;
    saveConfiguration?(conversationId: string, expectedRevision: number, state: AgentContextState): Promise<AgentContextConfiguration>;
    readonly configurationSave?: AgentContextConfigurationSaveGateway;
    readonly configurationSeed?: AgentContextConfigurationSeedGateway;
    readonly queue?: AgentContextQueueGateway & Partial<AgentQueueEditInspection<AgentContextQueueEditRequest>>;
    readonly preparation?: {readonly proven: AgentContextPrepareProvenGateway; readonly status: AgentContextPrepareStatusGateway};
    readonly guardedStart?: AgentContextStartGateway;
    readonly taskObservation?: AgentTaskObservationGateway;
    readonly steeringHandoff?: AgentSteeringHandoffGateway;
    steer?(request: AgentContextSteerRequest): Promise<AgentSteerResponse>;
    recoverQueue?(conversationId: string): Promise<AgentContextPreview | null>;
    readonly sessions: AgentSessionGateway;
    list(): Promise<readonly AgentContextConversation[]>;
    search(query: string): Promise<readonly AgentContextSelection[]>;
    catalog?(query: string): Promise<readonly AgentContextSelection[]>;
    /** Resolve one visible resource without enumerating the context catalog.
     * A suggestion is not an access grant or a prepared submission. */
    recommend?(candidate:AgentContextSelection, signal?:AbortSignal):Promise<AgentContextSelection>;
    prepare(request: AgentContextPrepareRequest): Promise<AgentContextPreview>;
    restore(receipt: AgentContextReceipt): Promise<AgentContextPreview>;
    start(preview: AgentContextPreview, options?: {readonly resumeQueue: boolean}): Promise<AgentStartResponse>;
    interrupt(conversationId: string, turnId: string): Promise<void>;
}
async function call(command: string, args: Record<string, unknown>, options?: {readonly signal?: AbortSignal}): Promise<unknown> {
    const signal = command === "agent_recommend_context" ? options?.signal : undefined;
    signal?.throwIfAborted();
    const runtime = getAppRuntime();
    if (runtime.mode === "preview") {
        if (!runtime.invoke)
            throw new Error("预览缺少上下文适配器");
        const previewOwnerIntent = runtime.nativeAgentPreview?.bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY)
            ? await resolveOrRecoverPreviewOwnerIntent(runtime.nativeAgentPreview, runtime.storage, command, args) : undefined;
        return previewOwnerIntent ? runtime.invoke(command, args, {previewOwnerIntent, ...(signal ? {signal} : {})})
            : signal ? runtime.invoke(command, args, {signal}) : runtime.invoke(command, args);
    }
    if (!isDesktopRuntime())
        throw new Error("独立对话需要桌面应用");
    return invoke(command, args);
}
function check(preview: AgentContextPreview, receipt: AgentContextReceipt): AgentContextPreview {
    if (preview.conversationId !== receipt.conversationId.toLowerCase() || preview.requestId !== receipt.requestId.toLowerCase())
        throw new Error("冻结提交身份不匹配");
    return preview;
}
export const agentContextGateway: AgentContextGateway = runtimeAgentGateway("agentContextGateway", {
    steeringOwner: {},
    subscribeConfiguration: async (_conversationId:string,changed:()=>void) => listen("research-assistant:structured-resource-changed",changed),
    readConfiguration: async (conversationId: string, initializeInternal = true) => { const value = await call("agent_read_context_configuration", {conversationId,initializeInternal}); return value === null ? null : configuration(value); },
    saveConfiguration: async (conversationId: string, expectedRevision: number, state: AgentContextState) => configuration(await call("agent_save_context_configuration", {request:{conversationId,expectedRevision,state:decodeAgentContextState(state)}})),
    configurationSave: createAgentContextConfigurationSaveGateway(call),
    configurationSeed: createAgentContextConfigurationSeedGateway(call),
    queue: createAgentContextQueueGateway(call),
    preparation: {proven: createAgentContextPrepareProvenGateway(call), status: createAgentContextPrepareStatusGateway(call)},
    guardedStart: createAgentContextStartGateway(call),
    taskObservation: agentTaskObservationGateway,
    steeringHandoff: createAgentSteeringHandoffGateway(call),
    steer: async request => {
        const response = decodeAgentSteerResponse(await call("agent_steer_context_turn", {request}));
        if (response.conversationId !== request.conversationId.toLowerCase() || response.turnId !== request.turnId.toLowerCase() || response.inputId !== request.inputId.toLowerCase()) throw new Error("插话回执身份不匹配");
        return response;
    },
    recoverQueue: async (conversationId: string) => {
        const result = await call("agent_read_pending_context_queue_preview", {request:{conversationId}});
        if (result === null) return null;
        const preview = decodeAgentContextPreview(result);
        if (preview.conversationId !== conversationId || !preview.queue) throw new Error("队列恢复身份不匹配");
        return preview;
    },
    sessions: agentSessionGateway,
    list: async () => decodeAgentContextConversations(await call("agent_list_context_conversations", {})),
    search: async (query: string) => decodeAgentContextResources(await call("agent_search_context_resources", { query })),
    catalog: async (query: string) => decodeAgentContextResources(await call("agent_search_context_catalog", { query })),
    recommend: async (candidate: AgentContextSelection, signal?: AbortSignal) => {
        const [result] = decodeAgentContextResources([await call("agent_recommend_context", {candidate}, signal ? {signal} : undefined)]);
        signal?.throwIfAborted();
        const kinds = candidate.resource.kind === "note" ? ["note", "noteGroup"] : candidate.resource.kind === "paper" ? ["paperGroup"] : ["manuscript"];
        if (!result || !kinds.includes(result.resource.kind) || result.resource.resourceId !== candidate.resource.resourceId)
            throw new Error("推荐资源身份不匹配");
        return result;
    },
    prepare: async (request: AgentContextPrepareRequest) => check(decodeAgentContextPreview(await call("agent_prepare_context_submission", { request })), request),
    restore: async (request: AgentContextReceipt) => check(decodeAgentContextPreview(await call("agent_restore_context_submission", { request })), request),
    start: async (preview: AgentContextPreview, options?: {readonly resumeQueue: boolean}) => {
        const response = decodeAgentStartResponse(await call("agent_start_context_submission", { request: { conversationId: preview.conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId, ...(preview.queue ? {queue:preview.queue} : {}), ...(options?.resumeQueue ? {resumeQueue:true} : {}) } }));
        if (response.conversationId !== preview.conversationId || response.requestId !== preview.requestId || response.turnId !== preview.turnId)
            throw new Error("启动回执身份不匹配");
        return response;
    },
    interrupt: async (conversationId: string, turnId: string) => { await agentTurnGateway.interrupt({ conversationId, turnId }); },
});
function configuration(value: unknown): AgentContextConfiguration {
    if (!value || typeof value !== "object" || !("revision" in value) || !("state" in value) || typeof value.revision !== "number" || !Number.isSafeInteger(value.revision) || value.revision < 1) throw new Error("上下文配置回执无效");
    return {revision:value.revision,state:decodeAgentContextState(value.state)};
}
