import {expect, it, vi} from "vitest";
import type {AgentPaperPrepareStatusGateway, AgentPaperSubmissionGateway, AgentPaperStartStatusGateway} from "@research-assistant/data/agent";
import {AgentInputQueueController} from "@research-assistant/data/agent";
import {agentQueueEditIntentFromRequest, type AgentPaperQueueEditRequest, type AgentPaperQueueResponse} from "@research-assistant/core/agent";
import {registerStoragePersistence} from "../../../../../../apps/web/src/core/runtime/storagePersistence";
import {PaperPreparation} from "./paperPreparation";
import {ConversationDraftStore, conversationDraftKey} from "../../../../../../apps/web/src/core/features/agent/conversationDraftStore";

const conversationId = "a1111111-1111-4111-8111-111111111111", requestId = "b1111111-1111-4111-8111-111111111111";
const turnId = "c1111111-1111-4111-8111-111111111111", contextSnapshotId = "d1111111-1111-4111-8111-111111111111";
const epoch = "e1111111-1111-4111-8111-111111111111";
const scope = {paperId: "paper", conversationId};
const request = {...scope, requestId, paperContextSnapshotId: "snapshot", expectedContextHash: "a".repeat(64), text: "Explain"};
const metadata = {sourceCount: 1, imageCount: 1, queuedInputId: null, draftAcknowledgment: null};
const result = {schemaVersion: 1 as const, ...scope, requestId, paperContextSnapshotId: "snapshot", turnId, contextSnapshotId,
  input: {text: request.text, model: {providerId: "deepseek", modelId: "fixture", revision: null}, reasoningEffort: null, context: []}};
