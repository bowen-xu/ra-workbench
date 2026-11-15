export interface StoragePersistence {
  readonly backend: "IndexedDB";
  readonly pending: boolean;
  readonly error: Error | undefined;
  readonly resetting: boolean;
  flush(): Promise<void>;
  /** Check this hydrated view without replacing it or merging another view. */
  assertCurrent?(): Promise<void>;
  retry(): Promise<void>;
  reset(beforeClear?: () => void): Promise<void>;
  subscribe(changed: () => void): () => void;
  close(): void;
}

// The owner carries its port: HMR can reload this module while retained storage
// instances are still alive. A module-local registry would lose their receipts.
const persistencePort = Symbol.for("research-assistant.storage-persistence.v1");
type PersistentStorage = Storage & {readonly [persistencePort]?: StoragePersistence};
export function registerStoragePersistence(storage: Storage, port: StoragePersistence): void {
  Object.defineProperty(storage, persistencePort, {value: port, configurable: true});
}
export function storagePersistence(storage: Storage): StoragePersistence | undefined { return (storage as PersistentStorage)[persistencePort]; }
export async function flushStorage(storage: Storage): Promise<void> { await storagePersistence(storage)?.flush(); }
export function storageBackend(storage: Storage): string { return storagePersistence(storage)?.backend ?? "localStorage"; }
