import {expect, it, vi} from "vitest";
import {AgentInputQueueController} from "@research-assistant/data/agent";
import type {AgentWritingQueuedStartRequest, AgentWritingQueueEditRequest, AgentWritingQueueResponse, AgentWritingQueueScope} from "@research-assistant/core/agent";
import {writingQueuedStartQueue} from "./writingQueuedStartQueue";
import {WritingQueuedStart} from "./writingQueuedStart";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import type {AgentInputQueuePreviewGateway} from "@research-assistant/data/agent";
const id = "11111111-1111-4111-8111-111111111111", inputId = "22222222-2222-4222-8222-222222222222";
const operationId = "33333333-3333-4333-8333-333333333333";
const scope = {conversationId: id, manuscriptId: id};
const intent: AgentWritingQueuedStartRequest = {inputId, expectedRevision: 2, start: {...scope, requestId: id, contextSnapshotId: id,
  model: {providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null}, mode: "deepSeekNonThinking"}};
const paused: AgentWritingQueueResponse = {schemaVersion: 1, ...scope, revision: 8, paused: true, preparingInputId: null, inputs: []};
const command: AgentWritingQueueEditRequest = {...scope, requestId: operationId, expectedRevision: 8, edit: {type: "resume"}};
function fixture() {
  const read = vi.fn(async () => paused), edit = vi.fn(async (_request: AgentWritingQueueEditRequest) => ({...paused, revision: 9, paused: false}));
  const owner = new AgentInputQueueController<AgentWritingQueueScope, AgentWritingQueueResponse>({read, edit}, scope, () => operationId);
  const release = vi.fn(), port = writingQueuedStartQueue(() => ({controller: owner, release}), read);
  return {read, edit, owner, release, port};
}
it("uses the existing queue owner and reliably saves the full Resume before any edit I/O", async () => {
  const f = fixture(); let release!: () => void;
  const beforeSend = vi.fn(async (request: AgentWritingQueueEditRequest) => {expect(request).toEqual(command); await new Promise<void>(resolve => {release = resolve;});});
  const work = f.port.resume(intent, null, beforeSend, () => true);
  await vi.waitFor(() => expect(beforeSend).toHaveBeenCalledOnce()); expect(f.edit).not.toHaveBeenCalled();
  release(); expect(await work).toBe("acknowledged"); expect(f.edit).toHaveBeenCalledExactlyOnceWith(command); expect(f.release).toHaveBeenCalledOnce();
});
it("a failed reliable save retains the original pending Resume and sends no mutation", async () => {
  const f = fixture(), save = vi.fn(async (): Promise<void> => {throw new Error("local flush failed");});
  expect(await f.port.resume(intent, null, save, () => true)).toBe("unknown");
  expect(f.edit).not.toHaveBeenCalled(); expect(f.owner.getPendingRequest()).toEqual(command);
  save.mockResolvedValue(undefined); expect(await f.port.resume(intent, command, save, () => true)).toBe("acknowledged");
  expect(f.edit).toHaveBeenCalledExactlyOnceWith(command);
});
it("reports only a fresh, explicitly rejected Resume as knownRejected", async () => {
  const f = fixture(); f.edit.mockRejectedValue({code: "conflict"});
  expect(await f.port.resume(intent, null, async () => {}, () => true)).toBe("knownRejected");
  expect(f.owner.getPendingRequest()).toBeNull(); expect(f.owner.getSnapshot().uncertain).toBe(false);
  expect(f.edit).toHaveBeenCalledExactlyOnceWith(command);
});
it("a restored Resume stays Unknown after a later explicit rejection and readonly reads still work", async () => {
  const f = fixture(); f.edit.mockRejectedValue({code: "conflict"});
  expect(await f.port.resume(intent, command, async () => {}, () => true)).toBe("unknown");
  expect(f.owner.getPendingRequest()).toEqual(command); expect(f.owner.getSnapshot().uncertain).toBe(true);
  expect(await f.port.read(intent)).toEqual(paused); expect(f.edit).toHaveBeenCalledExactlyOnceWith(command);
});
it("an old Resume acknowledgement cannot replace a later current Pause", async () => {
  const f = fixture(), later = {...paused, revision: 12}; f.read.mockResolvedValue(later);
  expect(await f.port.resume(intent, command, async () => {}, () => true)).toBe("acknowledged");
  expect(f.owner.getSnapshot().queue).toEqual(later); expect(await f.port.read(intent)).toEqual(later);
  expect(f.edit).toHaveBeenCalledExactlyOnceWith(command);
});
it("does not send or replace an edit when a different claim, owner, or original action is ineligible", async () => {
  const f = fixture(); expect(await f.port.resume(intent, null, async () => {}, () => false)).toBe("unknown");
  expect(f.edit).not.toHaveBeenCalled();
  f.read.mockResolvedValue({...paused, preparingInputId: inputId});
  await expect(f.port.resume(intent, command, async () => {}, () => true)).rejects.toThrow("QUEUE_START_OTHER_CLAIM"); expect(f.edit).not.toHaveBeenCalled();
  f.read.mockResolvedValue(paused); const other = {...command, requestId: inputId};
  expect(f.owner.restorePending(other)).toBe(true);
  await expect(f.port.resume(intent, command, async () => {}, () => true)).rejects.toThrow("QUEUE_START_QUEUE_RESUME_BUSY");
  expect(f.owner.getPendingRequest()).toEqual(other); expect(f.edit).not.toHaveBeenCalled();
});
it("a real queue owner sends no Resume if the start owner is disposed during its durable beforeSend flush", async () => {
  const f = fixture(), rows = new Map<string, string>(), storage: Storage = {get length() {return rows.size;}, key: index => [...rows.keys()][index] ?? null,
    getItem: key => rows.get(key) ?? null, setItem: (key, value) => {rows.set(key, value);}, removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
  let release!: () => void; const blocked = new Promise<void>(resolve => {release = resolve;});
  const flush = vi.fn(async () => {if ([...rows.values()].some(value => JSON.parse(value).resumeCommand)) await blocked;});
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false, flush,
    retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const preview = {schemaVersion: 1 as const, ...scope, requestId: id, contextSnapshotId: id, turnId: id,
    input: {text: "original", model: intent.start.model, reasoningEffort: null, context: []}};
  const gateway: AgentInputQueuePreviewGateway = {start: vi.fn(async () => {throw new Error("Lost start reply");}),
    startStatus: vi.fn<AgentInputQueuePreviewGateway["startStatus"]>(async () => ({schemaVersion: 1, intent, status: {type: "admitted", preview}})),
    prepare: vi.fn(), prepareStatus: vi.fn(), cancel: vi.fn(), cancelStatus: vi.fn(), recover: vi.fn()};
  const owner = new WritingQueuedStart(gateway, storage, () => {}, () => {}, async () => {}, scope);
  await owner.begin(intent, id); const work = owner.retry(f.port, true);
  await vi.waitFor(() => expect(owner.getSnapshot().resumeCommand).toEqual(command));
  expect(f.edit).not.toHaveBeenCalled(); owner.dispose(); release(); await work;
  expect(f.edit).not.toHaveBeenCalled(); expect(f.owner.getPendingRequest()).toEqual(command); expect(f.owner.getSnapshot().uncertain).toBe(false);
  f.owner.dispose();
});

it("keeps a fresh Resume local if admission closes inside its original durable beforeSend", async () => {
  const f = fixture(); let disabled = false, release!: () => void;
  const work = f.port.resume(intent, null, async () => new Promise<void>(resolve => {release = resolve;}), () => true, () => disabled);
  await vi.waitFor(() => expect(release).toBeTypeOf("function")); disabled = true; release();
  await expect(work).rejects.toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "resume"});
  expect(f.edit).not.toHaveBeenCalled(); expect(f.owner.getPendingRequest()).toBeNull();
});

