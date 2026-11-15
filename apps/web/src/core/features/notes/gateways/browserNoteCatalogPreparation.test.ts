import {expect,it,vi} from "vitest";
import {joinBrowserNoteCatalogPreparation} from "./browserNoteCatalogPreparation";

// This owner token never performs I/O; real transaction/cursor behavior is
// covered by note-catalog-cancellation.pw.ts using IndexedDB.
const database = () => ({}) as IDBDatabase;
function deferred() {
  let resolve: () => void = () => {throw Error("deferred was not initialized");};
  const promise = new Promise<void>(complete => {resolve = complete;});
  return {promise,resolve};
}
it("a canceled consumer does not stop another reader of the same preparation",async () => {
  const db = database(), drain = deferred(), controller = new AbortController();
  let preparationSignal: AbortSignal | undefined;
  const run = vi.fn(async (signal: AbortSignal) => {preparationSignal = signal;await drain.promise;});
  const first = joinBrowserNoteCatalogPreparation(db,"search",run,controller.signal);
  const second = joinBrowserNoteCatalogPreparation(db,"search",run);
  await Promise.resolve();controller.abort();
  await expect(first).rejects.toMatchObject({name: "AbortError"});
  expect(preparationSignal?.aborted).toBe(false);expect(run).toHaveBeenCalledOnce();
  drain.resolve();await second;
});
it("replacement preparation waits for the canceled atomic batch to drain",async () => {
  const db = database(), drain = deferred(), controller = new AbortController();
  const firstRun = vi.fn(async (signal: AbortSignal) => {await drain.promise;signal.throwIfAborted();});
  const first = joinBrowserNoteCatalogPreparation(db,"search",firstRun,controller.signal);
  await Promise.resolve();controller.abort();
  await expect(first).rejects.toMatchObject({name: "AbortError"});
  const retry = vi.fn(async () => undefined);
  const second = joinBrowserNoteCatalogPreparation(db,"search",retry);
  await Promise.resolve();expect(retry).not.toHaveBeenCalled();
  drain.resolve();await second;expect(retry).toHaveBeenCalledOnce();
});
it("a failed preparation is released and its original diagnostic survives",async () => {
  const db = database(), failure = Error("note_catalog_document_missing: fixture");
  await expect(joinBrowserNoteCatalogPreparation(db,"titles",async () => {throw failure;})).rejects.toBe(failure);
  const retry = vi.fn(async () => undefined);
  await joinBrowserNoteCatalogPreparation(db,"titles",retry);expect(retry).toHaveBeenCalledOnce();
});
it("an already canceled owner starts no work",async () => {
  const run = vi.fn(async () => undefined), controller = new AbortController();controller.abort();
  await expect(joinBrowserNoteCatalogPreparation(database(),"metadata",run,controller.signal)).rejects.toMatchObject({name: "AbortError"});
  expect(run).not.toHaveBeenCalled();
});
