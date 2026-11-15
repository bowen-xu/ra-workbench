import type {CatalogStatus} from "./browserNoteCatalogScopes";
type PreparationMode = "metadata" | "searchMetadata" | "search" | "titles" | `search:${CatalogStatus}` | `titles:${CatalogStatus}`;
interface Preparation {
  readonly controller: AbortController;
  readonly promise: Promise<void>;
  consumers: number;
}
const pending = new WeakMap<IDBDatabase, Map<PreparationMode, Preparation>>();

/** Share preparation while any caller needs it. The final cancellation stops
 * future batches, not an in-flight atomic canonical/projection transaction.
 * A replacement waits for that transaction to drain before resuming its cursor. */
export async function joinBrowserNoteCatalogPreparation(
  database: IDBDatabase, mode: PreparationMode,
  prepare: (signal: AbortSignal) => Promise<void>, signal?: AbortSignal,
): Promise<void> {
  signal?.throwIfAborted();
  let operations = pending.get(database);
  if (!operations) {operations = new Map(); pending.set(database, operations);}
  let operation = operations.get(mode);
  if (operation?.controller.signal.aborted) {
    await operation.promise.catch(() => undefined);
    signal?.throwIfAborted();
    return joinBrowserNoteCatalogPreparation(database, mode, prepare, signal);
  }
  if (!operation) {
    const controller = new AbortController();
    const entry: Preparation = {
      controller, consumers: 0,
      promise: Promise.resolve().then(() => prepare(controller.signal)).finally(() => {
        if (operations.get(mode) === entry) operations.delete(mode);
      }),
    };
    operation = entry;
    operations.set(mode, entry);
  }
  const shared = operation;
  shared.consumers++;
  return new Promise<void>((resolve, reject) => {
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      signal?.removeEventListener("abort", abort);
      if (--shared.consumers === 0) shared.controller.abort();
    };
    const abort = () => {release(); reject(signal?.reason);};
    signal?.addEventListener("abort", abort, {once: true});
    shared.promise.then(() => {release(); resolve();}, reason => {release(); reject(reason);});
  });
}
