import { runtimeAgentGateway, resolveAppGateway, getAppRuntime } from "../runtime/appRuntime";
import { listen } from "../runtime/platformEvents";
import { hasPlatformCapabilities } from "../runtime/appRuntime";
import type { AgentCatalogGateway, AgentSubtaskGateway } from "@research-assistant/data/agent";
import {createAgentNoteSteeringGateway} from "@research-assistant/data/agent";
import {createAgentPaperContextGateway} from "@research-assistant/data/agent";
import {createAgentPaperReadingHistoryMigrationGateway} from "@research-assistant/data/agent";
import {createAgentNoteQueueGateway} from "@research-assistant/data/agent";
import {createAgentReadingNoteSubmissionGateway} from "@research-assistant/data/agent";
import {createAgentPaperSubmissionGateway, createAgentPaperPrepareProvenGateway, createAgentPaperPrepareStatusGateway, createAgentPaperStartStatusGateway} from "@research-assistant/data/agent";
import {createAgentPaperRecoveryGateway} from "@research-assistant/data/agent";
import {createAgentPaperQueueGateway} from "@research-assistant/data/agent";
import {createAgentReadingNoteRecoveryGateway} from "@research-assistant/data/agent";
import {createAgentNoteRejectionGateway} from "@research-assistant/data/agent";
import {createAgentNoteApplicationGateway} from "@research-assistant/data/agent";
import {createAgentNoteUndoGateway} from "@research-assistant/data/agent";
import {createAgentReadingNoteContextGateway} from "@research-assistant/data/agent";
import { createAgentProposalGateway, createAgentInstructionTrustGateway, createAgentWorkspaceGateway, createAgentWorkspaceSuccessorGateway, createAgentWorkspaceReviewGateway } from "@research-assistant/data/agent";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import { createAgentForkGateway, createAgentForkOriginGateway } from "@research-assistant/data/agent";
import { createAgentDeletionBatchGateway, createAgentDeletionGateway } from "@research-assistant/data/agent";
import { createAgentWorkspaceDiscardGateway } from "@research-assistant/data/agent";
import { createAgentNoteContextGateway, createAgentNoteSubmissionGateway, createAgentNoteRecoveryGateway, createAgentNoteReviewGateway } from "@research-assistant/data/agent";
import { createAgentArchiveGateway, createAgentConversationExportGateway, createAgentMetadataGateway, createAgentCreationGateway, createAgentSearchGateway } from "@research-assistant/data/agent";
import { createAgentApplicationGateway, createAgentEditorRestorationGateway, createAgentUndoEditorRestorationGateway, createAgentUndoGateway } from "@research-assistant/data/agent";
import { createAgentQueueAdmissionsGateway, createAgentInputQueueGateway, createAgentInputQueuePreviewGateway, createAgentCompileResultGateway, createAgentSubtaskGateway } from "@research-assistant/data/agent";
import { createAgentChildSpawnGateway } from "@research-assistant/data/agent";
import { createAgentChildCreationStatusGateway } from "@research-assistant/data/agent";
import { createAgentTaskObservationGateway } from "@research-assistant/data/agent";
import { createAgentCatalogGateway, createAgentSessionGateway, createAgentTurnGateway, createAgentSubmissionGateway, createAgentSteeringGateway, createAgentSteeringRecoveryGateway } from "@research-assistant/data/agent";
import { createAgentSkillCatalogGateway } from "@research-assistant/data/agent";
import { decodeAgentHostResources, type AgentResourceScope } from "@research-assistant/core/agent";
import { createAgentWritingFileDeliveryGateway } from "@research-assistant/data/agent";
import { createAgentPendingWritingUndoGateway } from "@research-assistant/data/agent";
import { createAgentPendingWritingApplicationGateway } from "@research-assistant/data/agent";
import { createAgentCompactionGateway } from "@research-assistant/data/agent";
import { invokeDesktopHistoryWait } from "./desktopHistoryWait";
import { NATIVE_PREVIEW_OWNER_CAPABILITY, resolveOrRecoverPreviewOwnerIntent } from "../runtime/nativePreviewOwnerIntent";
import type {AgentMcpServerSummary} from "../contracts/agentExtensions";
export type {AgentMcpServerSummary} from "../contracts/agentExtensions";

