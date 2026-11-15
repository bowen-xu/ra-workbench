/** @vitest-environment happy-dom */
import {afterEach, expect, it, vi} from "vitest";
import {NativeCommandDiagnostics} from "./NativeCommandDiagnostics";

afterEach(() => {vi.unstubAllEnvs(); delete (window as unknown as Record<string, unknown>).__nativeCommandDiagnostics;});

it("returns the original promise and records only closed command metadata", async () => {
  let time = 10;
  const diagnostics = new NativeCommandDiagnostics(() => time);
  const result = {privateResult: "synthetic confidential result"};
  let resolve!: (value: typeof result) => void;
  const pending = new Promise<typeof result>(done => {resolve = done;});
  expect(diagnostics.observe("load_note_index_documents", () => pending)).toBe(pending);
  expect(diagnostics.read().entries[0]).toMatchObject({status: "pending", startedAtMs: 10});
  time = 25; resolve(result);
  await expect(pending).resolves.toBe(result);
  expect(diagnostics.read()).toEqual({dropped: 0, entries: [{sequence: 1, command: "load_note_index_documents", startedAtMs: 10, status: "succeeded", durationMs: 15}]});
  const copy = diagnostics.read().entries[0]!;
  Object.assign(copy, {command: "changed"});
  expect(diagnostics.read().entries[0]!.command).toBe("load_note_index_documents");
  expect(JSON.stringify(diagnostics.read())).not.toContain(result.privateResult);
});

it("preserves both asynchronous and synchronous error identity", async () => {
  const diagnostics = new NativeCommandDiagnostics();
  const failure = Object.assign(new Error("safe original cause"), {code: "operation_failed"});
  const pending = Promise.reject(failure);
  expect(diagnostics.observe("save_manuscript_file", () => pending)).toBe(pending);
  await expect(pending).rejects.toBe(failure);
  expect(() => diagnostics.observe("read_manuscript_file", () => {throw failure;})).toThrow(failure);
  expect(diagnostics.read().entries.map(entry => entry.status)).toEqual(["failed", "failed"]);
  expect(JSON.stringify(diagnostics.read())).not.toContain(failure.message);
});

it("bounds observations without skipping or repeating native operations", async () => {
  const diagnostics = new NativeCommandDiagnostics(() => 0, 1), call = vi.fn(async () => "receipt");
  await diagnostics.observe("first", call);
  await diagnostics.observe("second", call);
  const failure = new Error("failed once"), throwing = vi.fn(() => {throw failure;});
  expect(() => diagnostics.observe("third", throwing)).toThrow(failure);
  expect(call).toHaveBeenCalledTimes(2); expect(throwing).toHaveBeenCalledOnce();
  expect(diagnostics.read().entries).toHaveLength(1); expect(diagnostics.read().dropped).toBe(2);
});

it("keeps unavailable clocks outside the native command's failure boundary", async () => {
  const diagnostics = new NativeCommandDiagnostics(() => {throw new Error("clock unavailable");});
  const call = vi.fn(async () => "receipt");
  await expect(diagnostics.observe("read_file", call)).resolves.toBe("receipt");
  expect(call).toHaveBeenCalledOnce(); expect(diagnostics.read().entries).toEqual([]);
});

it("redacts malformed command names rather than recording arbitrary caller text", async () => {
  const diagnostics = new NativeCommandDiagnostics();
  await diagnostics.observe("synthetic body with spaces", async () => undefined);
  expect(diagnostics.read().entries[0]!.command).toBe("invalid-command-name");
});

it("exposes a read-only inspector snapshot only when explicitly enabled", async () => {
  vi.resetModules(); vi.stubEnv("VITE_NATIVE_COMMAND_TRACE", undefined);
  expect((await import("./NativeCommandDiagnostics")).nativeCommandDiagnostics).toBeUndefined();
  expect(Object.hasOwn(window, "__nativeCommandDiagnostics")).toBe(false);
  vi.resetModules(); vi.stubEnv("VITE_NATIVE_COMMAND_TRACE", "1");
  const enabled = (await import("./NativeCommandDiagnostics")).nativeCommandDiagnostics!;
  await enabled.observe("get_runtime", async () => undefined);
  const inspector = (window as unknown as {__nativeCommandDiagnostics: {read(): unknown}}).__nativeCommandDiagnostics;
  expect(Object.keys(inspector)).toEqual(["read"]);
  expect(inspector.read()).toEqual(enabled.read());
  expect(Object.isFrozen(inspector)).toBe(true);
});
