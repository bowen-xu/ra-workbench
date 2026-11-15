import {expect, it, vi} from "vitest";
import {MockIndependentReceipts} from "./mockIndependentReceipts";
import {decodeAgentEvent} from "@research-assistant/core/agent";
const id = (n: number) => `abcdef00-1234-4678-abcd-${String(n).padStart(12, "0")}`;
function storage(): Storage {
  const rows = new Map<string, string>(); return {get length() {return rows.size;}, key: n => [...rows.keys()][n] ?? null,
    getItem: key => rows.get(key) ?? null, setItem: (key, value) => {rows.set(key, value);}, removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
}
const request = {conversationId: id(1), requestId: id(2), text: "Question", resources: []};
const preview = {schemaVersion: 1 as const, conversationId: id(1), requestId: id(2), turnId: id(3), contextSnapshotId: id(4),
  input: {text: "Question", model: {providerId: "fixture", modelId: "mock", revision: null}, reasoningEffort: null, context: []}};
const startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(4), queue: null, resumeQueue: false};
it("retains the original guarded result across reopening and rejects changed intent", async () => {
  const data = storage(), prepare = vi.fn().mockResolvedValue(preview), start = vi.fn(), open = () => new MockIndependentReceipts(data, prepare, start, () => []);
  const first = open(), epoch = first.epoch;
  expect((await first.status.read(request)).status.type).toBe("unconfirmed");
  const original = await first.proven.prepare(request, {expectedEpoch: epoch});
  expect(original.proof.replayed).toBe(false);
  const reopened = open(), replay = await reopened.proven.prepare(request, {expectedEpoch: epoch});
  expect(replay).toMatchObject({proof: {epoch, replayed: true}, result: preview}); expect(prepare).toHaveBeenCalledOnce();
  await expect(reopened.proven.prepare({...request, text: "Changed"}, {expectedEpoch: epoch})).rejects.toThrow();
  await expect(reopened.status.read(request, {expectedEpoch: id(9)})).rejects.toThrow();
  expect((await reopened.guardedStart.read(startRequest, epoch)).status.type).toBe("unconfirmed");
  await expect(reopened.guardedStart.start(startRequest, id(9))).rejects.toThrow(); expect(start).not.toHaveBeenCalled();
});
it("serializes concurrent identical preparations and retains a single frozen result", async () => {
  const data = storage(); let release!: () => void;
  const prepare = vi.fn(async () => {await new Promise<void>(resolve => {release = resolve;}); return preview;});
  const owner = new MockIndependentReceipts(data, prepare, vi.fn(), () => []);
  const first = owner.proven.prepare(request, {expectedEpoch: owner.epoch});
  const second = owner.proven.prepare(request, {expectedEpoch: owner.epoch});
  release(); const [one, two] = await Promise.all([first, second]);
  expect(prepare).toHaveBeenCalledOnce(); expect(one.proof.replayed).toBe(false); expect(two.proof.replayed).toBe(true); expect(two.result).toEqual(one.result);
});
it("observes actual start history and keeps an absent history unconfirmed", async () => {
  const data = storage(), events: ReturnType<typeof decodeAgentEvent>[] = [];
  const start = vi.fn(async () => {
    events.push(decodeAgentEvent({schemaVersion: 1, conversationId: id(1), seq: 1, timestampMs: 0, requestId: id(2),
      payload: {type: "turnStarted", turnId: id(3), contextSnapshotId: id(4), model: preview.input.model}}));
    return {schemaVersion: 1 as const, conversationId: id(1), requestId: id(2), turnId: id(3), alreadyStarted: false, outcome: null};
  });
  const owner = new MockIndependentReceipts(data, vi.fn().mockResolvedValue(preview), start, () => events);
  await owner.proven.prepare(request, {expectedEpoch: owner.epoch});
  expect((await owner.guardedStart.read(startRequest, owner.epoch)).status.type).toBe("unconfirmed");
  await owner.guardedStart.start(startRequest, owner.epoch);
  expect((await owner.guardedStart.read(startRequest, owner.epoch)).status).toMatchObject({type: "started", result: {turnId: id(3), alreadyStarted: true}});
});
it("does not fabricate a receipt when mock persistence fails", async () => {
  const data = storage(), write = data.setItem.bind(data);
  const owner = new MockIndependentReceipts(data, vi.fn().mockResolvedValue(preview), vi.fn(), () => []);
  data.setItem = (key, value) => {if (key === "independent-prepare-receipts") throw new Error("storage unavailable"); write(key, value);};
  await expect(owner.proven.prepare(request, {expectedEpoch: owner.epoch})).rejects.toThrow("storage unavailable");
  expect((await owner.status.read(request, {expectedEpoch: owner.epoch})).status.type).toBe("unconfirmed");
});
it("reports an admitted queue entry without inventing a started turn", async () => {
  const data = storage(), queued = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    frozen = {...preview, queue: {inputId: id(6), expectedRevision: 5}};
  const owner = new MockIndependentReceipts(data, vi.fn().mockResolvedValue(frozen), vi.fn(), () => [], undefined, value => value.queue?.inputId === id(6));
  await owner.proven.prepare(queued, {expectedEpoch: owner.epoch});
  const status = await owner.guardedStart.read({...startRequest, queue: frozen.queue}, owner.epoch);
  expect(status.status).toMatchObject({type: "admitted", preview: frozen});
});
