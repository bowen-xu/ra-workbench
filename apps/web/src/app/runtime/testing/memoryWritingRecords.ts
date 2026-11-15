import { allWritingRecordStores, writingRecordRevision, type BrowserWritingRecords, type WritingRecordStore, type WritingRecordSnapshot } from "../../../core/features/writing-storage/browserWritingRecords";

// Test composition binds the same Core-owned record authority as production.
export {registerWritingRecords} from "../../../core/features/writing-storage/browserWritingRecords";

export function memoryWritingState() { return {revision: 0, checkpointRevision: 0, stores: new Map(allWritingRecordStores.map(name => [name, new Map<string, unknown>()]))}; }
/** Explicit persistence double. Production exercises IndexedDB in browser checks. */
export function memoryWritingRecords(state: ReturnType<typeof memoryWritingState>, beforeCommit: () => Promise<void> = async () => undefined): BrowserWritingRecords & {
  readonly reads: {store: WritingRecordStore; count: number}[];
} {
  let stopped = false;
  const pending = new Set<Promise<unknown>>(), listeners = new Set<() => void>();
  const reads: {store: WritingRecordStore; count: number}[] = [];
  const check = () => { if (stopped) throw new Error("writing_storage_closed"); };
  return {
    reads,
    get pending() { return pending.size > 0; },
    subscribe(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    stop() { stopped = true; },
    async flush() { while (pending.size) await Promise.all([...pending]); },
    async read(selections) {
      check(); const rows: WritingRecordSnapshot["rows"] = {};
      const domain = writingRecordRevision(selections.map(selection => selection.store));
      for (const selection of selections) {
        const store = state.stores.get(selection.store)!;
        const values = selection.keys ? selection.keys.flatMap(key => store.has(key) ? [store.get(key)] : [])
          : [...store.values()].filter(value => selection.manuscriptId === undefined || (selection.store === "writingCheckpoints"
            ? (value as {checkpoint: {manuscriptId: string}}).checkpoint.manuscriptId
            : (value as {manuscriptId: string}).manuscriptId) === selection.manuscriptId);
        rows[selection.store] = structuredClone(values); reads.push({store: selection.store, count: values.length});
      }
      return {revision: domain === "writingRevision" ? state.revision : state.checkpointRevision, rows};
    },
    commit(expectedRevision, changes) {
      try { check(); } catch (reason) { return Promise.reject(reason); }
      const result = (async () => {
        const domain = writingRecordRevision(changes.map(change => change.store));
        await beforeCommit();
        if ((domain === "writingRevision" ? state.revision : state.checkpointRevision) !== expectedRevision) throw Object.assign(new Error("writing_revision_conflict"), {code: "writing_revision_conflict"});
        for (const change of changes) {
          const store = state.stores.get(change.store)!;
          if (change.value === undefined) store.delete(change.key); else store.set(change.key, structuredClone(change.value));
        }
        if (domain === "writingRevision") state.revision++; else state.checkpointRevision++;
      })();
      pending.add(result); for (const listener of listeners) listener();
      const complete = () => { pending.delete(result); for (const listener of listeners) listener(); };
      void result.then(complete, complete); return result;
    },
  };
}
