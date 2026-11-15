import { appStorageEventKey, getAppStorage } from "./appStorage";

const CHANGED = "research-assistant:conversation-catalog-changed";
const REVISION = "research-assistant:conversation-catalog-revision";

/** Invalidation only. Receivers always read their own authoritative gateway. */
export function invalidateConversationCatalog(): void {
  window.dispatchEvent(new Event(CHANGED));
  try { getAppStorage().setItem(REVISION, crypto.randomUUID()); }
  catch { /* Restricted storage must not prevent same-window synchronization. */ }
}

export function subscribeConversationCatalog(changed: () => void): () => void {
  const key = appStorageEventKey(REVISION);
  const storageChanged = (event: StorageEvent) => { if (event.key === key || event.key === null) changed(); };
  window.addEventListener(CHANGED, changed);
  window.addEventListener("storage", storageChanged);
  return () => { window.removeEventListener(CHANGED, changed); window.removeEventListener("storage", storageChanged); };
}
