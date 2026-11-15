// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {createMockStructuredResources} from "./mockStructuredResources";
import {decodeStructuredState} from "./mockStructuredStateCodec";
import {registerStoragePersistence, type StoragePersistence} from "../../../core/runtime/storagePersistence";

function storage(): Storage {
  const values = new Map<string, string>();
  return {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);}, clear: () => values.clear()};
}
const body = (text: string) => ({kind: "paper_overview_unit_v1", schemaVersion: 1,
  fields: [{field: "question", statements: [{id: "q", text, evidenceItemIds: []}]}]});
afterEach(() => vi.restoreAllMocks());

it("atomically persists HBS S/B and original request receipts across reopen without accepting a save", async () => {
  const store = storage(); let gateway = createMockStructuredResources(store);
  const reading = (await gateway.list("paper")).find(resource => resource.kind === "structuredReading")!;
  const written = (await gateway.write({resourceId: reading.id, expectedRevision: 0, requestId: "Agent first",
    content: JSON.stringify(body("Agent candidate")), editorKind: "agent"})).resource;
  const request = {id: "hbs:save", expectedVersion: written.revision, saved: body("my edited candidate"), baseline: written.reviewBaseline};
  const save = vi.spyOn(store, "setItem"); let notifications = 0;
  const stop = await gateway.subscribeChanges!(() => {
    notifications++;
    const durable = decodeStructuredState<{hbsReceipts: Record<string, unknown>; resources: typeof written[]}>(store.getItem("mock-structured-resources:v1")!);
    expect(durable.hbsReceipts[JSON.stringify([reading.id, request.id])]).toBeDefined();
    expect(durable.resources.find(resource => resource.id === reading.id)?.revision).toBe(written.revision + 1);
  });
  const receipt = await gateway.hbs.commit(reading.id, request);
  expect(save).toHaveBeenCalledTimes(1); expect(notifications).toBe(1); stop();
  const current = await gateway.get(reading.id);
  expect(current.content).toEqual(request.saved); expect(current.reviewBaseline).toEqual(written.reviewBaseline);
  expect(current.pendingFieldIds).toContain("question");
  gateway = createMockStructuredResources(store); save.mockClear();
  expect(await gateway.hbs.commit(reading.id, request)).toEqual(receipt);
  expect(save).not.toHaveBeenCalled();
  await expect(gateway.hbs.commit(reading.id, {...request, saved: body("different request")})).rejects.toMatchObject({code: "MODEL_REQUEST_ID_REUSED", requestId: request.id});
  expect(await gateway.hbs.commit(reading.id, {...request, id: "stale"})).toMatchObject({type: "conflict", code: "MODEL_VERSION_CONFLICT", snapshot: {version: current.revision}});
});

it("preserves typed S-prime with an Agent repair draft and rejects overwriting the formal resource", async () => {
  const store = storage(); let gateway = createMockStructuredResources(store);
  const reading = (await gateway.list("paper")).find(resource => resource.kind === "structuredReading")!;
  const failed = await gateway.write({resourceId: reading.id, expectedRevision: 0, requestId: "invalid Agent",
    content: "{broken", editorKind: "agent", conversationId: "agent"});
  const checkpoint = {format: 1, kind: "structured", observed: {version: 0, saved: null, baseline: null},
    content: body("preserved"), ancestor: null, reviewAncestor: null, baselineAncestor: null, ordinary: []};
  expect(await gateway.hbs.savePreservation(reading.id, checkpoint)).toEqual({revision: 1});
  await expect(gateway.hbs.commit(reading.id, {id: "blocked", expectedVersion: 0, saved: body("overwrite"), baseline: null})).rejects.toMatchObject({code: "MODEL_DRAFT_PENDING"});
  expect((await gateway.get(reading.id)).draft).toEqual(failed.draft);
  await expect(gateway.hbs.savePreservation(reading.id, {...checkpoint, content: ["wrong type"]})).rejects.toMatchObject({code: "MODEL_RECOVERY_INVALID"});
  gateway = createMockStructuredResources(store);
  expect(await gateway.hbs.readPreservation(reading.id)).toEqual(checkpoint);
  expect((await gateway.get(reading.id)).content).toBeNull();
});

