// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import type { AiRun, AiTaskSpecDraft } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../core/services/aiAssistanceGateway";
import { panelItem } from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";
import { MockIndependentAgents } from "./MockIndependentAgents";
import { mockInternalConversations } from "./mockInternalConversations";

vi.mock("@research-assistant/literature-library-plugin/literatureGateway", async () => {
  const { panelItem } = await import("../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures");
  return {getLiteratureGateway: () => ({get: async (id: string) => ({...panelItem(), id})})};
});

it("reuses and reopens the same internal portrait history while isolating papers and requests", async () => {
  const storage = localStorage; storage.clear();
  const controls = {delayMs: 0, nextFailure: null, beforeOperation: async () => undefined,
    configure() {}, reset() {}, dispose() {}};
  let agents = new MockIndependentAgents(storage, controls);
  let status: AiRun["status"] = "canceled";
  const base = {startRun: vi.fn(async (draft: AiTaskSpecDraft): Promise<AiRun> => ({
    taskKind: draft.taskKind, userGoal: draft.userGoal, pipelineVersion: draft.pipelineVersion,
    promptVersion: draft.promptVersion, outputSchemaVersion: draft.outputSchemaVersion,
    maxInputTokens: draft.maxInputTokens, maxOutputTokens: draft.maxOutputTokens, timeoutMs: draft.timeoutMs,
    id: crypto.randomUUID(), status, providerId: "fixture-generation", modelId: "mock", scopeHash: "a".repeat(64),
    inputItemCount: 1, cancelRequested: false, createdAt: "now", updatedAt: "now", scopeItems: [], stages: [], inputTokens: 0, outputTokens: 0,
  })), listCandidates: async () => [], getRun: async () => {throw new Error("reloaded mock runner");}} as unknown as AiAssistanceGateway;
  const draft: AiTaskSpecDraft = {taskKind: "paper_profile_classification", providerId: "fixture-generation", userGoal: "portrait",
    pipelineVersion: "v1", promptVersion: "v1", outputSchemaVersion: 1, maxInputTokens: 100, maxOutputTokens: 100,
    timeoutMs: 1000, scopeItems: [{itemKind: "attachment", objectId: "pdf", parentObjectId: panelItem().id}]};
  let gateway = mockInternalConversations(base, agents, storage);
  try {
    const first = await gateway.startRun(draft);
    status = "needs_review";
    const second = await gateway.startRun(draft);
    expect(second.sharedConversationId).toBe(first.sharedConversationId);
    expect(second.sharedRequestId).not.toBe(first.sharedRequestId);
    agents.dispose(); agents = new MockIndependentAgents(storage, controls);
    gateway = mockInternalConversations(base, agents, storage);
    const latest = await gateway.getLatestPaperPortraitRun(panelItem().id);
    expect(latest?.id).toBe(second.id);
    const third = await gateway.startRun(draft);
    expect(third.sharedConversationId).toBe(first.sharedConversationId);
    const conversationId = first.sharedConversationId!;
    expect((await agents.metadata.read({id: conversationId})).internal).toBe(true);
    const history = await agents.gateway.sessions.readHistory({conversationId, afterSeq: 0, limit: 100});
    expect(history.events.filter(event => event.payload.type === "turnFinished")).toHaveLength(3);
    const requests = JSON.parse(storage.getItem("independent-submissions")!) as [string, unknown][];
    expect(new Set(requests.map(([id]) => id)).size).toBe(3);
    const other = await gateway.startRun({...draft, scopeItems: [{itemKind: "attachment", objectId: "other-pdf", parentObjectId: "other-paper"}]});
    expect(other.sharedConversationId).not.toBe(conversationId);
    expect((await gateway.getLatestPaperPortraitRun(panelItem().id))?.id).toBe(third.id);

    await agents.archives.setArchived({operationId: crypto.randomUUID(), id: conversationId, archived: true, expectedWatermark: (await agents.metadata.read({id: conversationId})).durableCount});
    expect((await gateway.getLatestPaperPortraitRun(panelItem().id))?.sharedConversationId).toBe(conversationId);
    expect(await agents.gateway.sessions.readHistory({conversationId, afterSeq: 0, limit: 100})).toEqual(history);
    expect((await agents.catalog.list({archived:false, internal:true, binding:null, before:null, limit:100})).conversations.map(row=>row.id))
      .toEqual([other.sharedConversationId]);
    agents.dispose(); agents = new MockIndependentAgents(storage, controls);
    gateway = mockInternalConversations(base, agents, storage);
    expect((await gateway.getLatestPaperPortraitRun(panelItem().id))?.sharedConversationId).toBe(conversationId);
    const reset = await gateway.startRun(draft);
    expect(reset.sharedConversationId).not.toBe(conversationId);
    expect(reset.sharedRequestId).not.toBe(third.sharedRequestId);
    expect((await agents.metadata.read({id:conversationId})).archived).toBe(true);
    expect(await agents.gateway.sessions.readHistory({conversationId, afterSeq:0, limit:100})).toEqual(history);
    expect((await gateway.getRun(third.id)).sharedConversationId).toBe(conversationId);
    await agents.archives.setArchived({operationId: crypto.randomUUID(), id: conversationId, archived: false, expectedWatermark: null});
    const subsequent=await gateway.startRun(draft);
    expect(subsequent.sharedConversationId).toBe(reset.sharedConversationId);
    expect((await gateway.getLatestPaperPortraitRun(panelItem().id))?.id).toBe(subsequent.id);
  } finally {agents.dispose(); storage.clear();}
});
