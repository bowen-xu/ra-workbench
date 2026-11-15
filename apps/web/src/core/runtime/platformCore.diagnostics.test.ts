/** @vitest-environment happy-dom */
import {afterEach, beforeEach, expect, it, vi} from "vitest";

const native = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("@tauri-apps/api/core", () => ({invoke: native.invoke, isTauri: () => true}));
beforeEach(() => {vi.resetModules(); native.invoke.mockReset(); vi.stubEnv("VITE_NATIVE_COMMAND_TRACE", "1");});
afterEach(() => {vi.unstubAllEnvs(); delete (window as unknown as Record<string, unknown>).__nativeCommandDiagnostics;});

it("observes the real native dispatch without copying arguments or results", async () => {
  const {invoke} = await import("./platformCore");
  const {nativeCommandDiagnostics} = await import("./NativeCommandDiagnostics");
  const argumentsValue = {privateBody: "synthetic request body"}, receipt = {privateResult: "synthetic response body"};
  native.invoke.mockResolvedValue(receipt);
  await expect(invoke("read_manuscript_file", argumentsValue)).resolves.toBe(receipt);
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith("read_manuscript_file", argumentsValue);
  expect(nativeCommandDiagnostics!.read().entries[0]).toMatchObject({command: "read_manuscript_file", status: "succeeded"});
  const serialized = JSON.stringify(nativeCommandDiagnostics!.read());
  expect(serialized).not.toContain(argumentsValue.privateBody); expect(serialized).not.toContain(receipt.privateResult);
});

it("preserves native rejection details and identity", async () => {
  const {invoke} = await import("./platformCore");
  const failure = {code: "storage_error", operationId: "synthetic-operation", message: "safe cause"};
  native.invoke.mockRejectedValue(failure);
  await expect(invoke("save_manuscript_file")).rejects.toBe(failure);
  const {nativeCommandDiagnostics} = await import("./NativeCommandDiagnostics");
  expect(nativeCommandDiagnostics!.read().entries[0]!.status).toBe("failed");
});

it("keeps preview dispatch outside native diagnostics and the native bridge", async () => {
  const {selectAppRuntime} = await import("./appRuntime");
  const previewInvoke = vi.fn(async () => "preview receipt");
  selectAppRuntime({mode: "preview", storage: localStorage, gateways: {}, invoke: previewInvoke, dispose() {}});
  const {invoke} = await import("./platformCore");
  await expect(invoke("read_manuscript_file")).resolves.toBe("preview receipt");
  expect(native.invoke).not.toHaveBeenCalled();
  const {nativeCommandDiagnostics} = await import("./NativeCommandDiagnostics");
  expect(nativeCommandDiagnostics!.read().entries).toEqual([]);
});

it("leaves normal native dispatch available with diagnostics disabled", async () => {
  vi.stubEnv("VITE_NATIVE_COMMAND_TRACE", undefined);
  const {invoke} = await import("./platformCore");
  native.invoke.mockResolvedValue("receipt");
  await expect(invoke("read_manuscript_file")).resolves.toBe("receipt");
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith("read_manuscript_file");
  expect(Object.hasOwn(window, "__nativeCommandDiagnostics")).toBe(false);
});
