import {registerWritingRecords} from "../../../core/features/writing-storage/browserWritingRecords";
import { PreviewStorageError, type PreviewDatabase } from "../../../core/runtime/previewIndexedDatabase";
import {memoryWritingRecords, memoryWritingState} from "./memoryWritingRecords";

/** Persistence test double; browser checks exercise actual IndexedDB transactions. */
export function createMemoryPreviewDatabase() {
  const data = new Map<string, {revision: number; entries: Map<string, string>}>();
  const writing = new Map<string, ReturnType<typeof memoryWritingState>>();
  let failure: unknown, barrier: Promise<void> | undefined;
  const metrics = {commits: 0, initializations: 0};
  return {metrics,
    failNext(reason: unknown) { failure = reason; },
    holdUntil(value: Promise<void> | undefined) { barrier = value; },
    open: async (name: string): Promise<PreviewDatabase> => {
      if (!writing.has(name)) writing.set(name, memoryWritingState());
      const records = memoryWritingRecords(writing.get(name)!, async () => {
        await barrier;
        if (failure !== undefined) { const reason = failure; failure = undefined; throw reason; }
      });
      return {
      managed: records,
      bindStorage: storage => registerWritingRecords(storage,records),
      close() {},
      async readRevision() { return data.get(name)!.revision; },
      async load() {
        if (!data.has(name)) { data.set(name, {revision: 0, entries: new Map()}); metrics.initializations++; }
        const current = data.get(name)!;
        return {revision: current.revision, entries: new Map(current.entries)};
      },
      async commit(revision, changes, clear) {
        await barrier;
        if (failure !== undefined) { const reason = failure; failure = undefined; throw reason; }
        const current = data.get(name)!;
        if (revision !== current.revision) throw new PreviewStorageError("preview_storage_conflict", "其他标签页已修改数据。");
        const entries = clear ? new Map<string, string>() : new Map(current.entries);
        if (clear) { const state = writing.get(name)!; for (const store of state.stores.values()) store.clear(); state.revision++; state.checkpointRevision++; }
        for (const [key, value] of changes) { if (value === null) entries.delete(key); else entries.set(key, value); }
        data.set(name, {revision: revision + 1, entries}); metrics.commits++;
        return revision + 1;
      },
    }; },
  };
}
