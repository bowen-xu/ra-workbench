// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import { MockCreateBoundReceipts } from "./MockCreateBoundReceipts";
import { MockRenameReceipts } from "./MockRenameReceipts";
const epoch = "00000000-0000-0000-0000-000000000003";
const request = {operationId: "00000000-0000-0000-0000-000000000001", id: "00000000-0000-0000-0000-000000000002",
  binding: {kind: "note" as const, resourceId: "note-one"}};

it("keeps proof separate from current metadata and restores the immutable intent", () => {
  const receipts = new MockCreateBoundReceipts(() => epoch), live = vi.fn(), mutate = vi.fn();
  const proof = receipts.apply(request, live, mutate);
  const restored = new MockCreateBoundReceipts(() => epoch); restored.restore(receipts.snapshot());
  expect(restored.apply(request, live, mutate)).toEqual({...proof, replayed: true});
  expect(mutate).toHaveBeenCalledTimes(1);
  expect(restored.status(request).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
});
it("checks live target before write replay while status retains proof without a target lookup", () => {
  const receipts = new MockCreateBoundReceipts(() => epoch), mutate = vi.fn(); let exists = true;
  const live = vi.fn(() => { if (!exists) throw "notFound"; });
  const proof = receipts.apply(request, live, mutate); exists = false;
  expect(() => receipts.apply(request, live, mutate)).toThrow();
  const before = receipts.snapshot();
  expect(receipts.status(request).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
  expect(live).toHaveBeenCalledTimes(2); expect(mutate).toHaveBeenCalledTimes(1); expect(receipts.snapshot()).toEqual(before);
});
it("rejects changed raw C or binding and conflicting rename identities in both directions", () => {
  const renames: MockRenameReceipts = new MockRenameReceipts(id => receipts.has(id));
  const receipts: MockCreateBoundReceipts = new MockCreateBoundReceipts(() => renames.epochId(), id => renames.has(id));
  const mutate = vi.fn(); receipts.apply(request, () => undefined, mutate);
  for (const patch of [{id: crypto.randomUUID()}, {binding: {...request.binding, resourceId: " note-one"}}])
    expect(() => receipts.status({...request, ...patch})).toThrow();
  const rename = {operationId: request.operationId, id: request.id, expectedTitle: "A", title: "B"};
  expect(() => renames.apply(rename, () => undefined)).toThrow();
  renames.apply({...rename, operationId: epoch}, () => undefined);
  expect(() => receipts.apply({...request, operationId: epoch}, () => undefined, mutate)).toThrow();
  expect(mutate).toHaveBeenCalledTimes(1);
});
it("keeps missing receipts absent after rejected mutations and rejects corrupt restored identity atomically", () => {
  const receipts = new MockCreateBoundReceipts(() => epoch);
  expect(() => receipts.apply(request, () => undefined, () => { throw "conflict"; })).toThrow();
  expect(receipts.status(request).status.type).toBe("noSqlReceipt");
  receipts.apply(request, () => undefined, () => undefined);
  const before = receipts.snapshot(), entries = new Map(before.entries), entry = entries.get(request.operationId)!;
  entries.set(request.operationId, {...entry, proof: {...entry.proof, epoch: crypto.randomUUID()}});
  expect(() => receipts.restore({entries})).toThrow("Invalid mock creation receipt");
  expect(receipts.snapshot()).toEqual(before);
});
