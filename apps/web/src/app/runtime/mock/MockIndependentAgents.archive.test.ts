// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { MockIndependentAgents } from "./MockIndependentAgents";

function fixture() {
  const values = new Map<string, string>();
  const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  const id = crypto.randomUUID();
  storage.setItem("independent-conversations", JSON.stringify([[id, {conversationId: id, title: "合成归档"}]]));
  const open = () => new MockIndependentAgents(storage, {delayMs: 0, nextFailure: null,
    beforeOperation: async () => undefined, configure() {}, reset() {}, dispose() {}});
  return {values, id, open};
}

it("reopens historical archive proof without reverting a later restore or writing on status/replay", async () => {
  const f = fixture(), first = f.open();
  const request = {operationId: crypto.randomUUID(), id: f.id, archived: true, expectedWatermark: 0};
  const proof = await first.archives.setArchived(request);
  await first.archives.setArchived({operationId: crypto.randomUUID(), id: f.id, archived: false, expectedWatermark: null});
  first.dispose();
  const reopened = f.open();
  try {
    const before = new Map(f.values);
    expect(await reopened.archives.setArchived(request)).toEqual({...proof, replayed: true});
    expect((await reopened.archives.archiveStatus(request)).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
    expect((await reopened.archives.archiveStatus({...request, operationId: crypto.randomUUID()})).status.type).toBe("noSqlReceipt");
    expect(f.values).toEqual(before);
    expect(await reopened.metadata.read({id: f.id})).toMatchObject({archived: false});
  } finally { reopened.dispose(); }
});

it("retains proof after deletion without resurrection and rejects operation identity reuse", async () => {
  const f = fixture(), agents = f.open();
  try {
    const request = {operationId: crypto.randomUUID(), id: f.id, archived: true, expectedWatermark: 0};
    const proof = await agents.archives.setArchived(request);
    await agents.deletions.delete({schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: f.id,
      binding: null, expectedTitle: "合成归档", expectedDurableCount: 0, force: true});
    const before = new Map(f.values);
    expect(await agents.archives.setArchived(request)).toEqual({...proof, replayed: true});
    expect((await agents.archives.archiveStatus(request)).status.type).toBe("committed");
    await expect(agents.metadata.read({id: f.id})).rejects.toBe("notFound");
    for (const changed of [{...request, expectedWatermark: 1}, {...request, archived: false, expectedWatermark: null},
      {...request, id: crypto.randomUUID()}]) {
      await expect(agents.archives.setArchived(changed)).rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    }
    await expect(agents.catalog.rename({operationId: request.operationId, id: f.id, expectedTitle: "合成归档", title: "别名"}))
      .rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    expect(f.values).toEqual(before);
  } finally { agents.dispose(); }
});

it("rejects stale history watermarks without minting a receipt and freezes input before asynchronous reads", async () => {
  const f = fixture(), agents = f.open();
  try {
    const stale = {operationId: crypto.randomUUID(), id: f.id, archived: true, expectedWatermark: 1};
    await expect(agents.archives.setArchived(stale)).rejects.toBe("conflict");
    expect((await agents.archives.archiveStatus(stale)).status.type).toBe("noSqlReceipt");
    const request = {...stale, operationId: crypto.randomUUID(), expectedWatermark: 0};
    const original = {...request}, pending = agents.archives.setArchived(request);
    request.archived = false; request.expectedWatermark = 19;
    const proof = await pending;
    expect((await agents.archives.archiveStatus(original)).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
    expect(await agents.metadata.read({id: f.id})).toMatchObject({archived: true});
  } finally { agents.dispose(); }
});