it("does not mutate S/B or claim a receipt after a synchronous storage failure", async () => {
  const store = storage(), gateway = createMockStructuredResources(store);
  const reading = (await gateway.list("paper")).find(resource => resource.kind === "structuredReading")!;
  const request = {id: "retry-after-failure", expectedVersion: 0, saved: body("new"), baseline: null};
  vi.spyOn(store, "setItem").mockImplementationOnce(() => {throw new Error("storage blocked");});
  await expect(gateway.hbs.commit(reading.id, request)).rejects.toThrow("storage blocked");
  expect((await gateway.get(reading.id)).revision).toBe(0);
  await expect(gateway.hbs.commit(reading.id, request)).resolves.toMatchObject({type: "receipt", snapshot: {version: 1}});
});

it("waits for durability again when retrying the same unknown request after an IndexedDB flush failure", async () => {
  const store = storage(), gateway = createMockStructuredResources(store);
  const reading = (await gateway.list("paper")).find(resource => resource.kind === "structuredReading")!;
  const flush = vi.fn<StoragePersistence["flush"]>().mockRejectedValueOnce(new Error("IndexedDB blocked")).mockResolvedValue(undefined);
  registerStoragePersistence(store, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const request = {id: "unknown", expectedVersion: 0, saved: body("once"), baseline: null};
  const save = vi.spyOn(store, "setItem");
  await expect(gateway.hbs.commit(reading.id, request)).rejects.toThrow("IndexedDB blocked");
  const result = await gateway.hbs.commit(reading.id, request);
  expect(result).toMatchObject({type: "receipt", snapshot: {version: 1}});
  expect(save).toHaveBeenCalledTimes(1); expect(flush).toHaveBeenCalledTimes(2);
});

it("retains protected-field semantics and keeps acceptance separate from verification", async () => {
  const store = storage(), gateway = createMockStructuredResources(store);
  const reading = (await gateway.list("paper")).find(resource => resource.kind === "structuredReading")!;
  const initial = (await gateway.write({resourceId: reading.id, expectedRevision: 0, requestId: "human", content: JSON.stringify(body("before")), editorKind: "human"})).resource;
  const verified = await gateway.review({resourceId: reading.id, expectedRevision: initial.revision, requestId: "verify", action: "verify", fieldIds: ["question"]});
  await gateway.hbs.commit(reading.id, {id: "save-edit", expectedVersion: verified.revision, saved: body("after"), baseline: body("before")});
  const saved = await gateway.get(reading.id);
  expect(saved.reviewedFieldIds).toEqual([]); expect(saved.pendingFieldIds).toContain("question");
  const skipped = await gateway.review({resourceId: reading.id, expectedRevision: saved.revision, requestId: "keep-unreviewed", action: "keep", scope: "unreviewed"});
  expect(skipped.pendingFieldIds).toContain("question");
  await gateway.hbs.commit(reading.id, {id: "keep-edit", expectedVersion: skipped.revision, saved: body("after"), baseline: body("after")});
  const kept = await gateway.get(reading.id);
  expect(kept.reviewedFieldIds).toEqual([]); expect(kept.pendingFieldIds).toEqual([]);
  const generated = (await gateway.write({resourceId: reading.id, expectedRevision: kept.revision, requestId: "Agent later", content: JSON.stringify(body("next")), editorKind: "agent"})).resource;
  const accepted = await gateway.review({resourceId: reading.id, expectedRevision: generated.revision, requestId: "keep-later", action: "keep", scope: "unreviewed"});
  expect(accepted.pendingFieldIds).toEqual([]);
});
