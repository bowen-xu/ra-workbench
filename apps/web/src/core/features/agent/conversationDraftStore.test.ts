import {afterEach, expect, it, vi} from "vitest";
import {insertAgentReference, type AgentEvidenceInput, type AgentReferenceInput} from "@research-assistant/core/agent";
import {ConversationDraftStore, conversationDraftKey, decodeConversationDraftAcknowledgmentToken, type ConversationDraftScope} from "./conversationDraftStore";
import {localConversationDraftPersistence} from "./conversationDraftPersistence";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {releaseRemovedEvidence} from "./evidenceImageGateway";
vi.mock("./evidenceImageGateway", () => ({releaseRemovedEvidence: vi.fn()}));
afterEach(() => vi.clearAllMocks());
const scope: ConversationDraftScope = {hostKind: "manuscript", hostId: "original-host", conversationId: "original-chat"};
const key = conversationDraftKey(scope), storageKey = "research-assistant.agent-drafts.v1";
const id = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const skill: AgentReferenceInput = {type: "skill", name: "review", identity: {environment_id: "app", source_id: "review/SKILL.md", revision: "v1"}};
const annotation: AgentEvidenceInput = {type: "annotation", source: "source://notes/note", title: "Image", text: "", comment: "original",
  image: {assetId: "a".repeat(64), captureId: id(1), mediaType: "image/png", byteSize: 10}};
