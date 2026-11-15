// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { selectAppStorage } from "../../runtime/appStorage";
import { discoverConversationForks, saveConversationFork, clearConversationFork, subscribeConversationForks } from "./conversationForkPending";
const binding = { kind: "note" as const, resourceId: "case:sensitive/笔记" };
function record() { return { schemaVersion: 1 as const, intent: { parentId: crypto.randomUUID(), childId: crypto.randomUUID(), throughTurnId: crypto.randomUUID(), binding }, epoch: crypto.randomUUID(), stage: "unknown" as const }; }
afterEach(() => { localStorage.clear(); });
it("discovers only exact binding records after their parent/turn disappears; discovery never writes or drops malformed records", () => {
  const one = record(), another = record(); selectAppStorage(localStorage);
  saveConversationFork(localStorage, one, true); saveConversationFork(localStorage, another, true);
  const foreign = { ...record(), intent: { ...record().intent, binding: { ...binding, resourceId: "Case:sensitive/笔记" } } };
  saveConversationFork(localStorage, foreign, true);
  const malformed = `research-assistant:conversation-fork:v1:${binding.kind}:${encodeURIComponent(binding.resourceId)}:not-valid`;
  localStorage.setItem(malformed, "PRIVATE_INVALID_BODY");
  const size = localStorage.length, report = vi.fn(), write = vi.spyOn(localStorage, "setItem"), remove = vi.spyOn(localStorage, "removeItem");
  try {
    expect(discoverConversationForks(localStorage, binding, report)).toEqual([one, another]);
    expect(report).toHaveBeenCalledExactlyOnceWith(new Error("FORK_RECOVERY_INVALID"));
    expect(write).not.toHaveBeenCalled(); expect(remove).not.toHaveBeenCalled();
    expect(localStorage.length).toBe(size); expect(localStorage.getItem(malformed)).toBe("PRIVATE_INVALID_BODY");
    expect(discoverConversationForks(localStorage, { ...binding, kind: "readingNote" }, report)).toEqual([]);
  } finally { write.mockRestore(); remove.mockRestore(); }
});
it("invalidates same-view saves and removals, ignores foreign storage, and detaches listeners", () => {
  selectAppStorage(localStorage); const changed = vi.fn(), detach = subscribeConversationForks(localStorage, binding, changed), one = record();
  saveConversationFork(sessionStorage, one, true); expect(changed).not.toHaveBeenCalled();
  saveConversationFork(localStorage, one, true); expect(changed).toHaveBeenCalledOnce();
  clearConversationFork(localStorage, one); expect(changed).toHaveBeenCalledTimes(2);
  window.dispatchEvent(new StorageEvent("storage", { key: "research-assistant:conversation-fork:v1:note:another:anything" }));
  expect(changed).toHaveBeenCalledTimes(2);
  window.dispatchEvent(new StorageEvent("storage", { key: `research-assistant:conversation-fork:v1:note:${encodeURIComponent(binding.resourceId)}:anything` }));
  expect(changed).toHaveBeenCalledTimes(3); detach(); window.dispatchEvent(new Event("focus"));
  expect(changed).toHaveBeenCalledTimes(3); sessionStorage.clear();
});
