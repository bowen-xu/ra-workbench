import {expect, it, vi} from "vitest";
import type {AgentWritingQueuedStartRequest, AgentWritingStartResponse} from "@research-assistant/core/agent";
import type {AgentInputQueuePreviewGateway} from "@research-assistant/data/agent";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {ConversationDraftStore, conversationDraftKey} from "./conversationDraftStore";
import {localConversationDraftPersistence} from "./conversationDraftPersistence";
import {writingDraftAcknowledgments, type WritingDraftAcknowledgmentPort} from "./writingDraftAcknowledgment";
import {WritingQueuedStart} from "./writingQueuedStart";
import type {WritingQueuedStartQueuePort} from "./writingQueuedStartQueue";

class MemoryStorage implements Storage {
  readonly rows = new Map<string, string>(); get length() {return this.rows.size;}
  key(index: number) {return [...this.rows.keys()][index] ?? null;}
  getItem(key: string) {return this.rows.get(key) ?? null;} setItem(key: string, value: string) {this.rows.set(key, value);}
  removeItem(key: string) {this.rows.delete(key);} clear() {this.rows.clear();}
}
const scope = {conversationId: "11111111-1111-4111-8111-111111111111", manuscriptId: "22222222-2222-4222-8222-222222222222"};
const r = "33333333-3333-4333-8333-333333333333", i = "44444444-4444-4444-8444-444444444444", t = "55555555-5555-4555-8555-555555555555";
const intent: AgentWritingQueuedStartRequest = {inputId: i, expectedRevision: 7, start: {...scope, requestId: r,
  contextSnapshotId: "66666666-6666-4666-8666-666666666666", model: {providerId: "deepseek", modelId: "frozen", revision: null}, mode: "deepSeekNonThinking"}};