const detail = {operationId: requestId, epoch, sqlFence: "0", phase: "paper-prepare-status", sqliteCode: null, sqliteExtendedCode: null, safeCause: "paper_prepare_worker_unconfirmed"};
const proof = {schemaVersion: 1 as const, operationId: requestId, conversationId, family: "agent.paper.submission.prepare" as const, epoch, sqlFence: "0", replayed: true, result: null};
const startRequest = {...scope, requestId, paperContextSnapshotId: "snapshot", contextSnapshotId, model: result.input.model, mode: "chatGptDefault" as const};
const started = {schemaVersion: 1 as const, conversationId, requestId, turnId, alreadyStarted: true, outcome: null};
function fixture() {
  const rows = new Map<string, string>();
  const storage: Storage = {get length() {return rows.size;}, key: index => [...rows.keys()][index] ?? null,
    getItem: key => rows.get(key) ?? null, setItem: (key, value) => {rows.set(key, value);}, removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
  const prepare = vi.fn<AgentPaperSubmissionGateway["prepare"]>().mockRejectedValue("lost acknowledgment");
  const read = vi.fn<AgentPaperPrepareStatusGateway["read"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "unconfirmed", detail}}));
  const open = () => new PaperPreparation({prepare}, {read}, storage, () => {}, scope);
  return {rows, storage, prepare, read, open};
}
function confirmPreparation(f: ReturnType<typeof fixture>) {
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof, result}}));
}
function queueReceipt(value: AgentPaperQueueEditRequest, receiptEpoch = epoch) {
  return {schemaVersion: 1 as const, intent: agentQueueEditIntentFromRequest(value), status: {
    type: "committed" as const, proof: {schemaVersion: 1 as const, operationId: value.requestId, conversationId,
      family: "agent.input_queue.edit" as const, epoch: receiptEpoch, sqlFence: "0", replayed: true, result: null},
    queue: {schemaVersion: 1 as const, conversationId, revision: value.expectedRevision + 1, paused: false, preparingInputId: null, inputs: []},
  }};
}
function persistence(storage: Storage, flush: () => Promise<void>) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false, flush, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
}
it("freezes the original preparation proof and rejects a different environment after reopening", async () => {
  const f = fixture(), proven = {prepare: vi.fn(async () => ({type: "committed" as const, proof, result}))};
  const open = () => new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  const owner = open(); await owner.begin(request, metadata); expect(owner.getSnapshot()).toMatchObject({phase: "ready", prepareProof: proof});
  expect(JSON.parse([...f.rows.values()][0]!).schemaVersion).toBe(6); owner.dispose();
  const reopened = open(); f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof: {...proof, epoch: contextSnapshotId}, result}}));
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("unknown"); expect(f.rows.size).toBe(1);
});
it("keeps the first proof when a later status marks the same receipt as replayed", async () => {
  const f = fixture(), original = {...proof, replayed: false}, proven = {prepare: vi.fn(async () => ({type: "committed" as const, proof: original, result}))};
  const open = () => new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  const owner = open(); await owner.begin(request, metadata); owner.dispose(); confirmPreparation(f);
  const reopened = open(); await reopened.check(); expect(reopened.getSnapshot()).toMatchObject({phase: "ready", prepareProof: original});
});
it("recovers a lost reply only from the original guarded environment", async () => {
  const f = fixture(), proven = {prepare: vi.fn().mockRejectedValue(new Error("lost"))};
  const open = () => new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  const owner = open(); await owner.begin(request, metadata); owner.dispose(); confirmPreparation(f);
  const reopened = open(); await reopened.check(); expect(reopened.getSnapshot().phase).toBe("ready");
  expect(reopened.getSnapshot().prepareProof).toEqual(proof); expect(proven.prepare).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(1);
  expect(proven.prepare).toHaveBeenCalledWith(expect.anything(), {expectedEpoch: epoch}); expect(f.read).toHaveBeenLastCalledWith(expect.anything(), {expectedEpoch: epoch});
});
it("does not prepare when the preflight cannot identify the original environment", async () => {
  const f = fixture(), proven = {prepare: vi.fn(async () => ({type: "committed" as const, proof, result}))};
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "unconfirmed", detail: {...detail, epoch: null}}}));
  const owner = new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  await owner.begin(request, metadata); expect(proven.prepare).not.toHaveBeenCalled(); expect(owner.getSnapshot().phase).toBe("unknown");
});
it("does not dispatch when persisting the preflight environment fails", async () => {
  const f = fixture(), proven = {prepare: vi.fn(async () => ({type: "committed" as const, proof, result}))};
  let flushes = 0; persistence(f.storage, async () => {if (++flushes === 2) throw new Error("epoch flush failed");});
  const owner = new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  await owner.begin(request, metadata); expect(flushes).toBe(2); expect(proven.prepare).not.toHaveBeenCalled();
  expect(owner.getSnapshot().phase).toBe("unknown");
});
it("refuses a recovered receipt from another environment when the original reply was lost", async () => {
  const f = fixture(), proven = {prepare: vi.fn().mockRejectedValue(new Error("lost"))};
  const owner = new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  await owner.begin(request, metadata);
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof: {...proof, epoch: contextSnapshotId}, result}}));
  await owner.check(); expect(owner.getSnapshot().phase).toBe("unknown"); expect(owner.getSnapshot().prepareProof).toBeNull();
});
it("does not dispatch a start when the original preparation environment changed", async () => {
  const f = fixture(), proven = {prepare: vi.fn(async () => ({type: "committed" as const, proof, result}))};
  const owner = new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  await owner.begin(request, metadata);
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof: {...proof, epoch: contextSnapshotId}, result}}));
  const start = vi.fn().mockResolvedValue(started); owner.configureStart({start}, {read: vi.fn()});
  await owner.confirmStart(startRequest); expect(start).not.toHaveBeenCalled(); expect(owner.getSnapshot().phase).toBe("unknown"); expect(f.rows.size).toBe(1);
});
it("passes the frozen preparation environment to start and later start inspection", async () => {
  const f = fixture(), proven = {prepare: vi.fn(async () => ({type: "committed" as const, proof, result}))};
  const owner = new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  await owner.begin(request, metadata); confirmPreparation(f);
  const start = vi.fn().mockRejectedValue(new Error("lost"));
  const read = vi.fn().mockResolvedValue({schemaVersion: 1, intent: startRequest, status: {type: "unconfirmed", detail}});
  owner.configureStart({start}, {read}); await owner.confirmStart(startRequest); await owner.check();
  expect(start).toHaveBeenCalledWith(result, startRequest.mode, {expectedEpoch: epoch});
  expect(read).toHaveBeenCalledWith(startRequest, {expectedEpoch: epoch});
});
it("does not clear a started preparation if its original environment changes", async () => {
  const f = fixture(), proven = {prepare: vi.fn(async () => ({type: "committed" as const, proof, result}))};
  const owner = new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, () => {}, scope, proven);
  await owner.begin(request, metadata); confirmPreparation(f); owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof: {...proof, epoch: contextSnapshotId}, result}}));
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.rows.size).toBe(1);
});
it("flushes the complete original request before preparing", async () => {
  const f = fixture(), owner = f.open(); let release!: () => void;
  persistence(f.storage, () => new Promise<void>(resolve => {release = resolve;}));
  const caller = structuredClone(request), work = owner.begin(caller, metadata);
  await vi.waitFor(() => expect(release).toBeTypeOf("function")); expect(f.prepare).not.toHaveBeenCalled();
  caller.text = "changed"; expect(JSON.parse([...f.rows.values()][0]!).intent.text).toBe("Explain");
  release(); await work; expect(f.prepare).toHaveBeenCalledOnce(); expect(owner.getSnapshot().phase).toBe("unknown");
});
it("serializes synchronous reentrant checks with the active preparation", async () => {
  const f = fixture(); let owner!: PaperPreparation;
  owner = new PaperPreparation({prepare: f.prepare}, {read: f.read}, f.storage, state => {
    if (state.phase === "saving") void owner.check();
  }, scope);
  await owner.begin(request, metadata); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.read).not.toHaveBeenCalled();
});
it("reopens without resending and retains an unconfirmed original intent", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, metadata); owner.dispose(); const restored = f.open();
  expect(restored.getSnapshot()).toMatchObject({phase: "unknown", recovered: true, intent: request});
  await restored.check(); expect(restored.getSnapshot().phase).toBe("unconfirmed");
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.read).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(1);
});
it("recovers the committed original result using its exact receipt", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, metadata); owner.dispose();
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof, result}}));
  const restored = f.open(); await restored.check(); expect(restored.getSnapshot()).toMatchObject({phase: "ready", result, status: {type: "committed"}});
  expect(f.prepare).toHaveBeenCalledOnce(); expect(JSON.parse([...f.rows.values()][0]!).result.turnId).toBe(turnId);
});
it("persists an acknowledged result but rechecks it after reopening", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result);
  await owner.begin(request, metadata); expect(owner.getSnapshot().phase).toBe("ready"); owner.dispose();
  const restored = f.open(); expect(restored.getSnapshot()).toMatchObject({phase: "unknown", result});
  await restored.check(); expect(restored.getSnapshot().phase).toBe("unconfirmed");
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(1);
});
it("does not replace an acknowledged result with another turn", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result);
  await owner.begin(request, metadata); owner.dispose(); const restored = f.open();
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof, result: {...result, turnId: epoch}}}));
  await restored.check(); expect(restored.getSnapshot().phase).toBe("unknown");
  expect(JSON.parse([...f.rows.values()][0]!).result.turnId).toBe(turnId);
});
it("does not send after a failed local flush or overwrite a different locator", async () => {
  const f = fixture(), owner = f.open(); persistence(f.storage, async () => {throw new Error("flush failed");});
  await owner.begin(request, metadata); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.prepare).not.toHaveBeenCalled();
  const locator = [...f.rows.keys()][0]!; f.rows.set(locator, JSON.stringify({...JSON.parse(f.rows.get(locator)!), intent: {...request, requestId: turnId}}));
  persistence(f.storage, async () => {}); await owner.check(); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.read).not.toHaveBeenCalled();
});
it("never lets a late acknowledgment replace another locator after disposal", async () => {
  const f = fixture(), owner = f.open(); let resolve!: (value: typeof result) => void;
  f.prepare.mockImplementation(() => new Promise(done => {resolve = done;})); const work = owner.begin(request, metadata);
  await vi.waitFor(() => expect(resolve).toBeTypeOf("function")); owner.dispose(); const locator = [...f.rows.keys()][0]!;
  const newer = JSON.stringify({...JSON.parse(f.rows.get(locator)!), intent: {...request, requestId: turnId}}); f.rows.set(locator, newer);
  resolve(result); await work; expect(f.rows.get(locator)).toBe(newer);
});
it("fences a late acknowledgment when another owner replaces the locator", async () => {
  const f = fixture(), owner = f.open(); let resolve!: (value: typeof result) => void;
  f.prepare.mockImplementation(() => new Promise(done => {resolve = done;})); const work = owner.begin(request, metadata);
  await vi.waitFor(() => expect(resolve).toBeTypeOf("function")); const locator = [...f.rows.keys()][0]!;
  const newer = JSON.stringify({...JSON.parse(f.rows.get(locator)!), intent: {...request, requestId: turnId}}); f.rows.set(locator, newer);
  resolve(result); await work; expect(owner.getSnapshot().phase).toBe("localError"); expect(f.rows.get(locator)).toBe(newer);
});
it("treats a malformed saved locator as a barrier", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, metadata); owner.dispose(); const locator = [...f.rows.keys()][0]!;
  f.rows.set(locator, "not-json"); const restored = f.open(); expect(restored.getSnapshot().phase).toBe("localError");
  await restored.begin(request, metadata); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.rows.get(locator)).toBe("not-json");
});
it("binds retained draft provenance to the original paper and conversation", async () => {
  const f = fixture(), owner = f.open();
  const token = {schemaVersion: 1 as const, revision: epoch, originKey: JSON.stringify(["paper", scope.paperId, conversationId])};
  await owner.begin(request, {...metadata, draftAcknowledgment: token}); owner.dispose();
  expect(f.open().getSnapshot().context?.draftAcknowledgment).toEqual(token);
  const key = [...f.rows.keys()][0]!, saved = JSON.parse(f.rows.get(key)!);
  saved.context.draftAcknowledgment.originKey = JSON.stringify(["paper", "other-paper", conversationId]);
  f.rows.set(key, JSON.stringify(saved)); expect(f.open().getSnapshot().phase).toBe("localError");
});
it("retains the exact start before dispatch and only inspects after a lost acknowledgment", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, metadata);
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockRejectedValue("lost acknowledgment");
  const read = vi.fn<AgentPaperStartStatusGateway["read"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "unconfirmed", detail}}));
  owner.configureStart({start}, {read}); await owner.confirmStart(startRequest);
  expect(owner.getSnapshot()).toMatchObject({phase: "unknown", start: {request: startRequest, result: null}});
  expect(JSON.parse([...f.rows.values()][0]!).start.request).toEqual(startRequest);
  owner.dispose(); const restored = f.open(); restored.configureStart({start}, {read}); await restored.check();
  expect(restored.getSnapshot().phase).toBe("unconfirmed"); await restored.confirmStart(startRequest);
  expect(start).toHaveBeenCalledOnce(); expect(read).toHaveBeenCalledTimes(2);
  read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: started}}));
  await restored.check(); expect(restored.getSnapshot()).toMatchObject({phase: "started", start: {result: started}});
  expect(JSON.parse([...f.rows.values()][0]!).start.result.turnId).toBe(turnId);
});
it("does not start if the original start request cannot be flushed", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, metadata);
  persistence(f.storage, async () => {throw new Error("flush failed");});
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>();
  owner.configureStart({start}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  expect(owner.getSnapshot().phase).toBe("localError"); expect(start).not.toHaveBeenCalled();
});
it("refuses a different start or a historical result for another turn", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, metadata);
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockRejectedValue("lost acknowledgment");
  const read = vi.fn<AgentPaperStartStatusGateway["read"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: {...started, turnId: epoch}}}));
  owner.configureStart({start}, {read});
  await expect(owner.confirmStart({...startRequest, contextSnapshotId: epoch})).rejects.toThrow("PAPER_START_MISMATCH");
  expect(start).not.toHaveBeenCalled(); await owner.confirmStart(startRequest); await owner.check();
  expect(owner.getSnapshot().phase).toBe("unknown"); expect(start).toHaveBeenCalledOnce();
});
it("retains an acknowledged start across reopening and rechecks historical admission", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, metadata);
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockResolvedValue({...started, alreadyStarted: false});
  const read = vi.fn<AgentPaperStartStatusGateway["read"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: started}}));
  owner.configureStart({start}, {read}); await owner.confirmStart(startRequest); expect(owner.getSnapshot().phase).toBe("started");
  owner.dispose(); const restored = f.open(); restored.configureStart({start}, {read});
  expect(restored.getSnapshot().phase).toBe("unknown"); await restored.check();
  expect(restored.getSnapshot()).toMatchObject({phase: "started", start: {result: {alreadyStarted: false}}});
  expect(start).toHaveBeenCalledOnce(); expect(read).toHaveBeenCalledOnce();
});
it("reads the previous saved preparation format without inventing a start", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, metadata); owner.dispose();
  const key = [...f.rows.keys()][0]!, saved = JSON.parse(f.rows.get(key)!); delete saved.start; delete saved.queueRemoval; saved.schemaVersion = 1;
  f.rows.set(key, JSON.stringify(saved)); const restored = f.open();
  expect(restored.getSnapshot()).toMatchObject({phase: "unknown", start: null}); await restored.check();
  expect(JSON.parse(f.rows.get(key)!).schemaVersion).toBe(3); expect(f.prepare).toHaveBeenCalledOnce();
});
it("reads a version 2 saved start without inventing a queue operation", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, metadata);
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest); owner.dispose();
  const key = [...f.rows.keys()][0]!, saved = JSON.parse(f.rows.get(key)!); delete saved.queueRemoval; saved.schemaVersion = 2;
  f.rows.set(key, JSON.stringify(saved)); const restored = f.open();
  expect(restored.getSnapshot()).toMatchObject({phase: "unknown", queueRemoval: null, start: {request: startRequest}});
});
it.each([false, true])("clears only the acknowledged draft version after a confirmed start; edited=%s", async edited => {
  const f = fixture(), owner = f.open(), drafts = new ConversationDraftStore(), draftScope = {hostKind: "paper" as const, hostId: scope.paperId, conversationId};
  const draftKey = conversationDraftKey(draftScope); drafts.update(draftKey, request.text);
  const token = drafts.captureAcknowledgmentToken(draftKey, request.text); expect(token).not.toBeNull();
  owner.configureDraft(drafts); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, draftAcknowledgment: token});
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockResolvedValue({...started, alreadyStarted: false});
  owner.configureStart({start}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  if (edited) drafts.update(draftKey, "A newer draft");
  await owner.finish(); expect(owner.getSnapshot().cause).toBeNull(); expect(owner.getSnapshot().phase).toBe("complete"); expect(f.rows.size).toBe(0);
  expect(drafts.read(draftKey)).toBe(edited ? "A newer draft" : "");
});
it("does not clear a queued input without its own queue-edit receipt", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result);
  await owner.begin(request, {...metadata, queuedInputId: epoch});
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("started"); expect(f.rows.size).toBe(1);
});
it("persists the exact queue removal before sending and clears only after acknowledgment", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f);
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const read = vi.fn(async () => initial);
  const edit = vi.fn(async (value: AgentPaperQueueEditRequest): Promise<AgentPaperQueueResponse> => {
    expect(JSON.parse([...f.rows.values()][0]!).queueRemoval).toEqual({request: value, proof: null});
    return {...initial, revision: 1, inputs: []};
  });
  const queue = new AgentInputQueueController({read, edit}, scope, () => turnId); await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)});
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("complete"); expect(f.rows.size).toBe(0);
  expect(edit).toHaveBeenCalledExactlyOnceWith({...scope, requestId: turnId, expectedRevision: 0, expectedEpoch: epoch, edit: {type: "remove", inputId: epoch}});
});
it("only inspects an old unguarded ambiguous removal and never resends it", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f);
  const start = vi.fn().mockResolvedValue(started), readStart = vi.fn(async intent => ({schemaVersion: 1 as const, intent, status: {type: "started" as const, result: started}}));
  owner.configureStart({start}, {read: readStart}); await owner.confirmStart(startRequest);
  const key = [...f.rows.keys()][0]!, old = JSON.parse(f.rows.get(key)!);
  old.queueRemoval = {request: {...scope, requestId: turnId, expectedRevision: 0, edit: {type: "remove", inputId: epoch}}, proof: null};
  f.rows.set(key, JSON.stringify(old)); owner.dispose();
  const reopened = f.open(); reopened.configureStart({start}, {read: readStart}); await reopened.check();
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const edit = vi.fn(async () => initial), queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => requestId);
  await queue.reload(); const inspectEdit = vi.fn(async value => ({schemaVersion: 1 as const, intent: agentQueueEditIntentFromRequest(value), status: {type: "notFound" as const}}));
  reopened.configureQueue(queue, {inspectEdit}); await reopened.finish();
  expect(reopened.getSnapshot().phase).toBe("localError"); expect(inspectEdit).toHaveBeenCalledOnce(); expect(edit).not.toHaveBeenCalled(); expect(f.rows.size).toBe(1);
});
it("settles an old unguarded removal only from its committed original receipt", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch}); confirmPreparation(f);
  const start = vi.fn().mockResolvedValue(started), readStart = vi.fn(async intent => ({schemaVersion: 1 as const, intent, status: {type: "started" as const, result: started}}));
  owner.configureStart({start}, {read: readStart}); await owner.confirmStart(startRequest);
  const key = [...f.rows.keys()][0]!, old = JSON.parse(f.rows.get(key)!);
  old.queueRemoval = {request: {...scope, requestId: turnId, expectedRevision: 0, edit: {type: "remove", inputId: epoch}}, proof: null};
  f.rows.set(key, JSON.stringify(old)); owner.dispose();
  const reopened = f.open(); reopened.configureStart({start}, {read: readStart}); await reopened.check();
  const current: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 1, paused: false, preparingInputId: null, inputs: []};
  const edit = vi.fn(async () => current), queue = new AgentInputQueueController({read: async () => current, edit}, scope, () => requestId);
  await queue.reload(); const inspectEdit = vi.fn(async value => queueReceipt(value)); reopened.configureQueue(queue, {inspectEdit});
  await reopened.finish(); expect(reopened.getSnapshot().phase).toBe("complete"); expect(edit).not.toHaveBeenCalled(); expect(inspectEdit).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(0);
});
it("retains the locator when a queue receipt belongs to a different data environment", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f); owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const queue = new AgentInputQueueController({read: async () => initial, edit: async () => ({...initial, revision: 1, inputs: []})}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value, contextSnapshotId)}); await owner.finish();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.rows.size).toBe(1);
  expect(JSON.parse([...f.rows.values()][0]!).queueRemoval.proof).toBeNull();
});
it("rejects a queue proof behind the original preparation fence without rounding", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  f.read.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", proof: {...proof, sqlFence: "9007199254740993"}, result}}));
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const queue = new AgentInputQueueController({read: async () => initial, edit: async () => ({...initial, revision: 1, inputs: []})}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); await owner.finish();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.rows.size).toBe(1);
});
it("does not clear when the claimed committed queue still contains the original input", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f); owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const queue = new AgentInputQueueController({read: async () => initial, edit: async () => ({...initial, revision: 1, inputs: []})}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => {
    const observed = queueReceipt(value); return {...observed, status: {...observed.status, queue: {...observed.status.queue, inputs: initial.inputs}}};
  }}); await owner.finish(); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.rows.size).toBe(1);
});
it("does not issue queue removal when the original preparation proof is unconfirmed", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const edit = vi.fn(async () => initial), queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); await owner.finish();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(edit).not.toHaveBeenCalled(); expect(f.rows.size).toBe(1);
});
it("rechecks the environment before using a saved queue proof after reopening", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f);
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockResolvedValue(started);
  const startRead = vi.fn<AgentPaperStartStatusGateway["read"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: started}}));
  owner.configureStart({start}, {read: startRead}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const current = {...initial, revision: 1, inputs: []}, edit = vi.fn(async () => current);
  const queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId); await queue.reload();
  const inspect = vi.fn(async (value: AgentPaperQueueEditRequest) => queueReceipt(value)); owner.configureQueue(queue, {inspectEdit: inspect});
  let flushes = 0; persistence(f.storage, async () => {if (++flushes === 3) throw new Error("proof flush failed");});
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("localError"); expect(JSON.parse([...f.rows.values()][0]!).queueRemoval.proof.epoch).toBe(epoch);
  owner.dispose(); persistence(f.storage, async () => {});
  const reopened = f.open(), reopenedQueue = new AgentInputQueueController({read: async () => current, edit}, scope, () => requestId);
  await reopenedQueue.reload(); reopened.configureQueue(reopenedQueue, {inspectEdit: inspect}); reopened.configureStart({start}, {read: startRead});
  await reopened.check(); await reopened.finish(); expect(reopened.getSnapshot().phase).toBe("complete");
  expect(edit).toHaveBeenCalledOnce(); expect(inspect).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(0);
});
it("preserves the queue input if it changed after the original submission", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f);
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 2, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: "New content"}]};
  const edit = vi.fn(async () => initial), queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); await owner.finish();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(edit).not.toHaveBeenCalled(); expect(f.rows.size).toBe(1);
});
it("finishes a queued submission with its frozen paper origin after reopening", async () => {
  const f = fixture(), owner = f.open(), input = {inputId: epoch, text: request.text, origin: {kind: "paper" as const, resourceId: scope.paperId}};
  f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch, queueInput: input, queueRevision: 2});
  expect(JSON.parse([...f.rows.values()][0]!).schemaVersion).toBe(4);
  confirmPreparation(f);
  const start = vi.fn().mockResolvedValue(started), readStart = vi.fn(async intent => ({schemaVersion: 1 as const, intent, status: {type: "started" as const, result: started}}));
  owner.configureStart({start}, {read: readStart}); await owner.confirmStart(startRequest); owner.dispose();
  const reopened = f.open(); reopened.configureStart({start}, {read: readStart}); await reopened.check();
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 2, paused: false, preparingInputId: null, inputs: [input]};
  const edit = vi.fn(async () => ({...initial, revision: 3, inputs: []}));
  const queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId);
  await queue.reload(); reopened.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); await reopened.finish();
  expect(reopened.getSnapshot().phase).toBe("complete"); expect(edit).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(0);
});
it.each(["different origin", "paused", "missing origin"] as const)
  ("retains a queued item whose frozen metadata changed: %s", async change => {
    const f = fixture(), owner = f.open(), input = {inputId: epoch, text: request.text, origin: {kind: "paper" as const, resourceId: scope.paperId}};
    f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch, queueInput: input, queueRevision: 2}); confirmPreparation(f);
    owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
    const changed = change === "different origin" ? {...input, origin: {kind: "paper" as const, resourceId: "other"}}
      : change === "paused" ? {...input, paused: true} : {inputId: input.inputId, text: input.text};
    const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 2, paused: false, preparingInputId: null, inputs: [changed]};
    const edit = vi.fn(async () => initial), queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId);
    await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); await owner.finish();
    expect(owner.getSnapshot().phase).toBe("localError"); expect(edit).not.toHaveBeenCalled(); expect(f.rows.size).toBe(1);
  });
