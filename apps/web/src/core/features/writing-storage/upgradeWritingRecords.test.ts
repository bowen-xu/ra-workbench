// @vitest-environment happy-dom
import {expect, it, vi} from "vitest";
import {upgradeWritingRecords} from "./upgradeWritingRecords";

const time = "2026-10-24T00:00:00Z", sha256 = "a".repeat(64);
function fixture() {
  return {manuscripts: [["project", {manuscript: {id: "project", title: "Synthetic", rootPath: "main.tex", status: "active", createdAt: time, updatedAt: time},
    files: [["main.tex", {relativePath: "main.tex", content: "合成原始内容", sha256}]], directories: ["notes"], editedAt: [["main.tex", "2026-10-24T01:02:03Z"]]}] as const],
  sessions: [["project", {manuscriptId: "project", openPaths: ["main.tex"], activePath: "main.tex", cursorState: {offset: 3}, layoutState: {}, updatedAt: time}]],
  drafts: [["project:main.tex", {manuscriptId: "project", relativePath: "main.tex", content: "候选", baseSha256: sha256, updatedAt: time}]]};
}
/** This double only observes upgrade requests. Browser verification proves
 * actual versionchange rollback and reopen; it is not simulated by this test. */
function prepare(value: unknown) {
  const request = {result: value, onsuccess: undefined as undefined | (() => void)};
  const stores = new Map<string, {add: ReturnType<typeof vi.fn>; delete: ReturnType<typeof vi.fn>; get: ReturnType<typeof vi.fn>}>();
  const objectStore = (name: string) => {
    if (!stores.has(name)) stores.set(name, {add: vi.fn(), delete: vi.fn(), get: vi.fn(() => request)});
    return stores.get(name)!;
  };
  const abort = vi.fn(), failed = vi.fn();
  upgradeWritingRecords({objectStore, abort, addEventListener: vi.fn()} as unknown as IDBTransaction, failed);
  request.onsuccess?.(); return {stores, abort, failed};
}

it("splits canonical files, sessions and recovery without changing payloads or file edit times", () => {
  const source = fixture(), result = prepare(JSON.stringify(source));
  expect(result.abort).not.toHaveBeenCalled(); expect(result.failed).not.toHaveBeenCalled();
  expect(result.stores.get("writingProjects")?.add).toHaveBeenCalledWith({manuscript: source.manuscripts[0]![1].manuscript, directories: ["notes"]}, "project");
  expect(result.stores.get("writingFiles")?.add).toHaveBeenCalledWith({manuscriptId: "project", relativePath: "main.tex", sha256, modifiedAt: "2026-10-24T01:02:03Z", sizeBytes: new TextEncoder().encode("合成原始内容").length, kind: "tex"}, '["project","main.tex"]');
  expect(result.stores.get("writingBodies")?.add).toHaveBeenCalledWith({relativePath: "main.tex", content: "合成原始内容", sha256}, '["project","main.tex"]');
  expect(result.stores.get("writingSessions")?.add).toHaveBeenCalledWith(source.sessions[0]![1], "project");
  expect(result.stores.get("writingDrafts")?.add).toHaveBeenCalledWith(source.drafts[0]![1], '["project","main.tex"]');
  expect(result.stores.get("entries")?.delete).toHaveBeenCalledWith("writing-files");
});

it("leaves fresh spaces empty and uses the original project timestamp when per-file timestamps were absent", () => {
  const empty = prepare(undefined);
  expect(empty.stores.size).toBe(1); expect(empty.stores.get("entries")?.delete).not.toHaveBeenCalled();
  const source = fixture(), {editedAt: _dates, ...project} = source.manuscripts[0]![1];
  const result = prepare(JSON.stringify({...source, manuscripts: [["project", project]]}));
  expect(result.stores.get("writingFiles")?.add).toHaveBeenCalledWith(expect.objectContaining({modifiedAt: time}), '["project","main.tex"]');
});

it.each(["{ SYNTHETIC_PRIVATE_SENTINEL", JSON.stringify({...fixture(), drafts: [["wrong-key", fixture().drafts[0]![1]]]})])("aborts malformed source, retains a safe stage, and never echoes its payload", value => {
  const result = prepare(value);
  expect(result.abort).toHaveBeenCalledOnce(); expect(result.stores.get("entries")?.delete).not.toHaveBeenCalled();
  const error = result.failed.mock.calls[0]?.[0] as Error & {code: string};
  expect(error.code).toBe("writing_storage_corrupt"); expect(error.message).toMatch(/stage=(snapshot|drafts)/u);
  expect(error.message).not.toContain("SYNTHETIC_PRIVATE_SENTINEL"); expect(error.message).not.toContain("候选");
});