const placement = () => ({input: {inputId: id(2), text: "original"}, index: 0, preceding: [], following: []});
function fixture(initial?: string) {
  const rows = new Map<string, string>(initial ? [[storageKey, initial]] : []); let fail = false;
  const storage: Storage = {get length() {return rows.size;}, key: index => [...rows.keys()][index] ?? null, getItem: vi.fn(key => rows.get(key) ?? null),
    setItem: vi.fn((key, value) => {if (fail) throw new Error("quota"); rows.set(key, value);}), removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
  const persistence = localConversationDraftPersistence(storage), open = () => new ConversationDraftStore(persistence);
  return {rows, storage, persistence, open, failSave: (value: boolean) => {fail = value;}};
}
function port(storage: Storage, flush: () => Promise<void>, retry = async () => {}) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false, flush, retry,
    reset: async () => {}, subscribe: () => () => {}, close: () => {}});
}
it("loads content and revision from one snapshot and consumes only that exact revision after a cold reopen", async () => {
  const f = fixture(), first = f.open(); first.update(key, "original"); const token = first.captureAcknowledgmentToken(key, "original")!; await first.flush();
  expect(token).toMatchObject({schemaVersion: 1, originKey: key}); vi.mocked(f.storage.getItem).mockClear();
  const reopened = f.open(); expect(f.storage.getItem).toHaveBeenCalledTimes(1);
  expect(reopened.captureAcknowledgmentToken(key, "original")).toEqual(token);
  expect(await reopened.consumeAcknowledgment(token, scope)).toBe("cleared"); expect(reopened.read(key)).toBe(""); expect(f.open().read(key)).toBe("");
  reopened.update(key, "original"); expect(await reopened.consumeAcknowledgment(token, scope)).toBe("preserved"); expect(reopened.read(key)).toBe("original");
});
it("same text no-op keeps a version, but explicit structured restore, edit-away-and-back and clear/retype never reuse it", async () => {
  const f = fixture(), store = f.open(); store.update(key, "original"); const original = store.captureAcknowledgmentToken(key, "original")!;
  store.update(key, "original"); store.updateStructured(key, store.readStructured(key)); expect(store.captureAcknowledgmentToken(key, "original")).toEqual(original);
  store.updateStructured(key, structuredClone(store.readStructured(key))); expect(store.captureAcknowledgmentToken(key, "original")?.revision).not.toBe(original.revision);
  expect(await store.consumeAcknowledgment(original, scope)).toBe("preserved");
  const restored = store.captureAcknowledgmentToken(key, "original")!; store.update(key, "changed"); store.update(key, "original");
  expect(await store.consumeAcknowledgment(restored, scope)).toBe("preserved");
  const beforeClear = store.captureAcknowledgmentToken(key, "original")!; store.update(key, ""); store.update(key, "original");
  expect(await store.consumeAcknowledgment(beforeClear, scope)).toBe("preserved"); expect(f.open().read(key)).toBe("original");
});
it("binding-only, evidence-only and unbound-reference edits rotate the complete draft revision", async () => {
  const f = fixture(), store = f.open(); store.updateStructured(key, insertAgentReference({text: "original ", bindings: []}, skill));
  const beforeBinding = store.captureAcknowledgmentToken(key, store.read(key).trim())!, original = store.readStructured(key);
  store.updateStructured(key, {...original, bindings: original.bindings.map(binding => ({...binding, input: {...skill, identity: {...skill.identity, revision: "v2"}}}))});
  expect(store.read(key)).toBe(original.text); expect(await store.consumeAcknowledgment(beforeBinding, scope)).toBe("preserved");
  const beforeEvidence = store.captureAcknowledgmentToken(key, store.read(key).trim())!; store.updateStructured(key, {...store.readStructured(key), evidence: [annotation]});
  expect(await store.consumeAcknowledgment(beforeEvidence, scope)).toBe("preserved");
  const beforeUnbound = store.captureAcknowledgmentToken(key, store.read(key).trim())!;
  store.updateStructured(key, {...store.readStructured(key), unboundReferences: [skill]});
  expect(await store.consumeAcknowledgment(beforeUnbound, scope)).toBe("preserved");
});
it("pending adoption preserves provenance across restart without giving the token access to another host or conversation", async () => {
  const f = fixture(), store = f.open(), pending = conversationDraftKey({...scope, conversationId: null});
  store.update(pending, "original"); const token = store.captureAcknowledgmentToken(pending, "original")!; store.adoptPending(scope, scope.conversationId!); await store.flush();
  const reopened = f.open();
  for (const other of [{...scope, conversationId: "other-chat"}, {...scope, hostId: "other-host"}, {...scope, hostKind: "note" as const}]) {
    const otherKey = conversationDraftKey(other); reopened.update(otherKey, "original");
    expect(await reopened.consumeAcknowledgment(token, other)).toBe("preserved"); expect(reopened.read(otherKey)).toBe("original");
  }
  expect(await reopened.consumeAcknowledgment(token, scope)).toBe("cleared"); expect(reopened.read(pending)).toBe("");
});
it.each([false, true])("a delayed ACK cannot strip a same-I newly restored queue placement (durable=%s)", async durable => {
  const f = fixture(), store = f.open(); store.restoreQueue(key, placement()); const token = store.captureAcknowledgmentToken(key, "original")!, closure = store.captureAcknowledgment(key, "original");
  store.restoreQueue(key, placement()); const next = store.captureAcknowledgmentToken(key, "original")!;
  expect(next.revision).not.toBe(token.revision); expect(next.sourceQueueRevision).not.toBe(token.sourceQueueRevision);
  if (durable) await f.open().consumeAcknowledgment(token, scope); else closure();
  expect(f.open().readStructured(key).queuePlacement?.input.inputId).toBe(id(2)); expect(store.readStructured(key).queuePlacement?.input.inputId).toBe(id(2));
});
it.each([false, true])("text edits retain queue origin, so only the accepted original slot is removed while the later draft remains (durable=%s)", async durable => {
  const f = fixture(), store = f.open(); store.restoreQueue(key, placement()); const token = store.captureAcknowledgmentToken(key, "original")!, closure = store.captureAcknowledgment(key, "original");
  store.update(key, "later text"); expect(store.captureAcknowledgmentToken(key, "later text")?.sourceQueueRevision).toBe(token.sourceQueueRevision);
  const target = durable ? f.open() : store;
  if (durable) expect(await target.consumeAcknowledgment(token, scope)).toBe("preserved"); else closure();
  expect(target.read(key)).toBe("later text"); expect(target.readStructured(key).queuePlacement).toBeUndefined();
});
it("fresh capture may stamp an unchanged v1 draft, but loading never manufactures old ACK or queue-origin authority", async () => {
  const bytes = JSON.stringify({version: 1, drafts: [[key, "original", [], placement()]]}), f = fixture(bytes), store = f.open();
  expect(f.rows.get(storageKey)).toBe(bytes); expect(f.persistence.load().get(key)?.revision).toBeNull();
  f.failSave(true); expect(store.captureAcknowledgmentToken(key, "original")).toBeNull(); expect(f.rows.get(storageKey)).toBe(bytes);
  f.failSave(false); const token = store.captureAcknowledgmentToken(key, "original")!;
  expect(token.sourceInputId).toBeUndefined(); expect(token.sourceQueueRevision).toBeUndefined(); expect(JSON.parse(f.rows.get(storageKey)!).version).toBe(2);
  store.update(key, "later"); expect(await store.consumeAcknowledgment(token, scope)).toBe("preserved"); expect(store.readStructured(key).queuePlacement).toBeDefined();
});
it("consuming an accepted queue slot removes an otherwise empty draft instead of persisting an empty revision", async () => {
  const f = fixture(), store = f.open(); store.restoreQueue(key, placement()); const token = store.captureAcknowledgmentToken(key, "original")!;
  store.update(key, ""); expect(await store.consumeAcknowledgment(token, scope)).toBe("cleared");
  expect(f.persistence.load().has(key)).toBe(false); expect(store.readStructured(key)).toEqual({text: "", bindings: []});
});
it("setItem failure preserves the draft and the same token can finish after retry, without clearing a later identical draft", async () => {
  const f = fixture(), store = f.open(); store.update(key, "original"); const token = store.captureAcknowledgmentToken(key, "original")!;
  f.failSave(true); await expect(store.consumeAcknowledgment(token, scope)).rejects.toThrow("AGENT_DRAFT_PERSISTENCE_FAILED"); expect(store.read(key)).toBe("original");
  f.failSave(false); await store.retryPersistence(); expect(await store.consumeAcknowledgment(token, scope)).toBe("cleared");
  store.update(key, "original"); expect(await store.consumeAcknowledgment(token, scope)).toBe("preserved"); expect(f.open().read(key)).toBe("original");
});
it("a failed deletion flush retains evidence; exact retry flushes the current snapshot and never clears or releases a newer draft", async () => {
  const f = fixture(), store = f.open(); store.updateStructured(key, {text: "original", bindings: [], evidence: [annotation]}); const token = store.captureAcknowledgmentToken(key, "original")!;
  let fail = true; port(f.storage, async () => {if (fail) throw new Error("disk");});
  await expect(store.consumeAcknowledgment(token, scope)).rejects.toThrow("AGENT_DRAFT_PERSISTENCE_FAILED"); expect(releaseRemovedEvidence).not.toHaveBeenCalled();
  store.updateStructured(key, {text: "original", bindings: [], evidence: [annotation]}); fail = false;
  expect(await store.consumeAcknowledgment(token, scope)).toBe("preserved");
  expect(store.read(key)).toBe("original"); expect(f.open().readStructured(key).evidence).toEqual([annotation]);
  expect(releaseRemovedEvidence).toHaveBeenCalledWith([annotation], [annotation]);
});
it("an explicit flush retry releases the removed capture only after deletion is durable and does not mint another version", async () => {
  const f = fixture(), store = f.open(); store.updateStructured(key, {text: "original", bindings: [], evidence: [annotation]});
  const token = store.captureAcknowledgmentToken(key, "original")!; let fail = true;
  const retry = vi.fn(async () => {fail = false;}); port(f.storage, async () => {if (fail) throw new Error("disk");}, retry);
  await expect(store.consumeAcknowledgment(token, scope)).rejects.toThrow(); expect(releaseRemovedEvidence).not.toHaveBeenCalled();
  await store.retryPersistence(); expect(retry).toHaveBeenCalledOnce();
  expect(releaseRemovedEvidence).toHaveBeenCalledExactlyOnceWith([annotation], []); expect(f.persistence.load().has(key)).toBe(false);
  expect(await store.consumeAcknowledgment(token, scope)).toBe("cleared"); expect(releaseRemovedEvidence).toHaveBeenCalledOnce();
});
it("an earlier flush cannot release evidence belonging to a later, still unconfirmed deletion", async () => {
  const f = fixture(), store = f.open(), otherKey = conversationDraftKey({...scope, conversationId: "other"});
  store.updateStructured(key, {text: "first", bindings: [], evidence: [annotation]});
  const second = {...annotation, image: {...annotation.image!, captureId: id(3)}};
  store.updateStructured(otherKey, {text: "second", bindings: [], evidence: [second]});
  const gates: (() => void)[] = []; port(f.storage, () => new Promise<void>(resolve => {gates.push(resolve);}));
  const first = store.consumeAcknowledgment(store.captureAcknowledgmentToken(key, "first")!, scope);
  const later = store.consumeAcknowledgment(store.captureAcknowledgmentToken(otherKey, "second")!, {...scope, conversationId: "other"});
  gates[0]!(); await first; expect(releaseRemovedEvidence).toHaveBeenCalledExactlyOnceWith([annotation], []);
  gates[1]!(); await later; expect(releaseRemovedEvidence).toHaveBeenLastCalledWith([second], []);
});
it("token decoding rejects partial queue origins and malformed or extra fields", () => {
  const valid = {schemaVersion: 1, originKey: key, revision: id(1)};
  expect(decodeConversationDraftAcknowledgmentToken(valid)).toEqual(valid);
  for (const value of [{...valid, sourceInputId: id(2)}, {...valid, sourceQueueRevision: id(3)}, {...valid, revision: "wrong"}, {...valid, originKey: "private-path"}, {...valid, text: "body"}])
    expect(() => decodeConversationDraftAcknowledgmentToken(value)).toThrow();
});
