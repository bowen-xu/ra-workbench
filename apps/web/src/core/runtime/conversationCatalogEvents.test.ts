// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { runtimeAgentGateway, selectAppRuntime, type AppRuntime } from "./appRuntime";
import { createTestNamespacedStorage } from "./testing/createTestNamespacedStorage";
import { subscribeConversationCatalog } from "./conversationCatalogEvents";

const cleanup: (() => void)[] = [];
function memory(): Storage {
  const values = new Map<string, string>();
  return { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);}, removeItem: key => {values.delete(key);}, clear: () => values.clear() };
}
const runtime = (values: Partial<AppRuntime> = {}): AppRuntime => ({ mode: "browser", storage: memory(), gateways: {}, dispose() {}, ...values });
afterEach(() => { cleanup.splice(0).forEach(stop => stop()); selectAppRuntime(runtime()); });
function observe() { const changed = vi.fn(); cleanup.push(subscribeConversationCatalog(changed)); return changed; }

it("invalidates successful and uncertain mutations, but never directory reads", async () => {
  selectAppRuntime(runtime());
  const changed = observe();
  const gateway = runtimeAgentGateway("agentCatalogGateway", { list: vi.fn(async () => []), rename: vi.fn(async () => "renamed") });
  await gateway.list(); expect(changed).not.toHaveBeenCalled();
  expect(await gateway.rename()).toBe("renamed"); expect(changed).toHaveBeenCalledTimes(1);
  const creation = runtimeAgentGateway("agentCreationGateway", { createBound: vi.fn(async () => "created") });
  expect(await creation.createBound()).toBe("created"); expect(changed).toHaveBeenCalledTimes(2);
  const archive = runtimeAgentGateway("agentArchiveGateway", { setArchived: async () => { throw "lost-response"; } });
  await expect(archive.setArchived()).rejects.toBe("lost-response");
  expect(changed).toHaveBeenCalledTimes(3);
});

it("does not invalidate the catalog or send delete after readonly deletion status", async () => {
  selectAppRuntime(runtime()); const changed = observe();
  const remove = vi.fn(), status = vi.fn().mockResolvedValue({type: "noSqlReceipt"});
  const gateway = runtimeAgentGateway("agentDeletionGateway", {delete: remove, status});
  await gateway.status(); status.mockRejectedValueOnce("status unavailable");
  await expect(gateway.status()).rejects.toBe("status unavailable");
  expect(remove).not.toHaveBeenCalled(); expect(changed).not.toHaveBeenCalled();
});

it("observes actual preview ports including nested session creation, without invoking native fallbacks", async () => {
  const create = vi.fn(async () => "created"), native = vi.fn();
  selectAppRuntime(runtime({ mode: "preview", gateways: { agentContextGateway: { sessions: { create } } as never } }));
  const changed = observe();
  const gateway = runtimeAgentGateway("agentContextGateway", { sessions: { create: native } });
  expect(gateway.sessions).toBe(gateway.sessions);
  expect(await gateway.sessions.create()).toBe("created");
  expect(changed).toHaveBeenCalledOnce(); expect(native).not.toHaveBeenCalled();
});

it("isolates storage notifications by runtime namespace and discards late mutations from a replaced runtime", async () => {
  const storage = createTestNamespacedStorage(memory(), "isolated:");
  selectAppRuntime(runtime({ storage }));
  const changed = observe();
  window.dispatchEvent(new StorageEvent("storage", { key: "research-assistant:conversation-catalog-revision" }));
  expect(changed).not.toHaveBeenCalled();
  window.dispatchEvent(new StorageEvent("storage", { key: "isolated:research-assistant:conversation-catalog-revision" }));
  expect(changed).toHaveBeenCalledOnce();
  let finish!: () => void;
  const gateway = runtimeAgentGateway("agentDeletionGateway", { delete: () => new Promise<void>(resolve => { finish = resolve; }) });
  const pending = gateway.delete(); selectAppRuntime(runtime());
  finish(); await pending;
  expect(changed).toHaveBeenCalledOnce();
});
