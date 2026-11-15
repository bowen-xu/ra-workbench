// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { PreviewPersistenceMonitor } from "./PreviewPersistenceMonitor";
import { createIndexedPreviewStorage } from "./previewIndexedStorage";
import { configurePreviewDatabase } from "./previewIndexedDatabase";
import { createMemoryPreviewDatabase } from "../../app/runtime/testing/memoryPreviewDatabase";
import { flushStorage } from "./storagePersistence";
import { notify } from "../services/notifications";

vi.mock("../services/notifications", () => ({notify: {error: vi.fn(), loading: vi.fn(), dismiss: vi.fn()}}));
let root: ReturnType<typeof createRoot> | undefined, host: HTMLDivElement;
let database: ReturnType<typeof createMemoryPreviewDatabase>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.clearAllMocks();
  database = createMemoryPreviewDatabase(); configurePreviewDatabase(database.open);
});
afterEach(async () => { await act(async () => root?.unmount()); host?.remove(); vi.unstubAllGlobals(); });
async function open(storage: Storage) {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<PreviewPersistenceMonitor storage={storage} />));
}

it("reports synchronous preference-write failures with a durable retry action", async () => {
  const storage = await createIndexedPreviewStorage("monitor:"); await open(storage);
  database.failNext(new DOMException("full", "QuotaExceededError"));
  storage.setItem("preference", "new"); await expect(flushStorage(storage)).rejects.toThrow("QuotaExceededError");
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("preview_storage_failed"), expect.objectContaining({dismissible: false}));
  const options = vi.mocked(notify.error).mock.calls.at(-1)![1]!;
  expect(options.actions?.[0]?.label).toBe("重试保存");
  await options.actions![0]!.onClick();
  expect((await createIndexedPreviewStorage("monitor:")).getItem("preference")).toBe("new");
  expect(notify.dismiss).toHaveBeenCalledWith("preview-indexeddb-persistence");
});

it("blocks leaving while a write is pending and removes the guard after its receipt", async () => {
  const storage = await createIndexedPreviewStorage("monitor:"); await open(storage);
  let release!: () => void;
  database.holdUntil(new Promise<void>(resolve => { release = resolve; }));
  storage.setItem("draft", "retained"); const saving = flushStorage(storage);
  const pending = new Event("beforeunload", {cancelable: true}); window.dispatchEvent(pending);
  expect(pending.defaultPrevented).toBe(true);
  release(); await saving;
  const saved = new Event("beforeunload", {cancelable: true}); window.dispatchEvent(saved);
  expect(saved.defaultPrevented).toBe(false);
});
