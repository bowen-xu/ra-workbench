import {expect, it, vi} from "vitest";
import {decodeAgentPaperPrepareStatusResponse, decodeAgentPaperStartStatusResponse, decodeAgentQueueEditStatusResponse,
  type AgentPaperQueueResponse} from "@research-assistant/core/agent";
import {MockPaperReceipts} from "./mockPaperReceipts";
import {MockResearchAgents} from "./MockResearchAgents";
import {WritingDiffDemoHistory} from "../../previews/agent/WritingDiffDemoHistory";
import {WritingDiffDemoQueue} from "../../previews/agent/WritingDiffDemoQueue";
import {createMockResearchQueues} from "./mockResearchQueues";

const id = (n: number) => `abcdef00-1234-5678-abcd-${String(n).padStart(12,"0")}`;
function storage(): Storage {
  const rows = new Map<string, string>(); return {get length() {return rows.size;}, key: n => [...rows.keys()][n] ?? null,
    getItem: key => rows.get(key) ?? null, setItem: (key, value) => {rows.set(key, value);}, removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
}
it("returns a first-write proof and preserves the same proof and result on replay", async () => {
  const data = storage(), hash = "a".repeat(64), snapshot = `paper-context-${hash}`;
  data.setItem("paper-contexts", JSON.stringify([[snapshot, {schemaVersion: 1, paperId: "paper", snapshotId: snapshot, contextHash: hash, attachments: [{attachmentId: "attachment", contentSha256: hash, parseRunId: "parse", ordinal: 0}], items: []}]]));
  const request = {conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: snapshot, expectedContextHash: hash, text: "Question"};
  const firstOwner = new MockResearchAgents(data, {} as never, {} as never);
  const first = await firstOwner.gateways().agentPaperPrepareProvenGateway!.prepare(request);
  const reopened = new MockResearchAgents(data, {} as never, {} as never);
  const replay = await reopened.gateways().agentPaperPrepareProvenGateway!.prepare(request);
  expect(first.proof.replayed).toBe(false); expect(replay.proof.replayed).toBe(true);
  expect(replay.proof.epoch).toBe(first.proof.epoch); expect(replay.proof.sqlFence).toBe(first.proof.sqlFence);
  expect(replay.result).toEqual(first.result);
  await expect(reopened.gateways().agentPaperPrepareProvenGateway!.prepare({...request, text: "Changed"})).rejects.toThrow();
  const guarded = {...request, requestId: id(7)}, epoch = first.proof.epoch;
  const gateway = reopened.gateways().agentPaperPrepareProvenGateway!;
  await expect(gateway.prepare(guarded, {expectedEpoch: id(8)})).rejects.toThrow("data_epoch_changed");
  expect((await gateway.prepare(guarded, {expectedEpoch: epoch})).proof.replayed).toBe(false);
  expect((await gateway.prepare(guarded, {expectedEpoch: epoch})).proof.replayed).toBe(true);
  await expect(gateway.prepare(guarded)).rejects.toThrow("身份冲突");
  await expect(reopened.gateways().agentPaperPrepareStatusGateway!.read(guarded)).rejects.toThrow("身份冲突");
});
it("retains mock evidence with the same original identities and environment across reopening", () => {
  const data = storage(), first = new MockPaperReceipts(data);
  const request = {conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", expectedContextHash: "a".repeat(64), text: "Question"};
  const preview = {schemaVersion: 1 as const, conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", turnId: id(3), contextSnapshotId: id(4),
    input: {text: request.text, model: {providerId: "deepseek", modelId: "fixture", revision: null}, reasoningEffort: null, context: []}};
  expect(decodeAgentPaperPrepareStatusResponse(first.preparation(request)).status.type).toBe("unconfirmed");
  first.prepared(request, preview); first.started(preview, {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(3), alreadyStarted: false, outcome: null});
  const removal = {conversationId: id(1), paperId: "paper", requestId: id(5), expectedRevision: 0, edit: {type: "remove" as const, inputId: id(6)}};
  const queue: AgentPaperQueueResponse = {schemaVersion: 1, conversationId: id(1), paperId: "paper", revision: 1, paused: false, preparingInputId: null, inputs: []};
  first.queueEdit(removal, queue); const reopened = new MockPaperReceipts(data);
  const prepared = decodeAgentPaperPrepareStatusResponse(reopened.preparation(request));
  const removed = decodeAgentQueueEditStatusResponse(reopened.queueStatus(removal), removal);
  const start = decodeAgentPaperStartStatusResponse(reopened.start({conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", contextSnapshotId: id(4), model: preview.input.model, mode: "chatGptDefault"}));
  expect(prepared.status.type).toBe("committed"); expect(removed.status.type).toBe("committed"); expect(start.status.type).toBe("started");
  if (prepared.status.type === "committed" && removed.status.type === "committed") expect(removed.status.proof.epoch).toBe(prepared.status.proof.epoch);
  expect(() => reopened.preparation({...request, text: "Changed"})).toThrow();
  expect(() => reopened.queueStatus({...removal, edit: {type: "remove", inputId: id(9)}})).toThrow();
});
it("does not report a committed mock receipt when its storage write fails", () => {
  const data = storage(), saved = data.setItem.bind(data), facts = new MockPaperReceipts(data);
  data.setItem = (key, value) => {if (key === "mock-paper-receipts") throw new Error("storage unavailable"); saved(key, value);};
  const request = {conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", expectedContextHash: "a".repeat(64), text: "Question"};
  const preview = {schemaVersion: 1 as const, conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", turnId: id(3), contextSnapshotId: id(4),
    input: {text: request.text, model: {providerId: "deepseek", modelId: "fixture", revision: null}, reasoningEffort: null, context: []}};
  expect(() => facts.prepared(request, preview)).toThrow("storage unavailable");
  expect(decodeAgentPaperPrepareStatusResponse(facts.preparation(request)).status.type).toBe("unconfirmed");
});
it("reports mock start status from matching history, including terminal outcome", async () => {
  const data = storage(), history = new WritingDiffDemoHistory(), agents = {backend: () => ({agent: {history}})};
  const research = new MockResearchAgents(data, agents as never, {} as never);
  const request = {conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", expectedContextHash: "a".repeat(64), text: "Question"};
  const preview = {schemaVersion: 1 as const, conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", turnId: id(3), contextSnapshotId: id(4),
    input: {text: request.text, model: {providerId: "deepseek", modelId: "fixture", revision: null}, reasoningEffort: null, context: []}};
  research.paperReceipts.prepared(request, preview); research.paperReceipts.started(preview, {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(3), alreadyStarted: false, outcome: null});
  const start = {conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", contextSnapshotId: id(4), model: preview.input.model, mode: "chatGptDefault" as const};
  const gateway = research.gateways().agentPaperStartStatusGateway!;
  expect((await gateway.read(start)).status.type).toBe("unconfirmed");
  const admission = {scope: {manuscriptId: "paper", conversationId: id(1), requestId: id(2), contextSnapshotId: id(4)}, turnId: id(3)};
  history.start(admission, request.text, preview.input.model);
  expect((await gateway.read(start)).status).toMatchObject({type: "started", result: {outcome: null}});
  history.complete(admission, [], true);
  expect((await gateway.read(start)).status).toMatchObject({type: "started", result: {outcome: "completed"}});
});
it("rejects a changed mock environment before admitting a paper turn", async () => {
  const data = storage(), backend = vi.fn(), research = new MockResearchAgents(data, {backend} as never, {beforeOperation: async () => {}} as never);
  const preview = {schemaVersion: 1 as const, conversationId: id(1), requestId: id(2), paperId: "paper", paperContextSnapshotId: "snapshot", turnId: id(3), contextSnapshotId: id(4),
    input: {text: "Question", model: {providerId: "deepseek", modelId: "fixture", revision: null}, reasoningEffort: null, context: []}};
  await expect(research.gateways().agentPaperSubmissionGateway!.start(preview, "chatGptDefault", {expectedEpoch: id(9)})).rejects.toThrow("data_epoch_changed");
  expect(backend).not.toHaveBeenCalled();
});
it("keeps a non-UUID paper queue identity while the reused mock queue uses its required UUID scope", async () => {
  const data = storage(), facts = new MockPaperReceipts(data), queue = new WritingDiffDemoQueue({} as never);
  const agents = {backend: () => ({runtime: {queue}})}, gateway = createMockResearchQueues(agents as never, {}, facts).agentPaperQueueGateway;
  const scope = {conversationId: id(1), paperId: "paper-1"}, inputId = id(2);
  expect((await gateway.read(scope)).paperId).toBe("paper-1");
  const request = {...scope, requestId: id(3), expectedRevision: 0, edit: {type: "add" as const, inputId, text: "Question"}};
  expect((await gateway.edit(request)).inputs).toMatchObject([{inputId, text: "Question", origin: {kind: "paper", resourceId: "paper-1"}}]);
  expect((await gateway.read(scope)).inputs).toMatchObject([{inputId, text: "Question", origin: {kind: "paper", resourceId: "paper-1"}}]);
  const observed = decodeAgentQueueEditStatusResponse(await gateway.inspectEdit!(request), request);
  expect(observed.status).toMatchObject({type: "committed", proof: {epoch: facts.epoch}});
  const removal = {...scope, requestId: id(4), expectedRevision: 1, expectedEpoch: id(5), edit: {type: "remove" as const, inputId}};
  await expect(gateway.edit(removal)).rejects.toThrow("data_epoch_changed");
  expect((await gateway.read(scope)).inputs).toHaveLength(1);
  const valid = {...removal, expectedEpoch: facts.epoch};
  expect((await gateway.edit(valid)).inputs).toEqual([]);
  expect(decodeAgentQueueEditStatusResponse(await gateway.inspectEdit!(valid), valid).status.type).toBe("committed");
  expect(new MockPaperReceipts(data).queueScopeId("paper-1")).toBe(facts.queueScopeId("paper-1"));
});
