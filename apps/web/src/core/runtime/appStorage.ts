/** Browser data is an explicit runtime port. Preview never sees business keys. */

let selectedStorage: Storage | undefined;
const prefixes = new WeakMap<Storage, string>();

export function selectAppStorage(storage: Storage): void { selectedStorage = storage; }
export function registerAppStoragePrefix(storage: Storage, prefix: string): void { prefixes.set(storage, prefix); }
export function getAppStorage(): Storage { return selectedStorage ?? window.localStorage; }
/** Use the captured storage's namespace, even if the active runtime changed. */
export function appStorageScopedKey(storage: Storage, key: string): string { return (prefixes.get(storage) ?? "") + key; }
export function appStorageEventKey(key: string): string { return (selectedStorage ? prefixes.get(selectedStorage) ?? "" : "") + key; }

export const PREVIEW_STORAGE_PREFIX = "research-assistant.app-preview.v3:";