const result: AgentWritingStartResponse = {schemaVersion: 1, conversationId: scope.conversationId, requestId: r, turnId: t, alreadyStarted: false, outcome: null};
const locator = `research-assistant:writing-queue-start:v1:${scope.manuscriptId}:${scope.conversationId}`;
function persistent(storage: Storage, flush: () => Promise<void>): void {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
}
function fixture() {
  const storage = new MemoryStorage(), draftStorage = new MemoryStorage();
  const draftScope = {hostKind: "manuscript" as const, hostId: scope.manuscriptId, conversationId: scope.conversationId};
  const draftKey = conversationDraftKey(draftScope), persistence = localConversationDraftPersistence(draftStorage);
  const drafts = new ConversationDraftStore(persistence); drafts.restoreQueue(draftKey, {input: {inputId: i, text: "original"}, index: 0, preceding: [], following: []});
  const token = drafts.captureAcknowledgmentToken(draftKey, "original")!;
  const actual = writingDraftAcknowledgments(drafts), port = {flush: vi.fn(actual.flush), retry: vi.fn(actual.retry), consume: vi.fn(actual.consume)};
  const start = vi.fn<AgentInputQueuePreviewGateway["start"]>().mockResolvedValue(result);
  const unused = async (): Promise<never> => {throw new Error("unexpected remote recovery");};
  const startStatus = vi.fn<AgentInputQueuePreviewGateway["startStatus"]>(unused);
  const gateway: AgentInputQueuePreviewGateway = {start, startStatus, prepare: unused, prepareStatus: unused, recover: unused, cancel: unused, cancelStatus: unused};
  const observed = vi.fn<(_intent: AgentWritingQueuedStartRequest, _result: AgentWritingStartResponse) => void | Promise<void>>();
  const cleanup = vi.fn(async () => {});
  const queue: WritingQueuedStartQueuePort = {read: vi.fn(unused), resume: vi.fn(unused)};
  const open = (acknowledgments: WritingDraftAcknowledgmentPort | null = port) => {
    const owner = new WritingQueuedStart(gateway, storage, () => {}, observed, cleanup, scope);
    owner.configureDraftAcknowledgments(acknowledgments ?? undefined); return owner;
  };
  return {storage, draftStorage, draftKey, drafts, persistence, token, port, start, startStatus, observed, cleanup, queue, open};
}
it("saves the complete queue token before draft flush and cannot start when its port is unavailable", async () => {
  const f = fixture(), owner = f.open(null); await owner.begin(intent, t, f.token);
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.start).not.toHaveBeenCalled();
  expect(JSON.parse(f.storage.getItem(locator)!)).toMatchObject({schemaVersion: 2, intent, turnId: t, result: null, draftAcknowledgment: f.token});
  expect(f.token).toMatchObject({sourceInputId: i, sourceQueueRevision: expect.any(String)}); expect(f.drafts.read(f.draftKey)).toBe("original");
});
it("keeps a plain draft token plain and never manufactures a queue-origin identity from Native I", async () => {
  const f = fixture(); f.drafts.updateStructured(f.draftKey, {text: "original", bindings: []}, null);
  const token = f.drafts.captureAcknowledgmentToken(f.draftKey, "original")!;
  f.cleanup.mockRejectedValueOnce(new Error("retain local finish")); const owner = f.open(); await owner.begin(intent, t, token);
  expect(JSON.parse(f.storage.getItem(locator)!).draftAcknowledgment).toEqual(token);
  expect(token).not.toHaveProperty("sourceInputId"); expect(token).not.toHaveProperty("sourceQueueRevision");
  expect(f.start).toHaveBeenCalledExactlyOnceWith(intent);
});
it("delivers admission and consumes the draft only after the known Native result reliably flushes", async () => {
  const f = fixture(); let writes = 0, release: (() => void) | undefined;
  persistent(f.storage, async () => {if (++writes === 2) await new Promise<void>(resolve => {release = resolve;});});
  const owner = f.open(), work = owner.begin(intent, t, f.token);
  await vi.waitFor(() => expect(release).toBeTypeOf("function"));
  expect(owner.getSnapshot().result).toEqual(result); expect(f.port.consume).not.toHaveBeenCalled(); expect(f.observed).not.toHaveBeenCalled(); expect(f.cleanup).not.toHaveBeenCalled();
  expect(f.drafts.read(f.draftKey)).toBe("original"); release!(); await work;
  expect(f.start).toHaveBeenCalledExactlyOnceWith(intent); expect(f.port.consume).toHaveBeenCalledExactlyOnceWith(f.token, scope); expect(f.observed).toHaveBeenCalledOnce();
  expect(f.drafts.read(f.draftKey)).toBe(""); expect(owner.getSnapshot().phase).toBe("done");
});
it("result-save failure retains the original draft and retries only local saving and CAS", async () => {
  const f = fixture(); let writes = 0, failing = true;
  persistent(f.storage, async () => {if (++writes === 2 && failing) throw new Error("result flush unavailable");});
  const owner = f.open(); await owner.begin(intent, t, f.token);
  expect(owner.getSnapshot()).toMatchObject({phase: "localError", result}); expect(f.port.consume).not.toHaveBeenCalled(); expect(f.observed).not.toHaveBeenCalled();
  failing = false; await owner.retry(f.queue, true);
  expect(f.start).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled(); expect(f.queue.read).not.toHaveBeenCalled(); expect(f.queue.resume).not.toHaveBeenCalled();
  expect(f.drafts.read(f.draftKey)).toBe(""); expect(owner.getSnapshot().phase).toBe("done"); expect(f.observed).toHaveBeenCalledOnce();
});
it("cold known result retries CAS and local flush without status, start, Resume or a freshly captured token", async () => {
  const f = fixture(); f.port.consume.mockRejectedValueOnce(new Error("draft save unavailable"));
  const owner = f.open(); await owner.begin(intent, t, f.token); owner.dispose();
  expect(JSON.parse(f.storage.getItem(locator)!)).toMatchObject({result, draftAcknowledgment: f.token}); expect(f.cleanup).not.toHaveBeenCalled();
  const cold = new ConversationDraftStore(f.persistence), capture = vi.spyOn(cold, "captureAcknowledgmentToken");
  const reopened = f.open(writingDraftAcknowledgments(cold)); await reopened.retry(f.queue, true);
  expect(cold.read(f.draftKey)).toBe(""); expect(capture).not.toHaveBeenCalled(); expect(f.start).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled();
  expect(f.queue.read).not.toHaveBeenCalled(); expect(f.queue.resume).not.toHaveBeenCalled(); expect(reopened.getSnapshot().phase).toBe("done");
});
it("a real draft deletion-flush failure retains known start evidence and never consumes a later identical revision", async () => {
  const f = fixture(); let writes = 0, failing = true;
  persistent(f.draftStorage, async () => {if (++writes === 2 && failing) throw new Error("draft deletion flush unavailable");});
  const owner = f.open(); await owner.begin(intent, t, f.token);
  expect(owner.getSnapshot()).toMatchObject({phase: "localError", result});
  expect(JSON.parse(f.storage.getItem(locator)!)).toMatchObject({result, draftAcknowledgment: f.token});
  expect(f.observed).toHaveBeenCalledOnce(); expect(f.cleanup).not.toHaveBeenCalled();
  f.drafts.update(f.draftKey, "original"); failing = false; await owner.retry(f.queue, true);
  expect(f.drafts.read(f.draftKey)).toBe("original"); expect(new ConversationDraftStore(f.persistence).read(f.draftKey)).toBe("original");
  expect(f.start).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled(); expect(f.queue.read).not.toHaveBeenCalled();
  expect(owner.getSnapshot().phase).toBe("done");
});
it.each(["retyped", "reference"] as const)("preserves a newer %s draft while consuming the old queue token", async change => {
  const f = fixture(); let accepted!: (value: AgentWritingStartResponse) => void;
  f.start.mockImplementationOnce(() => new Promise(resolve => {accepted = resolve;}));
  const owner = f.open(), work = owner.begin(intent, t, f.token); await vi.waitFor(() => expect(f.start).toHaveBeenCalledOnce());
  if (change === "retyped") {f.drafts.update(f.draftKey, ""); f.drafts.update(f.draftKey, "original");}
  else f.drafts.updateStructured(f.draftKey, {...f.drafts.readStructured(f.draftKey), unboundReferences: [
    {type: "skill", name: "later", identity: {environment_id: "app", source_id: "later/SKILL.md", revision: "v2"}}]});
  const later = structuredClone(f.drafts.readStructured(f.draftKey)); accepted(result); await work;
  expect(f.drafts.readStructured(f.draftKey).text).toBe(later.text); expect(f.drafts.readStructured(f.draftKey).bindings).toEqual(later.bindings);
  expect(f.drafts.readStructured(f.draftKey).unboundReferences).toEqual(later.unboundReferences);
  expect(f.drafts.readStructured(f.draftKey).queuePlacement).toBeUndefined(); expect(owner.getSnapshot().phase).toBe("done");
});
it("saves the known result before workspace observation, then retries removal without duplicate delivery or cleanup", async () => {
  const f = fixture(); let finish: (() => void) | undefined, writes = 0;
  persistent(f.storage, async () => {if (++writes === 3) throw new Error("remove flush unavailable");});
  f.observed.mockImplementationOnce(async () => {expect(f.drafts.read(f.draftKey)).toBe("original"); await new Promise<void>(resolve => {finish = resolve;});});
  const owner = f.open(), work = owner.begin(intent, t, f.token); await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
  expect(f.cleanup).not.toHaveBeenCalled(); expect(f.port.consume).not.toHaveBeenCalled(); expect(JSON.parse(f.storage.getItem(locator)!).result).toEqual(result);
  finish!(); await work; expect(owner.getSnapshot().phase).toBe("localError"); expect(f.storage.getItem(locator)).toBeNull();
  await owner.check(); expect(owner.getSnapshot().phase).toBe("done"); expect(f.port.consume).toHaveBeenCalledOnce(); expect(f.observed).toHaveBeenCalledOnce(); expect(f.cleanup).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled();
});
it("v1 known results remain tokenless and cannot clear a cold same-text draft", async () => {
  const f = fixture(); f.storage.setItem(locator, JSON.stringify({schemaVersion: 1, intent, turnId: t, knownRejected: false, result, resumeCommand: null, resumeAcknowledged: false}));
  const owner = f.open(); expect(owner.getSnapshot().draftAcknowledgment).toBeNull(); await owner.check();
  expect(f.drafts.read(f.draftKey)).toBe("original"); expect(f.port.consume).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled(); expect(f.startStatus).not.toHaveBeenCalled();
});
it("foreign or incomplete queued tokens fail closed before any RPC or locator cleanup", async () => {
  const f = fixture();
  for (const token of [{...f.token, originKey: JSON.stringify(["manuscript", crypto.randomUUID(), scope.conversationId])}, {...f.token, sourceQueueRevision: undefined}]) {
    f.storage.setItem(locator, JSON.stringify({schemaVersion: 2, intent, turnId: t, knownRejected: false, result, draftAcknowledgment: token, resumeCommand: null, resumeAcknowledged: false}));
    const owner = f.open(); expect(owner.getSnapshot().phase).toBe("localError"); await owner.retry(f.queue, true);
    expect(f.start).not.toHaveBeenCalled(); expect(f.startStatus).not.toHaveBeenCalled(); expect(f.port.consume).not.toHaveBeenCalled(); expect(f.cleanup).not.toHaveBeenCalled();
  }
});
it.each(["CAS", "cleanup"] as const)("preserves a same-R replacement token arriving during deferred %s and performs no remote recovery", async stage => {
  const f = fixture();
  f.storage.setItem(locator, JSON.stringify({schemaVersion: 2, intent, turnId: t, knownRejected: false, result, draftAcknowledgment: f.token, resumeCommand: null, resumeAcknowledged: false}));
  let release: (() => void) | undefined;
  if (stage === "CAS") {
    const consume = f.port.consume.getMockImplementation()!;
    f.port.consume.mockImplementationOnce(async (token, own) => {await new Promise<void>(resolve => {release = resolve;}); await consume(token, own);});
  } else f.cleanup.mockImplementationOnce(async () => {await new Promise<void>(resolve => {release = resolve;});});
  const owner = f.open(), work = owner.check(); await vi.waitFor(() => expect(release).toBeTypeOf("function"));
  f.drafts.update(f.draftKey, ""); f.drafts.update(f.draftKey, "original");
  const newerToken = f.drafts.captureAcknowledgmentToken(f.draftKey, "original")!;
  const newer = JSON.stringify({...JSON.parse(f.storage.getItem(locator)!), draftAcknowledgment: newerToken});
  f.storage.setItem(locator, newer); release!(); await work;
  expect(owner.getSnapshot()).toMatchObject({phase: "localError", result, draftAcknowledgment: f.token}); expect(f.storage.getItem(locator)).toBe(newer);
  expect(f.drafts.read(f.draftKey)).toBe("original"); if (stage === "CAS") expect(f.cleanup).not.toHaveBeenCalled();
  await owner.retry(f.queue, true); expect(f.storage.getItem(locator)).toBe(newer);
  expect(f.start).not.toHaveBeenCalled(); expect(f.startStatus).not.toHaveBeenCalled(); expect(f.queue.read).not.toHaveBeenCalled(); expect(f.queue.resume).not.toHaveBeenCalled();
});
it("a remove-flush retry cannot delete a same-R start locator now owned by another token", async () => {
  const f = fixture(); let writes = 0;
  persistent(f.storage, async () => {if (++writes === 3) throw new Error("old removal not flushed");});
  const owner = f.open(); await owner.begin(intent, t, f.token); expect(f.storage.getItem(locator)).toBeNull();
  const newerToken = {...f.token, revision: crypto.randomUUID()};
  const newer = JSON.stringify({schemaVersion: 2, intent, turnId: t, knownRejected: false, result, draftAcknowledgment: newerToken, resumeCommand: null, resumeAcknowledged: false});
  f.storage.setItem(locator, newer); await owner.check();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.storage.getItem(locator)).toBe(newer);
  expect(f.start).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled(); expect(f.cleanup).toHaveBeenCalledOnce();
});
