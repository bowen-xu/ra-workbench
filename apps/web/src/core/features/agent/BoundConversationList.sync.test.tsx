// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentCatalogGateway, AgentSearchGateway } from "@research-assistant/data/agent";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import { BoundConversationList } from "./BoundConversationList";
import { invalidateConversationCatalog } from "../../runtime/conversationCatalogEvents";

vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
vi.mock("../../services/agentGateway", async importOriginal => ({ ...await importOriginal<typeof import("../../services/agentGateway")>(), agentArchiveGateway: {}, agentCatalogGateway: {}, agentDeletionGateway: {}, agentSearchGateway: {} }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const cleanup: (() => Promise<void>)[] = [];
afterEach(async () => { for (const stop of cleanup.splice(0)) await stop(); vi.restoreAllMocks(); });
const row = (id: string, title = id): AgentConversationSummary => ({ id, title, archived: false, durableCount: 0, createdAt: "now", updatedAt: "now" });
const page = (rows: readonly AgentConversationSummary[], next: string | null = null) => ({ schemaVersion: 1 as const, conversations: rows, next: next ? { id: next, createdAt: "now" } : null });
async function flush() { await act(async () => { await new Promise(resolve => setTimeout(resolve, 10)); }); }
async function mount(list: AgentCatalogGateway["list"], search?: AgentSearchGateway) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), onSelect = vi.fn();
  cleanup.push(async () => { await act(async () => root.unmount()); host.remove(); });
  await act(async () => root.render(<BoundConversationList selectedId={null} binding={null} onSelect={onSelect}
    catalog={{ list, rename: vi.fn(), renameStatus: vi.fn() }} {...(search ? {search} : {})} />));
  const button = (name: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(node => (node.getAttribute("aria-label") ?? node.textContent) === name)!;
  return { host, button, onSelect, root };
}

it("does not reload the catalog when an unrelated file tab changes", async () => {
  const list=vi.fn<AgentCatalogGateway["list"]>(async()=>page([row("one")]));
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host),onSelect=vi.fn();
  cleanup.push(async()=>{await act(async()=>root.unmount());host.remove();});
  for(const tab of ["paper-a","paper-b","paper-a"]){
    await act(async()=>root.render(<div data-file-tab={tab}><BoundConversationList selectedId={null} binding={null}
      onSelect={onSelect} catalog={{list,rename:vi.fn(),renameStatus:vi.fn()}}/></div>));
  }
  expect(list).toHaveBeenCalledTimes(1);
});

it("coalesces changes, keeps loaded pagination and scroll, and removes the permanent refresh control", async () => {
  let title = "Original";
  const list = vi.fn<AgentCatalogGateway["list"]>(async query => query.before ? page([row("older")]) : page([row("first", title)], "first"));
  const { host, button, onSelect } = await mount(list);
  expect(host.textContent).not.toContain("刷新列表");
  await act(async () => button("加载更多").click());
  const section = host.querySelector("section")!; section.scrollTop = 123;
  list.mockClear(); title = "Changed";
  act(() => { invalidateConversationCatalog(); invalidateConversationCatalog(); }); await flush();
  expect(list).toHaveBeenCalledTimes(2);
  expect(host.textContent).toContain("Changed"); expect(host.textContent).toContain("older");
  expect(section.scrollTop).toBe(123); expect(onSelect).not.toHaveBeenCalled();
  expect(host.querySelector('[role="status"]')).toBeNull();
});

it("preserves the current search and archive filter when returning to the foreground", async () => {
  const list = vi.fn<AgentCatalogGateway["list"]>(async () => page([]));
  const search = vi.fn<AgentSearchGateway["search"]>(async () => ({ schemaVersion: 1, hits: [], next: null }));
  const { host, button } = await mount(list, { search });
  await act(async () => button("查看归档对话").click());
  await act(async () => button("搜索对话").click());
  const input = host.querySelector("input")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "query");
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  search.mockClear();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
  invalidateConversationCatalog(); await flush(); expect(search).not.toHaveBeenCalled();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  window.dispatchEvent(new Event("focus")); document.dispatchEvent(new Event("visibilitychange")); await flush();
  expect(search).toHaveBeenCalledExactlyOnceWith({ term: "query", scope: { archived: true, before: null, limit: 50, binding: null } });
  expect(input.value).toBe("query"); expect(button("返回当前对话")).toBeDefined();
});

it("waits for a requested next page before synchronizing instead of losing that page", async () => {
  let finish!: (value: ReturnType<typeof page>) => void;
  const list = vi.fn<AgentCatalogGateway["list"]>().mockResolvedValueOnce(page([row("first")], "first"))
    .mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }))
    .mockResolvedValueOnce(page([row("first", "Updated")], "first")).mockResolvedValue(page([row("second")]));
  const { host, button } = await mount(list);
  await act(async () => button("加载更多").click());
  invalidateConversationCatalog(); await flush(); expect(list).toHaveBeenCalledTimes(2);
  await act(async () => finish(page([row("second")]))); await flush();
  expect(list).toHaveBeenCalledTimes(4); expect(host.textContent).toContain("Updated"); expect(host.textContent).toContain("second");
});

it("retains existing rows and offers retry on read failure, then stops subscriptions after unmount", async () => {
  const list = vi.fn<AgentCatalogGateway["list"]>().mockResolvedValueOnce(page([row("first")])).mockRejectedValueOnce("offline").mockResolvedValue(page([row("first", "Recovered")]));
  const { host, button, root } = await mount(list);
  invalidateConversationCatalog(); await flush();
  expect(host.textContent).toContain("first");
  await act(async () => button("重新加载对话").click()); expect(host.textContent).toContain("Recovered");
  await act(async () => root.unmount()); cleanup.pop(); host.remove();
  list.mockClear(); invalidateConversationCatalog(); window.dispatchEvent(new Event("focus")); await flush();
  expect(list).not.toHaveBeenCalled();
});
