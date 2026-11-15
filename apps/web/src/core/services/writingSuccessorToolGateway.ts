import { decodeAgentWorkspaceConfirmation, type AgentWorkspaceConfirmation } from "@research-assistant/core/agent";
import type { AgentWorkspaceSuccessorIdentity } from "@research-assistant/data/agent";
import { invoke } from "../runtime/platformCore";
import { listen } from "../runtime/platformEvents";

export interface WritingSuccessorToolRequest extends AgentWorkspaceSuccessorIdentity {
  readonly requestId: string;
}
export interface WritingSuccessorToolGateway {
  listen(receive: (request: WritingSuccessorToolRequest) => void): Promise<() => void>;
  report(result: { request: WritingSuccessorToolRequest; confirmation: AgentWorkspaceConfirmation | null }): Promise<void>;
}

export const writingSuccessorToolGateway: WritingSuccessorToolGateway = {
  listen: receive => listen<WritingSuccessorToolRequest>("research-writing-successor-workspace", event => {
    try {
      const request = event.payload;
      if (!Number.isInteger(request.generation) || request.generation < 1 || request.generation > 2_147_483_647) return;
      for (const leaseId of [request.requestId, request.leaseId, request.predecessorDeliveryId]) {
        decodeAgentWorkspaceConfirmation({ scope: request.scope, turnId: request.turnId, leaseId, manifestHash: "0".repeat(64) });
      }
      receive(structuredClone(request));
    } catch { /* Invalid events cannot authorize a workspace. */ }
  }),
  report: result => invoke("agent_report_writing_successor_workspace", { result }),
};
