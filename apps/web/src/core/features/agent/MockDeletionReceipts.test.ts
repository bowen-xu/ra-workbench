import { expect, it, vi } from "vitest";
import { MockDeletionReceipts } from "./MockDeletionReceipts";
const epoch = "00000000-0000-4000-8000-000000000001", foreign = "00000000-0000-4000-8000-000000000002";
const request = {schemaVersion: 1 as const, requestId: "00000000-0000-4000-8000-000000000003",
  conversationId: "00000000-0000-4000-8000-000000000004", binding: null,
  expectedTitle: "Synthetic", expectedDurableCount: 2, force: true};
it("reopens exact managed evidence, rejects changed confirmations and fences even existing receipts", () => {
  const owner = new MockDeletionReceipts(() => epoch), mutate = vi.fn();
  expect(owner.status(request).status.type).toBe("noSqlReceipt");
  owner.validateWrite(request, {expectedEpoch: epoch}); owner.apply(request, mutate);
  const saved = owner.snapshot(), reopened = new MockDeletionReceipts(() => epoch); reopened.restore(saved);
  expect(reopened.status(request)).toEqual(owner.status(request));
  expect(reopened.replay(request)).toBe(true);
  expect(() => reopened.validateWrite(request, {expectedEpoch: foreign})).toThrow();
  for (const patch of [{force: false}, {expectedTitle: "Changed"}, {binding: {kind: "note" as const, resourceId: "other"}},
    {expectedDurableCount: 3}, {conversationId: foreign}]) expect(() => reopened.status({...request, ...patch})).toThrow();
  reopened.apply(request, mutate); expect(mutate).toHaveBeenCalledOnce();
  expect(reopened.snapshot()).toEqual(saved);
});
it("keeps legacy business receipts unconfirmed and never fabricates a managed proof on replay", () => {
  const owner = new MockDeletionReceipts(() => epoch), mutate = vi.fn();
  owner.restore(undefined, new Map([[request.requestId, JSON.stringify(request)]]));
  expect(owner.replay(request)).toBe(true); const saved = owner.snapshot();
  expect(owner.status(request).status).toMatchObject({type: "unconfirmed", reason: "legacy_sql_receipt_unavailable"});
  owner.apply(request, mutate); expect(mutate).not.toHaveBeenCalled(); expect(owner.snapshot()).toEqual(saved);
});
it("refuses foreign operation families before invoking mutation and rejects a foreign restored epoch", () => {
  const owner = new MockDeletionReceipts(() => epoch, id => id === request.requestId), mutate = vi.fn();
  expect(() => owner.apply(request, mutate)).toThrow(); expect(mutate).not.toHaveBeenCalled();
  expect(() => owner.restore({epoch: foreign, entries: new Map()})).toThrow();
});
