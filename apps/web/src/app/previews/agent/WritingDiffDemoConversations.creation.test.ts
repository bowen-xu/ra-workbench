// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { WritingDiffDemoConversations } from "./WritingDiffDemoConversations";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";

async function fixture() {
  const manuscriptId = crypto.randomUUID(), firstId = crypto.randomUUID(); let available = true;
  const agent = new WritingDiffDemoAgentStorage(new WritingDiffDemoStorage(manuscriptId, []), undefined);
  await agent.initialized;
  const conversations = new WritingDiffDemoConversations(manuscriptId, agent, firstId, undefined, undefined, "manuscript", () => available);
  const request = {operationId: crypto.randomUUID(), id: crypto.randomUUID(), binding: {kind: "manuscript" as const, resourceId: manuscriptId}};
  return {agent, conversations, request, firstId, disappear: () => { available = false; }};
}
it("restores original creation proof without reverting later title or archive metadata", async () => {
  const f = await fixture();
  try {
    const proof = await f.conversations.creation.createBound(f.request);
    await f.conversations.catalog.rename({operationId: crypto.randomUUID(), id: f.request.id, expectedTitle: "新对话", title: "Later"});
    await f.conversations.archives.setArchived({operationId: crypto.randomUUID(), id: f.request.id, archived: true, expectedWatermark: (await f.conversations.metadata.read({id: f.request.id})).durableCount});
    const restored = new WritingDiffDemoConversations(f.request.binding.resourceId, f.agent, f.firstId);
    restored.restore(f.conversations.snapshot());
    expect(await restored.creation.createBound(f.request)).toEqual({...proof, replayed: true});
    expect(await restored.metadata.read({id: f.request.id, binding: f.request.binding})).toMatchObject({title: "Later", archived: true});
    expect((await restored.creation.createBoundStatus(f.request)).status.type).toBe("committed");
  } finally { f.agent.history.dispose(); }
});
it("accepts an existing C only with its exact binding and never adopts an unbound row", async () => {
  const f = await fixture();
  try {
    const exact = {...f.request, id: f.firstId};
    expect((await f.conversations.creation.createBound(exact)).replayed).toBe(false);
    const snapshot = f.conversations.snapshot(); snapshot.bindings.delete(f.firstId); f.conversations.restore(snapshot);
    await expect(f.conversations.creation.createBound({...exact, operationId: crypto.randomUUID()})).rejects.toBe("conflict");
    expect((await f.conversations.creation.createBoundStatus(exact)).status.type).toBe("committed");
  } finally { f.agent.history.dispose(); }
});
it("retains receipt tombstones after conversation deletion and checks resource liveness before replay", async () => {
  const f = await fixture();
  try {
    const proof = await f.conversations.creation.createBound(f.request);
    await f.conversations.archives.setArchived({operationId: crypto.randomUUID(), id: f.request.id, archived: true, expectedWatermark: (await f.conversations.metadata.read({id: f.request.id})).durableCount});
    await f.conversations.deletions.delete({schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: f.request.id,
      binding: f.request.binding, expectedTitle: "新对话", expectedDurableCount: 0, force: true});
    expect(await f.conversations.creation.createBound(f.request)).toEqual({...proof, replayed: true});
    await expect(f.conversations.creation.createBound({...f.request, operationId: crypto.randomUUID()})).rejects.toBe("notFound");
    f.disappear();
    await expect(f.conversations.creation.createBound(f.request)).rejects.toBe("notFound");
    expect((await f.conversations.creation.createBoundStatus(f.request)).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
    await expect(f.conversations.metadata.read({id: f.request.id})).rejects.toBe("notFound");
  } finally { f.agent.history.dispose(); }
});