it("removes a freshly blocked Resume command locally so warm retries cannot restore it as Unknown", async () => {
  const f = fixture(), rows = new Map<string, string>(), storage: Storage = {get length() {return rows.size;}, key: index => [...rows.keys()][index] ?? null,
    getItem: key => rows.get(key) ?? null, setItem: (key, value) => {rows.set(key, value);}, removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
  let disabled = false, release!: () => void; const gate = new Promise<void>(resolve => {release = resolve;});
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: async () => {if ([...rows.values()].some(value => JSON.parse(value).resumeCommand)) await gate;},
    retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const preview = {schemaVersion: 1 as const, ...scope, requestId: id, contextSnapshotId: id, turnId: id,
    input: {text: "original", model: intent.start.model, reasoningEffort: null, context: []}};
  const gateway: AgentInputQueuePreviewGateway = {start: vi.fn(async () => {throw new Error("Lost start reply");}),
    startStatus: vi.fn<AgentInputQueuePreviewGateway["startStatus"]>(async request => ({schemaVersion: 1, intent: request, status: {type: "admitted", preview}})),
    prepare: vi.fn(), prepareStatus: vi.fn(), cancel: vi.fn(), cancelStatus: vi.fn(), recover: vi.fn()};
  const owner = new WritingQueuedStart(gateway, storage, () => {}, () => {}, async () => {}, scope, () => disabled);
  await owner.begin(intent, id); const work = owner.retry(f.port, true);
  await vi.waitFor(() => expect(owner.getSnapshot().resumeCommand).toEqual(command)); disabled = true; release(); await work;
  expect(f.edit).not.toHaveBeenCalled(); expect(owner.getSnapshot()).toMatchObject({resumeCommand: null,
    cause: {code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "resume"}});
  expect(JSON.parse([...rows.values()][0]!).resumeCommand).toBeNull();
  await owner.retry(f.port, true); expect(f.edit).not.toHaveBeenCalled(); expect(gateway.start).toHaveBeenCalledOnce();
  owner.dispose(); f.owner.dispose();
});
