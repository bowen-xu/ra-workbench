// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from "vitest";
import { createIndexedPreviewStorage } from "./previewIndexedStorage";
import { configurePreviewDatabase, PreviewStorageError } from "./previewIndexedDatabase";
import { createMemoryPreviewDatabase } from "../../app/runtime/testing/memoryPreviewDatabase";
import { flushStorage, storagePersistence } from "./storagePersistence";
import { withPreviewPersistence } from "./previewPersistenceRuntime";
import type { AppRuntime } from "./appRuntime";
import { capturePreservation, createDraft, type HbsSnapshot, type TextDiff } from "@research-assistant/core/hbs";
import { BrowserHbsPreservationStore } from "../components/hbs/browserHbsPreservation";

let database: ReturnType<typeof createMemoryPreviewDatabase>;
beforeEach(() => { localStorage.clear(); database = createMemoryPreviewDatabase(); configurePreviewDatabase(database.open); });

it("reads and writes only IndexedDB, ignoring retired localStorage data", async () => {
  localStorage.setItem("one:body", "retired backup"); localStorage.setItem("other:body", "other"); localStorage.setItem("business", "private");
  const readLegacy = vi.spyOn(localStorage, "getItem");
  const storage = await createIndexedPreviewStorage("one:");
  expect(storage.getItem("body")).toBeNull(); expect(storage.length).toBe(0);
  storage.setItem("body", "new"); storage.setItem("asset", "图像数据".repeat(20_000)); await flushStorage(storage);
  expect(database.metrics.commits).toBe(1);
  const reopened = await createIndexedPreviewStorage("one:");
  expect(reopened.getItem("body")).toBe("new"); expect(reopened.getItem("asset")).toBe(storage.getItem("asset"));
  expect(readLegacy).not.toHaveBeenCalled(); readLegacy.mockRestore();
  expect(localStorage.getItem("one:body")).toBe("retired backup");
  expect(localStorage.getItem("other:body")).toBe("other"); expect(localStorage.getItem("business")).toBe("private");
  expect(database.metrics.initializations).toBe(1);
});

it("keeps failed writes pending, blocks false receipts and retries without losing the previous durable value", async () => {
  const storage = await createIndexedPreviewStorage("test:");
  storage.setItem("body", "old"); await flushStorage(storage);
  database.failNext(new DOMException("quota exhausted", "QuotaExceededError"));
  storage.setItem("body", "new"); storage.setItem("history", "完整历史");
  await expect(flushStorage(storage)).rejects.toThrow("QuotaExceededError");
  expect(storage.getItem("body")).toBe("new"); expect(storagePersistence(storage)?.pending).toBe(true);
  expect((await createIndexedPreviewStorage("test:")).getItem("body")).toBe("old");
  expect(() => storage.setItem("body", "later")).toThrow("preview_storage_failed");
  await storagePersistence(storage)!.retry();
  const reopened = await createIndexedPreviewStorage("test:");
  expect(reopened.getItem("body")).toBe("new"); expect(reopened.getItem("history")).toBe("完整历史");
  expect(storagePersistence(storage)?.pending).toBe(false); expect(storagePersistence(storage)?.error).toBeUndefined();
});

it("waits for an in-flight commit and includes later queued writes in the same flush barrier", async () => {
  let release!: () => void;
  database.holdUntil(new Promise<void>(resolve => { release = resolve; }));
  const storage = await createIndexedPreviewStorage("test:");
  storage.setItem("first", "1"); let saved = false;
  const pending = flushStorage(storage).then(() => { saved = true; });
  await Promise.resolve(); expect(saved).toBe(false);
  storage.setItem("second", "2"); release(); await pending;
  const reopened = await createIndexedPreviewStorage("test:");
  expect(reopened.getItem("first")).toBe("1"); expect(reopened.getItem("second")).toBe("2");
});

it("resets only this namespace and does not resurrect migrated localStorage records on reopen", async () => {
  localStorage.setItem("one:body", "backup"); localStorage.setItem("two:body", "other");
  const storage = await createIndexedPreviewStorage("one:");
  const other = await createIndexedPreviewStorage("two:");
  other.setItem("body", "other"); await flushStorage(other);
  storage.setItem("body", "changed"); await flushStorage(storage);
  await storagePersistence(storage)!.reset(() => storage.setItem("dispose-history", "final shutdown write"));
  expect(() => storage.setItem("late-background-write", "blocked")).toThrow("preview_storage_closed");
  expect((await createIndexedPreviewStorage("one:")).length).toBe(0);
  expect(other.getItem("body")).toBe("other"); expect(localStorage.getItem("one:body")).toBe("backup");
});

it("retries the full reset after a shutdown-history write fails rather than reopening that history", async () => {
  const storage = await createIndexedPreviewStorage("reset:");
  storage.setItem("body", "old"); await flushStorage(storage);
  database.failNext(new DOMException("full", "QuotaExceededError"));
  await expect(storagePersistence(storage)!.reset(() => storage.setItem("shutdown", "history"))).rejects.toThrow("QuotaExceededError");
  expect(storagePersistence(storage)?.resetting).toBe(true);
  await storagePersistence(storage)!.retry();
  expect((await createIndexedPreviewStorage("reset:")).length).toBe(0);
  expect(() => storage.setItem("late", "blocked")).toThrow("preview_storage_closed");
});