async function invokeAgent(command: string, args: Record<string, unknown>, options?: { signal?: AbortSignal }): Promise<unknown> {
  const runtime = getAppRuntime();
  if (runtime.mode === "preview") {
    if (!runtime.invoke) throw new Error(`预览未覆盖平台调用：${command}`);
    const previewOwnerIntent = runtime.nativeAgentPreview?.bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY)
      ? await resolveOrRecoverPreviewOwnerIntent(runtime.nativeAgentPreview, runtime.storage, command, args) : undefined;
    return previewOwnerIntent ? runtime.invoke(command, args, {...options, previewOwnerIntent})
      : options ? runtime.invoke(command, args, options) : runtime.invoke(command, args);
  }
  if (!isDesktopRuntime()) throw new Error("共享 Agent 会话需要桌面应用");
  if (command === "agent_wait_for_history" && options?.signal) return invokeDesktopHistoryWait(invoke, args, options.signal);
  return invoke(command, args);
}

/** Native-only MCP approval boundary; secrets and process configuration stay native. */
export const agentMcpGateway = runtimeAgentGateway("agentMcpGateway", {
  list: () => invokeAgent("agent_list_mcp_servers", {}) as Promise<AgentMcpServerSummary[]>,
  install: (approvedServerIds: readonly string[]) =>
    invokeAgent("agent_install_mcp_runtime", { request: { approvedServerIds } }),
  revoke: (serverId: string) => invokeAgent("agent_revoke_mcp_server", { serverId }),
});
export const agentSkillCatalogGateway = runtimeAgentGateway("agentSkillCatalogGateway", createAgentSkillCatalogGateway(invokeAgent));
export async function agentHostResourceGateway(scope: AgentResourceScope) {
  return decodeAgentHostResources(await invokeAgent("agent_list_host_resources", {request: scope}));
}
export const agentCatalogGateway: AgentCatalogGateway = runtimeAgentGateway("agentCatalogGateway", {
  ...createAgentCatalogGateway(invokeAgent),
  subscribeActivity: async (changed: () => void) => hasPlatformCapabilities()
    ? listen("research-assistant:agent-activity-changed", () => changed()) : () => {},
});
export const agentForkGateway = runtimeAgentGateway("agentForkGateway", createAgentForkGateway(invokeAgent));
export const agentForkOriginGateway = runtimeAgentGateway("agentForkOriginGateway", createAgentForkOriginGateway(invokeAgent));
export const agentDeletionGateway = runtimeAgentGateway("agentDeletionGateway", createAgentDeletionGateway(invokeAgent));
export const agentDeletionBatchGateway = runtimeAgentGateway("agentDeletionBatchGateway", createAgentDeletionBatchGateway(invokeAgent));
export const agentWorkspaceDiscardGateway = runtimeAgentGateway("agentWorkspaceDiscardGateway", createAgentWorkspaceDiscardGateway(invokeAgent));
export const agentNoteContextGateway = runtimeAgentGateway("agentNoteContextGateway", createAgentNoteContextGateway(invokeAgent));
export const agentPaperContextGateway = runtimeAgentGateway("agentPaperContextGateway", createAgentPaperContextGateway(invokeAgent));
export const agentPaperReadingHistoryMigrationGateway = runtimeAgentGateway("agentPaperReadingHistoryMigrationGateway", createAgentPaperReadingHistoryMigrationGateway(invokeAgent));
export const agentNoteSubmissionGateway = runtimeAgentGateway("agentNoteSubmissionGateway", createAgentNoteSubmissionGateway(invokeAgent));
export const agentNoteRecoveryGateway = runtimeAgentGateway("agentNoteRecoveryGateway", createAgentNoteRecoveryGateway(invokeAgent));
export const agentNoteReviewGateway = runtimeAgentGateway("agentNoteReviewGateway", createAgentNoteReviewGateway(invokeAgent));
export const agentSearchGateway = runtimeAgentGateway("agentSearchGateway", createAgentSearchGateway(invokeAgent));
export const agentCreationGateway = runtimeAgentGateway("agentCreationGateway", createAgentCreationGateway(invokeAgent));
export const agentMetadataGateway = runtimeAgentGateway("agentMetadataGateway", createAgentMetadataGateway(invokeAgent));
export const agentArchiveGateway = runtimeAgentGateway("agentArchiveGateway", createAgentArchiveGateway(invokeAgent));
export const agentConversationExportGateway = runtimeAgentGateway("agentConversationExportGateway", createAgentConversationExportGateway(invokeAgent));
export const agentSessionGateway = runtimeAgentGateway("agentSessionGateway", createAgentSessionGateway(invokeAgent));
export const agentTurnGateway = runtimeAgentGateway("agentTurnGateway", createAgentTurnGateway(invokeAgent));
export const agentTaskObservationGateway = runtimeAgentGateway("agentTaskObservationGateway", createAgentTaskObservationGateway(invokeAgent));
export const agentSubmissionGateway = runtimeAgentGateway("agentSubmissionGateway", createAgentSubmissionGateway(invokeAgent));
export const agentCompactionGateway = runtimeAgentGateway("agentCompactionGateway", createAgentCompactionGateway(invokeAgent));
export const agentSteeringGateway = runtimeAgentGateway("agentSteeringGateway", createAgentSteeringGateway(invokeAgent));
export const agentSteeringRecoveryGateway = runtimeAgentGateway("agentSteeringRecoveryGateway", createAgentSteeringRecoveryGateway(invokeAgent));
export const agentInputQueueGateway = runtimeAgentGateway("agentInputQueueGateway", createAgentInputQueueGateway(invokeAgent));
export const agentInputQueuePreviewGateway = runtimeAgentGateway("agentInputQueuePreviewGateway", createAgentInputQueuePreviewGateway(invokeAgent));
export const agentCompileResultGateway = runtimeAgentGateway("agentCompileResultGateway", createAgentCompileResultGateway(invokeAgent));
export const agentSubtaskGateway: AgentSubtaskGateway = runtimeAgentGateway("agentSubtaskGateway", {
  ...createAgentSubtaskGateway(invokeAgent),
  subscribe: async (parentConversationId: string, changed: () => void) => hasPlatformCapabilities()
    ? listen<{parentConversationId?: string}>("research-assistant:agent-subtasks-changed", event => {
      if (!event.payload.parentConversationId || event.payload.parentConversationId === parentConversationId) changed();
    }) : () => {},
});
export const agentChildSpawnGateway = runtimeAgentGateway("agentChildSpawnGateway", createAgentChildSpawnGateway(invokeAgent));
export const agentChildCreationStatusGateway = runtimeAgentGateway("agentChildCreationStatusGateway", createAgentChildCreationStatusGateway(invokeAgent));

