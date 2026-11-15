// @vitest-environment happy-dom
import {afterEach, beforeEach, describe, expect, it} from "vitest";
import {getAppStorage as getSdkStorage} from "@research-assistant/workbench-sdk/storage";
import {getRuntimeEnvironment, getRuntimeStorage} from "@research-assistant/workbench-sdk/environment";
import {selectAppRuntime} from "../runtime/appRuntime";
import {appStorageScopedKey, registerAppStoragePrefix, selectAppStorage} from "../runtime/appStorage";
import {getAppStorage as getCoreStorage} from "./storage";

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {return values.size;},
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);},
    key: index => [...values.keys()][index] ?? null,
    clear: () => {values.clear();},
  };
}

function select(storage: Storage, mode: "browser" | "preview" = "preview"): void {
  selectAppRuntime({mode, storage, gateways: {}, dispose() {}});
}

beforeEach(() => select(memoryStorage()));
afterEach(() => select(memoryStorage(), "browser"));

describe("Core storage SDK binding", () => {
  it("retains captured runtime storage when Core separately selects another global storage", () => {
    const first = memoryStorage(), second = memoryStorage();
    select(first);
    const captured = getRuntimeEnvironment();
    selectAppStorage(second);
    expect(getSdkStorage()).toBe(second);
    expect(getCoreStorage()).toBe(second);
    expect(getRuntimeStorage(captured)).toBe(first);
    expect(getRuntimeStorage(getRuntimeEnvironment())).toBe(first);
    getRuntimeStorage(captured).setItem("synthetic-owner", "first");
    expect(first.getItem("synthetic-owner")).toBe("first");
    expect(second.getItem("synthetic-owner")).toBeNull();
    select(second, "browser");
    expect(getRuntimeStorage(captured)).toBe(first);
    expect(getRuntimeStorage(getRuntimeEnvironment())).toBe(second);
  });

  it("returns the exact original Core function and the currently selected writable instance", () => {
    const storage = memoryStorage();
    select(storage);
    expect(getSdkStorage).toBe(getCoreStorage);
    expect(getSdkStorage()).toBe(storage);
    expect(getSdkStorage()).toBe(getCoreStorage());
    getSdkStorage().setItem("synthetic-key", "synthetic-value");
    expect(storage.getItem("synthetic-key")).toBe("synthetic-value");
    expect(getSdkStorage().length).toBe(1);
    expect(getSdkStorage().key(0)).toBe("synthetic-key");
    getSdkStorage().removeItem("synthetic-key");
    expect(storage.getItem("synthetic-key")).toBeNull();
  });

  it("reads the new owner each time while a captured storage keeps its original namespace and lifetime", () => {
    const first = memoryStorage(), second = memoryStorage();
    registerAppStoragePrefix(first, "synthetic-first:");
    registerAppStoragePrefix(second, "synthetic-second:");
    select(first);
    const captured = getSdkStorage(), oldView = getRuntimeEnvironment();
    expect(Reflect.ownKeys(oldView)).toEqual(["mode", "hasStructuredResourceGateway"]);
    expect(Object.isFrozen(oldView)).toBe(true);
    expect(oldView).not.toHaveProperty("storage");
    select(second, "browser");
    expect(getSdkStorage()).toBe(second);
    expect(captured).toBe(first);
    captured.setItem("synthetic-key", "first");
    getSdkStorage().setItem("synthetic-key", "second");
    expect(first.getItem("synthetic-key")).toBe("first");
    expect(second.getItem("synthetic-key")).toBe("second");
    expect(appStorageScopedKey(captured, "event")).toBe("synthetic-first:event");
    expect(appStorageScopedKey(getSdkStorage(), "event")).toBe("synthetic-second:event");
    const newView = getRuntimeEnvironment();
    expect(newView).not.toBe(oldView);
    expect(Reflect.ownKeys(newView)).toEqual(["mode", "hasStructuredResourceGateway"]);
    expect(oldView.mode).toBe("preview");
    expect(newView.mode).toBe("browser");
    captured.clear();
    expect(second.getItem("synthetic-key")).toBe("second");
  });

  it("preserves the original Storage operation errors without a proxy or replacement store", () => {
    const failure = Object.assign(new Error("Synthetic storage failure"), {code: "synthetic_storage_failure"});
    const storage = memoryStorage();
    storage.setItem = () => {throw failure;};
    select(storage);
    expect(getSdkStorage()).toBe(storage);
    expect(() => getSdkStorage().setItem("synthetic-key", "value")).toThrow(failure);
  });
});
