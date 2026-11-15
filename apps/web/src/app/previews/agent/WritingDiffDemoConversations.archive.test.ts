import { expect, it, vi } from "vitest";
import { WritingDiffDemoConversations } from "./WritingDiffDemoConversations";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";

it("restores through the metadata path when history reading is unavailable, preserving proof on snapshot reopen", async () => {
  const manuscriptId = crypto.randomUUID(), id = crypto.randomUUID();
  const storage = new WritingDiffDemoStorage(manuscriptId, []), agent = new WritingDiffDemoAgentStorage(storage, undefined);
  await agent.initialized;
  const conversations = new WritingDiffDemoConversations(manuscriptId, agent, id);
  try {
    const archive = {operationId: crypto.randomUUID(), id, archived: true, expectedWatermark: 0};
    const proof = await conversations.archives.setArchived(archive);
    const history = vi.spyOn(agent.gateways.delivery.sessions, "open").mockRejectedValue(new Error("history unavailable"));
    const restore = {operationId: crypto.randomUUID(), id, archived: false, expectedWatermark: null};
    const restoredProof = await conversations.archives.setArchived(restore);
    expect(history).not.toHaveBeenCalled();
    const reopened = new WritingDiffDemoConversations(manuscriptId, agent, id);
    reopened.restore(conversations.snapshot());
    expect(await reopened.archives.setArchived(archive)).toEqual({...proof, replayed: true});
    expect((await reopened.archives.archiveStatus(restore)).status).toEqual({type: "committed", proof: {...restoredProof, replayed: true}});
    expect(reopened.snapshot().records.get(id)?.archived).toBe(false);
    expect(history).not.toHaveBeenCalled(); history.mockRestore();
  } finally { agent.history.dispose(); }
});
