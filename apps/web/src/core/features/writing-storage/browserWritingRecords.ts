import type {WritingNoteCatalogGateway} from "@research-assistant/data/notes";
/** Record persistence belongs to the preview database owner, not an editor cache. */
export const writingRecordStores = ["writingProjects", "writingFiles", "writingBodies", "writingSessions", "writingDrafts"] as const;
/** Keep the v2 store definition unchanged; checkpoints were added in v4. */
export const allWritingRecordStores = [...writingRecordStores, "writingCheckpoints"] as const;
export type WritingRecordStore = typeof allWritingRecordStores[number];
/** Recovery records have their own CAS domain: a checkpoint cannot invalidate
 * an unrelated formal file save, and neither revision substitutes for HBS. */
export function writingRecordRevision(stores: readonly WritingRecordStore[]): "writingRevision" | "writingCheckpointRevision" {
  const recovery = stores.includes("writingCheckpoints");
  if (recovery && stores.some(store => store !== "writingCheckpoints")) throw Object.assign(new Error("writing_storage_domain_mismatch：检查点与正式文件须分别提交。"), {code: "writing_storage_domain_mismatch"});
  return recovery ? "writingCheckpointRevision" : "writingRevision";
}
export interface WritingRecordSelection {
  readonly store: WritingRecordStore;
  readonly keys?: readonly string[];
  readonly manuscriptId?: string;
}
export interface WritingRecordChange { readonly store: WritingRecordStore; readonly key: string; readonly value?: unknown }
export interface WritingRecordSnapshot { readonly revision: number; readonly rows: Partial<Record<WritingRecordStore, readonly unknown[]>> }
export interface BrowserWritingRecords {
  readonly noteCatalog?: WritingNoteCatalogGateway;
  readonly pending: boolean;
  subscribe(changed: () => void): () => void;
  /** Metadata/selected bodies share a transaction; checkpoints use a separate CAS domain. */
  read(selections: readonly WritingRecordSelection[]): Promise<WritingRecordSnapshot>;
  /** Compare and publish canonical metadata/content in one durable transaction. */
  commit(expectedRevision: number, changes: readonly WritingRecordChange[]): Promise<void>;
  flush(): Promise<void>;
  /** Prevent new operations; already issued transactions remain flushable. */
  stop(): void;
}
const RECORDS = Symbol.for("research-assistant.writing-records.v1");
type RecordStorage = Storage & { [RECORDS]?: BrowserWritingRecords };
export function registerWritingRecords(storage: Storage, records: BrowserWritingRecords): void {
  Object.defineProperty(storage, RECORDS, {value: records});
}
export function writingRecords(storage: Storage): BrowserWritingRecords | undefined { return (storage as RecordStorage)[RECORDS]; }
export function writingFileKey(manuscriptId: string, path: string): string { return JSON.stringify([manuscriptId, path]); }
