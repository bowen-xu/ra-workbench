import {writingRecords as ownedWritingRecords, type BrowserWritingRecords} from "./browserWritingRecords";

export type WritingRecordAccess = Pick<BrowserWritingRecords, "read" | "commit"> & {
  readonly noteCatalog?: BrowserWritingRecords["noteCatalog"];
};
const accessByOwner = new WeakMap<BrowserWritingRecords, WritingRecordAccess>();

/** Keep the canonical CAS owner and per-owner queues stable across gateway
 * recreation, without giving consumers registration or shutdown control. */
export function writingRecords(storage: Storage): WritingRecordAccess | undefined {
  const owner = ownedWritingRecords(storage);
  if (!owner) return;
  let access = accessByOwner.get(owner);
  if (!access) {
    access = {
      read: selections => owner.read(selections),
      commit: (revision, changes) => owner.commit(revision, changes),
      get noteCatalog() { return owner.noteCatalog; },
    };
    accessByOwner.set(owner, access);
  }
  return access;
}
