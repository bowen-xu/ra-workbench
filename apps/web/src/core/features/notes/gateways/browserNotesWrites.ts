import { NotesGatewayError, type NotesGateway } from "@research-assistant/data/notes";
import { getAppStorage } from "../../../runtime/appStorage";

const tails = new WeakMap<Storage, Promise<void>>();

/** Serialize one runtime owner's read/validate/commit sequences.
 * Other tabs still use IndexedDB version checks; editor CAS is unchanged. */
export function withBrowserNotesWrite<T>(operation: () => Promise<T>): Promise<T> {
  const storage = getAppStorage();
  const pending = (tails.get(storage) ?? Promise.resolve()).then(() => {
    if (getAppStorage() !== storage) throw new NotesGatewayError("operation_cancelled", "笔记存储拥有者已切换，操作未执行");
    return operation();
  });
  const settled = pending.then(() => {}, () => {});
  tails.set(storage, settled);
  void settled.then(() => {if (tails.get(storage) === settled) tails.delete(storage);});
  return pending;
}

const writes = new Set<keyof NotesGateway>([
  "createPage", "createLiteratureNote", "updatePage", "movePage", "duplicatePage", "setPageStatus", "permanentlyDeletePage",
  "saveDocument", "resolveConflict", "setRevisionPinned", "restoreRevision", "createReadingNote", "updateReadingNote",
  "setReadingNoteStatus", "permanentlyDeleteReadingNote", "upsertLink", "removeLink",
  "applyMetadataBatch", "replayMetadataBatch", "commitNavigation", "reorderFavorites", "importNoteAsset", "ensureNoteTag",
]);

export function serializeBrowserNotesWrites(gateway: NotesGateway): NotesGateway {
  return new Proxy(gateway, {get(target, key, receiver) {
    const value: unknown = Reflect.get(target, key, receiver);
    if (typeof value !== "function") return value;
    // Internal calls bind to the raw adapter, avoiding nested queue deadlocks.
    return writes.has(key as keyof NotesGateway)
      ? (...args: unknown[]) => withBrowserNotesWrite(() => Reflect.apply(value, target, args))
      : value.bind(target);
  }});
}