it("rejects a stale tab instead of overwriting a newer committed namespace", async () => {
  const first = await createIndexedPreviewStorage("shared:");
  const stale = await createIndexedPreviewStorage("shared:");
  first.setItem("body", "first commit"); await flushStorage(first);
  stale.setItem("body", "stale commit"); await expect(flushStorage(stale)).rejects.toThrow("preview_storage_conflict");
  expect((await createIndexedPreviewStorage("shared:")).getItem("body")).toBe("first commit");
  expect(stale.getItem("body")).toBe("stale commit");
});
it("detects a stale hydrated view before a fork intent is allocated without reloading or overwriting either view", async () => {
  const first = await createIndexedPreviewStorage("fork-shared:");
  const stale = await createIndexedPreviewStorage("fork-shared:");
  first.setItem("original-fork", "frozen child"); await flushStorage(first); const commits = database.metrics.commits;
  await expect(storagePersistence(stale)!.assertCurrent!()).rejects.toThrow("FORK_RECOVERY_STORAGE_STALE");
  expect(stale.getItem("original-fork")).toBeNull(); expect(database.metrics.commits).toBe(commits);
  const reopened = await createIndexedPreviewStorage("fork-shared:");
  await storagePersistence(reopened)!.assertCurrent!(); expect(reopened.getItem("original-fork")).toBe("frozen child");
});

it("ignores retired corrupt snapshots and rejects an unavailable backend without a localStorage fallback", async () => {
  localStorage.setItem("test:body", "\0research-preview:lz16:1:999:broken");
  expect((await createIndexedPreviewStorage("test:")).getItem("body")).toBeNull();
  expect(localStorage.getItem("test:body")).toBe("\0research-preview:lz16:1:999:broken");
  configurePreviewDatabase(async () => { throw new PreviewStorageError("preview_storage_unavailable", "disabled"); });
  await expect(createIndexedPreviewStorage("test:")).rejects.toThrow("preview_storage_unavailable");
});

it("makes asynchronous gateway and platform receipts wait for persistence while retaining synchronous capabilities", async () => {
  let release!: () => void;
  const storage = await createIndexedPreviewStorage("test:");
  database.holdUntil(new Promise<void>(resolve => { release = resolve; }));
  const unsubscribe = () => undefined;
  const runtime = withPreviewPersistence({mode: "preview", storage, gateways: {}, dispose() {},
    invoke: async () => { storage.setItem("body", "saved"); return "receipt"; },
  });
  let saved = false;
  const write = runtime.invoke!("test").then(value => { saved = true; return value; });
  await Promise.resolve(); expect(saved).toBe(false); release(); expect(await write).toBe("receipt");
  const raw = {nested: {enabled: true, subscribe: () => unsubscribe, save: async () => { storage.setItem("body", "later"); return 2; }}};
  const fake = withPreviewPersistence({mode: "preview", storage, gateways: raw as AppRuntime["gateways"], dispose() {}});
  const port = fake.gateways as typeof raw;
  expect(port.nested.enabled).toBe(true); expect(port.nested.subscribe()).toBe(unsubscribe);
  database.failNext(new Error("disk failure")); await expect(port.nested.save()).rejects.toThrow("disk failure");
  expect(storage.getItem("body")).toBe("later");
});

it("does not acknowledge an HBS preservation checkpoint until the IndexedDB receipt succeeds", async () => {
  const storage = await createIndexedPreviewStorage("hbs:");
  const options = {storage, unitId: "doc", identity: {scope: "preview", resourceId: "doc", slot: "draft"}};
  const store = new BrowserHbsPreservationStore(options);
  const observed: HbsSnapshot = {version: 1, saved: [{id: "doc", label: "Document", text: "old"}], baseline: [{id: "doc", label: "Document", text: "old"}]};
  const diff: TextDiff = (before, after) => before === after ? [] : [{fromA: 0, toA: before.length, fromB: 0, toB: after.length}];
  const previous = capturePreservation(observed, undefined, "doc", diff);
  await store.save(previous);
  let release!: () => void, acknowledged = false;
  database.holdUntil(new Promise<void>(resolve => { release = resolve; }));
  const current = capturePreservation(observed, createDraft(observed, "doc", "new"), "doc", diff);
  const save = store.save(current).then(() => { acknowledged = true; });
  await Promise.resolve(); expect(acknowledged).toBe(false); release(); await save;
  const reloaded = await createIndexedPreviewStorage("hbs:");
  expect(new BrowserHbsPreservationStore({...options, storage: reloaded}).load()?.content).toBe("new");
  database.failNext(new DOMException("quota", "QuotaExceededError"));
  await expect(store.save(previous)).rejects.toThrow("QuotaExceededError");
  const reopened = await createIndexedPreviewStorage("hbs:");
  expect(new BrowserHbsPreservationStore({...options, storage: reopened}).load()?.content).toBe("new");
});

it("retains the persistence owner and receipt when the helper module is reloaded", async () => {
  const storage = await createIndexedPreviewStorage("hot-reload:");
  const original = storagePersistence(storage);
  storage.setItem("body", "still durable");
  vi.resetModules();
  const reloaded = await import("./storagePersistence");
  expect(reloaded.storagePersistence(storage)).toBe(original);
  await reloaded.flushStorage(storage);
  expect((await createIndexedPreviewStorage("hot-reload:")).getItem("body")).toBe("still durable");
});