export const agentQueueAdmissionsGateway = runtimeAgentGateway("agentQueueAdmissionsGateway", createAgentQueueAdmissionsGateway(invokeAgent));
export const agentWritingFileDeliveryGateway = runtimeAgentGateway("agentWritingFileDeliveryGateway", createAgentWritingFileDeliveryGateway(invokeAgent));

export const agentInstructionTrustGateway = runtimeAgentGateway("agentInstructionTrustGateway", createAgentInstructionTrustGateway(invokeAgent));
export const agentWorkspaceGateway = runtimeAgentGateway("agentWorkspaceGateway", createAgentWorkspaceGateway(invokeAgent));
export const agentWorkspaceSuccessorGateway = runtimeAgentGateway("agentWorkspaceSuccessorGateway", createAgentWorkspaceSuccessorGateway(invokeAgent));
export const agentWorkspaceReviewGateway = runtimeAgentGateway("agentWorkspaceReviewGateway", createAgentWorkspaceReviewGateway(invokeAgent));
export const agentProposalGateway = runtimeAgentGateway("agentProposalGateway", createAgentProposalGateway(invokeAgent));
export const agentApplicationGateway = runtimeAgentGateway("agentApplicationGateway", createAgentApplicationGateway(invokeAgent));
export const agentEditorRestorationGateway = runtimeAgentGateway("agentEditorRestorationGateway", createAgentEditorRestorationGateway(invokeAgent));

