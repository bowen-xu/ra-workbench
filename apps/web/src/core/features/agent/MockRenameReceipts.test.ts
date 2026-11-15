// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import { MockRenameReceipts } from "./MockRenameReceipts";

const request = {operationId: "00000000-0000-0000-0000-000000000001", id: "00000000-0000-0000-0000-000000000002", expectedTitle: "A", title: "B"};

it("preserves A after A→B→A and an exact replay, including a restored fixture snapshot", () => {
  const receipts = new MockRenameReceipts(); let title = "A";
  const mutate = vi.fn((value: typeof request) => { if (title !== value.expectedTitle) throw "conflict"; title = value.title.trim(); });
  const first = receipts.apply(request, mutate);
  receipts.apply({...request, operationId: crypto.randomUUID(), expectedTitle: "B", title: "A"}, mutate);
  const restored = new MockRenameReceipts(); restored.restore(receipts.snapshot());
  expect(restored.apply(request, mutate)).toEqual({...first, replayed: true});
  expect(title).toBe("A"); expect(mutate).toHaveBeenCalledTimes(2);
  expect(restored.status(request).status).toEqual({type: "committed", proof: {...first, replayed: true}});
});
it("keeps historical proof without requiring a current conversation, and rejects any changed intent", () => {
  const receipts = new MockRenameReceipts(), mutate = vi.fn();
  receipts.apply(request, mutate);
  for (const patch of [{id: crypto.randomUUID()}, {title: " B "}, {expectedTitle: "other"}]) {
    expect(() => receipts.apply({...request, ...patch}, mutate)).toThrow();
    expect(() => receipts.status({...request, ...patch})).toThrow();
  }
  expect(mutate).toHaveBeenCalledTimes(1);
  expect(receipts.lookup(request)?.conversationId).toBe(request.id);
});
it("does not create a receipt when the fixture's CAS rejects and status itself changes no data", () => {
  const receipts = new MockRenameReceipts();
  const before = receipts.snapshot();
  expect(() => receipts.apply(request, () => { throw "conflict"; })).toThrow();
  expect(receipts.status(request).status.type).toBe("noSqlReceipt");
  expect(receipts.snapshot()).toEqual(before);
});
it("rejects corrupt persisted identity without partially replacing existing receipts", () => {
  const receipts = new MockRenameReceipts(); receipts.apply(request, () => undefined);
  const before = receipts.snapshot(), entries = new Map(before.entries);
  const original = entries.get(request.operationId)!;
  entries.set(request.operationId, {...original, proof: {...original.proof, conversationId: crypto.randomUUID()}});
  expect(() => receipts.restore({...before, entries})).toThrow("Invalid mock rename receipt");
  expect(receipts.snapshot()).toEqual(before);
});
