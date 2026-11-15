// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {getAppRuntime, selectAppRuntime, type AppRuntime} from "../../../runtime/appRuntime";
import {configurePreviewDatabase} from "../../../runtime/previewIndexedDatabase";
import {createIndexedPreviewStorage} from "../../../runtime/previewIndexedStorage";
import {createMemoryPreviewDatabase} from "../../../../app/runtime/testing/memoryPreviewDatabase";
import {storagePersistence} from "../../../runtime/storagePersistence";
import {browserNotesGateway} from "./browserNotesGateway";
import {NOTE_ASSET_PREFIX} from "./browserNotesStorage";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});

const request = {displayName: "original-screenshot.png", mimeType: "image/png", bytes: [137, 80, 78, 71]};
let previous: AppRuntime, database: ReturnType<typeof createMemoryPreviewDatabase>;
beforeEach(() => {previous = getAppRuntime(); database = createMemoryPreviewDatabase(); configurePreviewDatabase(database.open);});
afterEach(() => {selectAppRuntime(previous); vi.restoreAllMocks();});
function activate(storage: Storage) {selectAppRuntime({mode: "preview", storage, gateways: {}, dispose() {}});}

it("persists ID-based image metadata and preserves content deduplication across reopen", async () => {
  const storage = await createIndexedPreviewStorage("note-assets-name:"); activate(storage);
  const first = await browserNotesGateway.importNoteAsset(request);
  const second = await browserNotesGateway.importNoteAsset({...request, displayName: "another.png"});
  expect(first.id).toBe(second.id); expect(first.displayName).toBe(`${first.id}.png`); expect(second.displayName).toBe(first.displayName);
  const metadata = storage.getItem(NOTE_ASSET_PREFIX + first.id)!;
  expect(JSON.parse(metadata)).toMatchObject({id: first.id, displayName: `${first.id}.png`, mimeType: "image/png"});
  expect(metadata).not.toContain(request.displayName); expect(metadata).not.toContain("another.png");
  activate(await createIndexedPreviewStorage("note-assets-name:"));
  expect([...await browserNotesGateway.readNoteAsset(first.id)]).toEqual(request.bytes);
});

it("waits for the durable IndexedDB receipt before publishing an imported image", async () => {
  const storage = await createIndexedPreviewStorage("note-assets-wait:"); activate(storage);
  let release!: () => void; database.holdUntil(new Promise<void>(resolve => {release = resolve;}));
  let published = false;
  const importing = browserNotesGateway.importNoteAsset(request).then(asset => {published = true; return asset;});
  await vi.waitFor(() => expect(storagePersistence(storage)?.pending).toBe(true));
  expect(published).toBe(false); release(); const asset = await importing;
  expect(published).toBe(true);
  activate(await createIndexedPreviewStorage("note-assets-wait:"));
  expect([...await browserNotesGateway.readNoteAsset(asset.id)]).toEqual(request.bytes);
});

it("rejects a failed durable import without publishing a usable reference and retains retry ownership", async () => {
  const storage = await createIndexedPreviewStorage("note-assets-fail:"); activate(storage);
  const previousAsset = await browserNotesGateway.importNoteAsset({...request, bytes: [1, 2, 3]});
  database.failNext(new DOMException("full", "QuotaExceededError"));
  const publish = vi.fn();
  await expect(browserNotesGateway.importNoteAsset(request).then(publish)).rejects.toThrow("QuotaExceededError");
  expect(publish).not.toHaveBeenCalled(); expect(storagePersistence(storage)?.pending).toBe(true);
  const failedId = storage.key(1)!.slice(NOTE_ASSET_PREFIX.length);
  activate(await createIndexedPreviewStorage("note-assets-fail:"));
  expect([...await browserNotesGateway.readNoteAsset(previousAsset.id)]).toEqual([1, 2, 3]);
  await expect(browserNotesGateway.readNoteAsset(failedId)).rejects.toMatchObject({code: "not_found"});
  await storagePersistence(storage)!.retry();
  activate(await createIndexedPreviewStorage("note-assets-fail:"));
  expect([...await browserNotesGateway.readNoteAsset(failedId)]).toEqual(request.bytes);
});

it("keeps a non-image attachment's original filename", async () => {
  const storage = await createIndexedPreviewStorage("note-assets-attachment:"); activate(storage);
  const asset = await browserNotesGateway.importNoteAsset({...request, displayName: "results.csv", mimeType: "text/csv"});
  expect(asset.displayName).toBe("results.csv");
  expect(JSON.parse(storage.getItem(NOTE_ASSET_PREFIX + asset.id)!)).toMatchObject({displayName: "results.csv"});
});

it("does not move an import into a different preview owner while hashing", async () => {
  const first = await createIndexedPreviewStorage("note-assets-source:"), second = await createIndexedPreviewStorage("note-assets-target:");
  activate(first);
  let release!: () => void;
  const barrier = new Promise<void>(resolve => {release = resolve;});
  const digest = vi.spyOn(crypto.subtle, "digest").mockImplementation(async () => {await barrier; return new ArrayBuffer(32);});
  const importing = browserNotesGateway.importNoteAsset(request);
  await vi.waitFor(() => expect(digest).toHaveBeenCalledOnce());
  activate(second); release();
  await expect(importing).rejects.toMatchObject({code: "operation_cancelled"});
  expect(first.length).toBe(0); expect(second.length).toBe(0);
});

it("does not publish a reference into a different preview owner after committing", async () => {
  const first = await createIndexedPreviewStorage("note-assets-source:"), second = await createIndexedPreviewStorage("note-assets-target:");
  activate(first);
  let release!: () => void; database.holdUntil(new Promise<void>(resolve => {release = resolve;}));
  const importing = browserNotesGateway.importNoteAsset(request);
  await vi.waitFor(() => expect(storagePersistence(first)?.pending).toBe(true));
  activate(second); release();
  await expect(importing).rejects.toMatchObject({code: "operation_cancelled"});
  expect(first.length).toBe(1); expect(second.length).toBe(0);
});
