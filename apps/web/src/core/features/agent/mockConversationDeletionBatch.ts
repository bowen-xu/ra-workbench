import { decodeAgentDeletionBatchRequests, type AgentDeletionBatchResponse, type AgentDeletionRequest } from "@research-assistant/core/agent";

/** Each owner validates and prepares without mutation. All accepted members are
 * rechecked, then published synchronously before any caller can observe a partial batch. */
export interface MockConversationDeletionPlan {
  readonly validate: () => void;
  readonly publish: () => void;
  readonly rollback: () => void;
  readonly finish: () => void;
}

export async function mockConversationDeletionBatch(input: readonly AgentDeletionRequest[],
  prepare: (request: AgentDeletionRequest) => Promise<MockConversationDeletionPlan>): Promise<AgentDeletionBatchResponse> {
  const requests = decodeAgentDeletionBatchRequests(input);
  const planned = await Promise.all(requests.map(async request => {
    try { return {request, plan: await prepare(request)}; }
    catch (error: unknown) { return {request, error}; }
  }));
  const accepted = planned.filter((entry): entry is {request: AgentDeletionRequest; plan: MockConversationDeletionPlan} => "plan" in entry);
  for (const entry of accepted) entry.plan.validate();
  try { for (const entry of accepted) entry.plan.publish(); }
  catch (error: unknown) { for (const entry of accepted.reverse()) entry.plan.rollback(); throw error; }
  for (const entry of accepted) entry.plan.finish();
  return {schemaVersion: 1, results: planned.map(entry => "plan" in entry
    ? {requestId: entry.request.requestId, conversationId: entry.request.conversationId, status: "deleted", cleanupPending: false}
    : {requestId: entry.request.requestId, conversationId: entry.request.conversationId, status: "failed", error: entry.error})};
}
