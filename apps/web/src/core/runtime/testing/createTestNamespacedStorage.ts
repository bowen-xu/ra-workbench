import {PREVIEW_STORAGE_PREFIX, registerAppStoragePrefix} from "../appStorage";

/** Test fixture only; production preview storage is IndexedDB. */
export function createTestNamespacedStorage(backing: Storage, prefix = PREVIEW_STORAGE_PREFIX): Storage {
  const keys = () => Array.from({length: backing.length}, (_, i) => backing.key(i)).filter((key): key is string => key !== null && key.startsWith(prefix));
  const storage: Storage = {
    get length() { return keys().length; },
    key: i => keys()[i]?.slice(prefix.length) ?? null,
    getItem: key => backing.getItem(prefix + key),
    setItem: (key, value) => backing.setItem(prefix + key, value),
    removeItem: key => backing.removeItem(prefix + key),
    clear: () => { for (const key of keys()) backing.removeItem(key); },
  };
  registerAppStoragePrefix(storage, prefix);
  return storage;
}
