// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { MockIndependentAgents } from "./MockIndependentAgents";

function fixture(title = "A") {
  const values = new Map<string, string>();
  const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  const id = crypto.randomUUID(); storage.setItem("independent-conversations", JSON.stringify([[id, {conversationId: id, title}]]));
  const open = () => new MockIndependentAgents(storage, {delayMs: 0, nextFailure: null,
    beforeOperation: async () => undefined, configure() {}, reset() {}, dispose() {}});
  return {values, id, open};
}
it("persists the original rename proof across mock refresh without overwriting a later title or archive", async () => {
  const f = fixture(), first = f.open();
  const request = {operationId: crypto.randomUUID(), id: f.id, expectedTitle: "A", title: "B"};
  const proof = await first.catalog.rename(request);
  await first.catalog.rename({...request, operationId: crypto.randomUUID(), expectedTitle: "B", title: "A"});
  await first.archives.setArchived({operationId: crypto.randomUUID(), id: f.id, archived: true, expectedWatermark: (await first.metadata.read({id: f.id})).durableCount}); first.dispose();
  const restored = f.open();
  try {
    expect(await restored.catalog.rename(request)).toEqual({...proof, replayed: true});
    expect(await restored.metadata.read({id: f.id})).toMatchObject({title: "A", archived: true});
    const before = new Map(f.values);
    expect((await restored.catalog.renameStatus(request)).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
    expect(f.values).toEqual(before);
  } finally { restored.dispose(); }
});
it("retains rename evidence after deletion and rejects changed original text before touching current metadata", async () => {
  const f = fixture(), agents = f.open();
  try {
    const request = {operationId: crypto.randomUUID(), id: f.id, expectedTitle: "A", title: " B "};
    const proof = await agents.catalog.rename(request);
    await agents.archives.setArchived({operationId: crypto.randomUUID(), id: f.id, archived: true, expectedWatermark: (await agents.metadata.read({id: f.id})).durableCount});
    await agents.deletions.delete({schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: f.id, binding: null,
      expectedTitle: "B", expectedDurableCount: 0, force: true});
    await expect(agents.metadata.read({id: f.id})).rejects.toBe("notFound");
    expect(agents.hasRenameOperation(request.operationId)).toBe(true);
    expect(await agents.catalog.rename(request)).toEqual({...proof, replayed: true});
    expect((await agents.catalog.renameStatus(request)).status.type).toBe("committed");
    await expect(agents.catalog.rename({...request, title: "B"})).rejects.toMatchObject({code: "storage",
      reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
  } finally { agents.dispose(); }
});
it("renames a long legacy title using its original opaque CAS text", async () => {
  const legacy = "原始历史标题".repeat(100), f = fixture(legacy), agents = f.open();
  try {
    const request = {operationId: crypto.randomUUID(), id: f.id, expectedTitle: legacy, title: "新名"};
    expect((await agents.catalog.rename(request)).operationId).toBe(request.operationId);
    expect((await agents.metadata.read({id: f.id})).title).toBe("新名");
    expect((await agents.catalog.renameStatus(request)).status.type).toBe("committed");
  } finally { agents.dispose(); }
});
