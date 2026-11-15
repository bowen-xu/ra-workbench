import {expect, it, vi} from "vitest";
import {writingRecords} from "../../public/writingStorage";
import {registerWritingRecords, type BrowserWritingRecords} from "./browserWritingRecords";

it("reuses one consumer port per authority without exposing registration or shutdown", async () => {
  const storage = {} as Storage, secondStorage = {} as Storage;
  expect(writingRecords(storage)).toBeUndefined();
  const snapshot = {revision: 7, rows: {}}, failure = Object.assign(new Error("original failure"), {code: "writing_revision_conflict"});
  const owner: BrowserWritingRecords = {read: vi.fn(async () => snapshot), commit: vi.fn(async () => {throw failure;}),
    pending: false, subscribe: vi.fn(() => () => undefined), stop: vi.fn(), flush: vi.fn(async () => undefined)};
  registerWritingRecords(storage, owner);
  registerWritingRecords(secondStorage, owner);
  const access = writingRecords(storage)!;
  expect(writingRecords(secondStorage)).toBe(access);
  expect(Object.keys(access).sort()).toEqual(["commit", "noteCatalog", "read"]);
  await expect(access.read([])).resolves.toBe(snapshot);
  const changes = [{store: "writingBodies" as const, key: "canonical", value: "unchanged bytes"}];
  await expect(access.commit(7, changes)).rejects.toBe(failure);
  expect(owner.commit).toHaveBeenCalledWith(7, changes);
  expect(owner.stop).not.toHaveBeenCalled();
  expect(owner.flush).not.toHaveBeenCalled();
});