export const agentUndoGateway = runtimeAgentGateway("agentUndoGateway", createAgentUndoGateway(invokeAgent));
export const agentPendingWritingUndoGateway = runtimeAgentGateway("agentPendingWritingUndoGateway", createAgentPendingWritingUndoGateway(invokeAgent));
export const agentPendingWritingApplicationGateway = runtimeAgentGateway("agentPendingWritingApplicationGateway", createAgentPendingWritingApplicationGateway(invokeAgent));

export const getAgentUndoEditorRestorationGateway = (scope: import("@research-assistant/core/agent").AgentApplicationScope): ReturnType<typeof createAgentUndoEditorRestorationGateway> => resolveAppGateway("getAgentUndoEditorRestorationGateway", (value: typeof scope) => createAgentUndoEditorRestorationGateway(invokeAgent, value))(scope);

export const agentNoteRejectionGateway = runtimeAgentGateway("agentNoteRejectionGateway", createAgentNoteRejectionGateway(invokeAgent));
export const agentNoteApplicationGateway = runtimeAgentGateway("agentNoteApplicationGateway", createAgentNoteApplicationGateway(invokeAgent));
export const agentNoteUndoGateway = runtimeAgentGateway("agentNoteUndoGateway", createAgentNoteUndoGateway(invokeAgent));
export const agentReadingNoteContextGateway = runtimeAgentGateway("agentReadingNoteContextGateway", createAgentReadingNoteContextGateway(invokeAgent));
export const agentReadingNoteSubmissionGateway = runtimeAgentGateway("agentReadingNoteSubmissionGateway", createAgentReadingNoteSubmissionGateway(invokeAgent));
export const agentPaperSubmissionGateway = runtimeAgentGateway("agentPaperSubmissionGateway", createAgentPaperSubmissionGateway(invokeAgent));
export const agentPaperPrepareProvenGateway = runtimeAgentGateway("agentPaperPrepareProvenGateway", createAgentPaperPrepareProvenGateway(invokeAgent));
export const agentPaperPrepareStatusGateway = runtimeAgentGateway("agentPaperPrepareStatusGateway", createAgentPaperPrepareStatusGateway(invokeAgent));
export const agentPaperStartStatusGateway = runtimeAgentGateway("agentPaperStartStatusGateway", createAgentPaperStartStatusGateway(invokeAgent));
export const agentPaperRecoveryGateway = runtimeAgentGateway("agentPaperRecoveryGateway", createAgentPaperRecoveryGateway(invokeAgent));
export const agentPaperQueueGateway = runtimeAgentGateway("agentPaperQueueGateway", createAgentPaperQueueGateway(invokeAgent));
export const agentReadingNoteRecoveryGateway = runtimeAgentGateway("agentReadingNoteRecoveryGateway", createAgentReadingNoteRecoveryGateway(invokeAgent));
export const agentNoteQueueGateway = runtimeAgentGateway("agentNoteQueueGateway", createAgentNoteQueueGateway(invokeAgent));

export const agentNoteSteeringGateway=createAgentNoteSteeringGateway(invokeAgent);
