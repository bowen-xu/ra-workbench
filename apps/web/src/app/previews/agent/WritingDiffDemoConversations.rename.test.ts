import { expect, it } from "vitest";
import { WritingDiffDemoConversations } from "./WritingDiffDemoConversations";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";

async function fixture() {
  const manuscriptId = crypto.randomUUID(), id = crypto.randomUUID();
  const storage = new WritingDiffDemoStorage(manuscriptId, []), agent = new WritingDiffDemoAgentStorage(storage, undefined);
  await agent.initialized;
  return {id, agent, conversations: new WritingDiffDemoConversations(manuscriptId, agent, id), manuscriptId};
}
it("restores rename proofs with the writing mock snapshot and preserves later metadata", async () => {
  const f = await fixture();
  try {
    const request = {operationId: crypto.randomUUID(), id: f.id, expectedTitle: "完善摘要", title: "B"};
    const proof = await f.conversations.catalog.rename(request);
    await f.conversations.catalog.rename({...request, operationId: crypto.randomUUID(), expectedTitle: "B", title: "完善摘要"});
    await f.conversations.archives.setArchived({operationId: crypto.randomUUID(), id: f.id, archived: true, expectedWatermark: (await f.conversations.metadata.read({id: f.id})).durableCount});
    const restored = new WritingDiffDemoConversations(f.manuscriptId, f.agent, f.id);
    restored.restore(f.conversations.snapshot());
    expect(await restored.catalog.rename(request)).toEqual({...proof, replayed: true});
    expect(await restored.metadata.read({id: f.id})).toMatchObject({title: "完善摘要", archived: true});
    expect((await restored.catalog.renameStatus(request)).status.type).toBe("committed");
  } finally { f.agent.history.dispose(); }
});
it("checks CAS again at the mock commit boundary when two asynchronous requests race", async () => {
  const f = await fixture();
  try {
    const request = {operationId: crypto.randomUUID(), id: f.id, expectedTitle: "完善摘要", title: "B"};
    const outcomes = await Promise.allSettled([f.conversations.catalog.rename(request),
      f.conversations.catalog.rename({...request, operationId: crypto.randomUUID(), title: "C"})]);
    expect(outcomes.map(result => result.status)).toEqual(["fulfilled", "rejected"]);
    expect(await f.conversations.metadata.read({id: f.id})).toMatchObject({title: "B"});
    expect((await f.conversations.catalog.renameStatus(request)).status.type).toBe("committed");
  } finally { f.agent.history.dispose(); }
});
