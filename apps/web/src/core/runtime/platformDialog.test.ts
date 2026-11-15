// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import capability from "../../../../desktop/src-tauri/capabilities/main.json";
import {confirm} from "./platformDialog";

const ports = vi.hoisted(() => ({mode: "desktop" as "desktop" | "browser" | "preview", invoke: vi.fn()}));
vi.mock("./appRuntime", () => ({getAppRuntime: () => ({mode: ports.mode}), isAppPreview: () => ports.mode === "preview"}));
const originals = new Map(["confirm", "__TAURI_INTERNALS__"].map(key => [key, Object.getOwnPropertyDescriptor(window, key)]));
beforeEach(() => {
  Object.defineProperty(window, "confirm", {value: vi.fn(), writable: true, configurable: true});
  // Exercise the installed official guest-js and API implementations; only the
  // native effect is replaced at the bridge used by @tauri-apps/api/core.
  Object.defineProperty(window, "__TAURI_INTERNALS__", {value: {invoke: ports.invoke}, configurable: true});
});
afterEach(() => {
  ports.mode = "desktop"; ports.invoke.mockReset(); vi.restoreAllMocks();
  for (const [key, descriptor] of originals) {
    if (descriptor) Object.defineProperty(window, key, descriptor);
    else Reflect.deleteProperty(window, key);
  }
});

it("uses the installed official guest API's message/OkCancel command instead of injected window.confirm", async () => {
  const injected = vi.spyOn(window, "confirm").mockImplementation(() => {throw Error("Obsolete injected command must not run");});
  let settle!: (button: string) => void;
  ports.invoke.mockReturnValue(new Promise<string>(resolve => {settle = resolve;}));
  const pending = confirm("Synthetic import confirmation");
  expect(ports.invoke).toHaveBeenCalledExactlyOnceWith("plugin:dialog|message", {
    message: "Synthetic import confirmation", title: undefined, kind: undefined, buttons: "OkCancel",
  }, undefined);
  expect(injected).not.toHaveBeenCalled();
  settle("Ok");
  await expect(pending).resolves.toBe(true);
  ports.invoke.mockResolvedValueOnce("Cancel");
  await expect(confirm("Synthetic cancelled import")).resolves.toBe(false);
  expect(capability.permissions).toContain("dialog:allow-message");
  expect(capability.permissions).not.toContain("dialog:default");
});

it.each(["browser", "preview"] as const)("retains a real browser confirmation in %s rather than accepting automatically", async mode => {
  ports.mode = mode;
  const browser = vi.spyOn(window, "confirm").mockReturnValueOnce(false).mockReturnValueOnce(true);
  await expect(confirm("Cancel this import")).resolves.toBe(false);
  await expect(confirm("Accept this import")).resolves.toBe(true);
  expect(browser.mock.calls).toEqual([["Cancel this import"], ["Accept this import"]]);
  expect(ports.invoke).not.toHaveBeenCalled();
});

it("retains stable safe diagnostics and the original failure without including dialog content", async () => {
  const failure = {code: "permission_denied", message: "Permission denied: synthetic private body", requestId: "request-1", operationId: "operation-1", status: 403};
  ports.invoke.mockRejectedValueOnce(failure);
  const pending = confirm("Synthetic private confirmation title/body");
  await expect(pending).rejects.toMatchObject({code: "PLATFORM_CONFIRMATION_FAILED", cause: failure,
    diagnostic: {code: "permission_denied", cause: "permission_denied", requestId: "request-1", operationId: "operation-1", httpStatus: 403}});
  await expect(pending).rejects.toThrow(/PLATFORM_CONFIRMATION_FAILED.*plugin:dialog\|message.*permission_denied/u);
  await expect(pending).rejects.not.toThrow(/private/u);
});

it("keeps the installed runtime's denied-message failure actionable without arbitrary exception text", async () => {
  const failure = "dialog.message not allowed. Permissions associated with this command: dialog:allow-message";
  ports.invoke.mockRejectedValueOnce(failure);
  const pending = confirm("Synthetic import content");
  await expect(pending).rejects.toMatchObject({code: "PLATFORM_CONFIRMATION_FAILED", cause: failure,
    diagnostic: {cause: "permission_denied"}});
  await expect(pending).rejects.toThrow(/adapter=plugin:dialog\|message；cause=permission_denied/u);
});
