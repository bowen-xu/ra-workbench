// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { browserConversationForkLock, forkLockName } from "./conversationForkLock";
import { registerAppStoragePrefix, selectAppStorage } from "../../runtime/appStorage";
afterEach(() => { vi.unstubAllGlobals(); });
it("uses an exclusive nonqueued lock and holds it until the entire operation settles", async () => {
  const held = new Set<string>();
  const request = vi.fn(async (name: string, options: unknown, callback: (value: object | null) => Promise<unknown>) => {
    expect(options).toEqual({mode:"exclusive",ifAvailable:true});
    const acquired = !held.has(name); if (acquired) held.add(name);
    try { return await callback(acquired ? {name, mode:"exclusive"} : null); }
    finally { if (acquired) held.delete(name); }
  });
  vi.stubGlobal("navigator", {locks:{request}});
  let release!: () => void; const work = browserConversationForkLock.run("synthetic", async () => {
    await new Promise<void>(resolve => { release = resolve; }); return "saved";
  });
  const duplicate = vi.fn(async () => "must not run");
  await expect(browserConversationForkLock.run("synthetic", duplicate)).rejects.toThrow("FORK_RECOVERY_LOCK_BUSY");
  expect(duplicate).not.toHaveBeenCalled(); expect(held.has("synthetic")).toBe(true);
  release(); expect(await work).toBe("saved"); expect(held.size).toBe(0);
  await expect(browserConversationForkLock.run("synthetic", async () => { throw Error("operation failed"); })).rejects.toThrow("operation failed");
  expect(await browserConversationForkLock.run("synthetic", async () => "retry")).toBe("retry");
});
it("has no unlocked or volatile fallback if the standard lock API is missing", async () => {
  vi.stubGlobal("navigator", {}); const operation = vi.fn(async () => "must not run");
  await expect(browserConversationForkLock.run("synthetic", operation)).rejects.toThrow("FORK_RECOVERY_LOCK_UNAVAILABLE");
  expect(operation).not.toHaveBeenCalled();
});
it("names locks using the captured storage namespace, binding and original boundary", () => {
  registerAppStoragePrefix(localStorage, "original:"); registerAppStoragePrefix(sessionStorage, "replacement:"); selectAppStorage(sessionStorage);
  const scope = {parentId:crypto.randomUUID(),throughTurnId:crypto.randomUUID(),binding:{kind:"note" as const,resourceId:"one"}};
  expect(forkLockName(localStorage, scope)).toMatch(/^original:/u);
  expect(forkLockName(localStorage, scope)).not.toBe(forkLockName(sessionStorage, scope));
  expect(forkLockName(localStorage, scope)).not.toBe(forkLockName(localStorage,{...scope,binding:{...scope.binding,resourceId:"One"}}));
  selectAppStorage(localStorage);
});
