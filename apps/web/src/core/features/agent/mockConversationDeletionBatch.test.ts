import { expect, it, vi } from "vitest";
import { mockConversationDeletionBatch, type MockConversationDeletionPlan } from "./mockConversationDeletionBatch";

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const requests = [1, 2, 3].map(n => ({schemaVersion: 1 as const, requestId: uuid(n + 10), conversationId: uuid(n),
  binding: null, expectedTitle: `Chat ${n}`, expectedDurableCount: 0, force: true}));

it("prepares every item before publishing accepted deletions and retains rejected items", async () => {
  const events: string[] = [];
  const result = await mockConversationDeletionBatch(requests, async request => {
    events.push(`prepare:${request.expectedTitle}`);
    if (request === requests[1] || request.expectedTitle === "Chat 2") throw "busy";
    return {validate: () => events.push(`validate:${request.expectedTitle}`), publish: () => events.push(`publish:${request.expectedTitle}`),
      rollback: () => events.push(`rollback:${request.expectedTitle}`), finish: () => events.push(`finish:${request.expectedTitle}`)};
  });
  expect(events).toEqual(["prepare:Chat 1", "prepare:Chat 2", "prepare:Chat 3", "validate:Chat 1", "validate:Chat 3",
    "publish:Chat 1", "publish:Chat 3", "finish:Chat 1", "finish:Chat 3"]);
  expect(result.results.map(item => item.status)).toEqual(["deleted", "failed", "deleted"]);
});

it("rolls back accepted members when publication fails before the complete batch is visible", async () => {
  const state = new Set<string>();
  const plan = (name: string, fail: boolean): MockConversationDeletionPlan => ({validate: vi.fn(),
    publish: () => {state.add(name); if (fail) throw Error("storage conflict");}, rollback: () => {state.delete(name);}, finish: vi.fn()});
  await expect(mockConversationDeletionBatch(requests.slice(0, 2), async request => plan(request.conversationId, request.conversationId === requests[1]!.conversationId)))
    .rejects.toThrow("storage conflict");
  expect(state.size).toBe(0);
});
