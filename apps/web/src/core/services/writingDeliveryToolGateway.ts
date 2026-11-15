import { decodeAgentWritingFileDeliveryAdmission, decodeAgentWorkspaceConfirmation, type AgentWorkspaceConfirmation } from "@research-assistant/core/agent";
import type { WritingSubmissionAdmission } from "../public/agentWriting";
import { invoke } from "../runtime/platformCore";
import { listen } from "../runtime/platformEvents";

export interface WritingDeliveryToolRequest extends WritingSubmissionAdmission { readonly requestId: string; readonly binding: AgentWorkspaceConfirmation }
export interface WritingDeliveryToolResult {
  readonly request: WritingDeliveryToolRequest;
  readonly complete: boolean;
  readonly message: string;
}
export interface WritingDeliveryToolGateway {
  listen(receive: (request: WritingDeliveryToolRequest) => void): Promise<() => void>;
  report(result: WritingDeliveryToolResult): Promise<void>;
}
export const writingDeliveryToolGateway: WritingDeliveryToolGateway = {
  listen: receive => listen<WritingDeliveryToolRequest>("research-writing-file-delivery", event => {
    const { requestId, scope, turnId } = event.payload;
    if (typeof requestId !== "string" || !/^[0-9a-f-]{36}$/u.test(requestId)) return;
    const admission = decodeAgentWritingFileDeliveryAdmission({ scope, turnId });
    const binding = decodeAgentWorkspaceConfirmation(event.payload.binding);
    if (binding.turnId !== admission.turnId || (Object.keys(admission.scope) as (keyof typeof admission.scope)[])
      .some(key => binding.scope[key] !== admission.scope[key])) return;
    receive({ ...admission, requestId, binding });
  }),
  report: result => invoke("agent_report_writing_file_delivery", { result }),
};