it("does not delete an identical replacement at a later queue revision", async () => {
  const f = fixture(), owner = f.open(), input = {inputId: epoch, text: request.text, origin: {kind: "paper" as const, resourceId: scope.paperId}};
  f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch, queueInput: input, queueRevision: 2}); confirmPreparation(f);
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 4, paused: false, preparingInputId: null, inputs: [input]};
  const edit = vi.fn(async () => initial), queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); await owner.finish();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(edit).not.toHaveBeenCalled(); expect(f.rows.size).toBe(1);
});
it.each([{paused: true}, {origin: {kind: "paper" as const, resourceId: scope.paperId}}])("does not remove an input with unproved queue state %j", async patch => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f);
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 2, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text, ...patch}]};
  const edit = vi.fn(async () => initial), queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); await owner.finish();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(edit).not.toHaveBeenCalled(); expect(f.rows.size).toBe(1);
});
it("does not send queue removal when its original command cannot be flushed", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f);
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const edit = vi.fn(async () => initial), queue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId);
  await queue.reload(); owner.configureQueue(queue, {inspectEdit: async value => queueReceipt(value)}); let flushes = 0;
  persistence(f.storage, async () => {if (++flushes === 2) throw new Error("queue command flush failed");});
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("localError"); expect(edit).not.toHaveBeenCalled();
  expect(JSON.parse([...f.rows.values()][0]!).queueRemoval.request.requestId).toBe(turnId);
});
it("reopens a lost queue acknowledgment and verifies its original receipt before clearing", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, queuedInputId: epoch});
  confirmPreparation(f);
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockResolvedValue(started);
  const startRead = vi.fn<AgentPaperStartStatusGateway["read"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: started}}));
  owner.configureStart({start}, {read: startRead}); await owner.confirmStart(startRequest);
  const initial: AgentPaperQueueResponse = {schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [{inputId: epoch, text: request.text}]};
  const current: AgentPaperQueueResponse = {...initial, revision: 1, inputs: []};
  let reject!: (cause: Error) => void;
  const edit = vi.fn<(value: AgentPaperQueueEditRequest) => Promise<AgentPaperQueueResponse>>().mockImplementation(() => new Promise((_resolve, fail) => {reject = fail;}));
  const firstQueue = new AgentInputQueueController({read: async () => initial, edit}, scope, () => turnId); await firstQueue.reload(); owner.configureQueue(firstQueue, {inspectEdit: async value => queueReceipt(value)});
  const work = owner.finish(); await vi.waitFor(() => expect(edit).toHaveBeenCalledOnce());
  const original = edit.mock.calls[0]![0]; owner.dispose(); firstQueue.dispose(); reject(new Error("lost ACK")); await work;
  const inspect = vi.fn(async (value: AgentPaperQueueEditRequest) => queueReceipt(value));
  const retryEdit = vi.fn(async () => current), reopenedQueue = new AgentInputQueueController({read: async () => current, edit: retryEdit, inspectEdit: inspect}, scope, () => requestId);
  await reopenedQueue.reload(); const reopened = f.open(); reopened.configureStart({start}, {read: startRead}); reopened.configureQueue(reopenedQueue, {inspectEdit: inspect});
  await reopened.check(); await reopened.finish();
  expect(reopened.getSnapshot().phase).toBe("complete"); expect(inspect).toHaveBeenCalledTimes(2); expect(inspect).toHaveBeenNthCalledWith(1, original); expect(inspect).toHaveBeenNthCalledWith(2, original);
  expect(retryEdit).not.toHaveBeenCalled(); expect(start).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(0);
});
it("does not prepare if the captured draft cannot be flushed", async () => {
  const f = fixture(), owner = f.open(), token = {schemaVersion: 1 as const, revision: epoch, originKey: JSON.stringify(["paper", scope.paperId, conversationId])};
  owner.configureDraft({flush: vi.fn().mockRejectedValue(new Error("draft failed")), consumeAcknowledgment: vi.fn()});
  await owner.begin(request, {...metadata, draftAcknowledgment: token});
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.prepare).not.toHaveBeenCalled(); expect(f.rows.size).toBe(1);
});
it("does not remove a different locator installed while cleanup is flushing", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, metadata);
  owner.configureStart({start: vi.fn().mockResolvedValue(started)}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  const key = [...f.rows.keys()][0]!, newer = JSON.stringify({...JSON.parse(f.rows.get(key)!), intent: {...request, requestId: epoch}});
  let flushes = 0; persistence(f.storage, async () => {if (++flushes === 2) f.rows.set(key, newer);});
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.rows.get(key)).toBe(newer);
});
it("retries a failed cleanup flush without resending or resurrecting its locator", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(result); await owner.begin(request, metadata);
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockResolvedValue(started);
  owner.configureStart({start}, {read: vi.fn()}); await owner.confirmStart(startRequest);
  let flushes = 0; persistence(f.storage, async () => {if (++flushes === 3) throw new Error("cleanup flush failed");});
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.rows.size).toBe(0);
  await owner.finish(); expect(owner.getSnapshot().phase).toBe("complete"); expect(f.rows.size).toBe(0);
  expect(f.prepare).toHaveBeenCalledOnce(); expect(start).toHaveBeenCalledOnce();
});
it("can recover after draft consumption succeeds but its acknowledgment is lost", async () => {
  const f = fixture(), owner = f.open(), drafts = new ConversationDraftStore(), draftScope = {hostKind: "paper" as const, hostId: scope.paperId, conversationId};
  const draftKey = conversationDraftKey(draftScope); drafts.update(draftKey, request.text);
  const token = drafts.captureAcknowledgmentToken(draftKey, request.text); expect(token).not.toBeNull();
  owner.configureDraft({flush: () => drafts.flush(), consumeAcknowledgment: async (value, target) => {
    await drafts.consumeAcknowledgment(value, target); throw new Error("draft acknowledgment lost");
  }});
  f.prepare.mockResolvedValue(result); await owner.begin(request, {...metadata, draftAcknowledgment: token});
  const start = vi.fn<AgentPaperSubmissionGateway["start"]>().mockResolvedValue(started);
  const read = vi.fn<AgentPaperStartStatusGateway["read"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: started}}));
  owner.configureStart({start}, {read}); await owner.confirmStart(startRequest); await owner.finish();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(drafts.read(draftKey)).toBe(""); expect(f.rows.size).toBe(1);
  owner.dispose(); const restored = f.open(); restored.configureStart({start}, {read}); restored.configureDraft(drafts);
  await restored.check(); await restored.finish(); expect(restored.getSnapshot().phase).toBe("complete");
  expect(f.rows.size).toBe(0); expect(start).toHaveBeenCalledOnce();
});
