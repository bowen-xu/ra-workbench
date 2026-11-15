/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";
import type { AgentSearchGateway, AgentCatalogGateway } from "@research-assistant/data/agent";
import type { AgentSearchPage } from "@research-assistant/core/agent";
vi.mock("../../services/agentGateway", async importOriginal => ({ ...await importOriginal<typeof import("../../services/agentGateway")>(), agentArchiveGateway: {}, agentCatalogGateway: {}, agentCreationGateway: {}, agentDeletionGateway: {}, agentSearchGateway: {}, agentConversationExportGateway: {} }));
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
afterEach(async () => { await act(async () => root.unmount()); document.body.replaceChildren(); });
const conversation = { id: "owner", title: "Research", archived: false, durableCount: 13, createdAt: "2026-09-10T00:00:00.000Z", updatedAt: "2026-09-10T00:00:00.000Z" };
const hit = { conversation, excerpt: "跨片段 <script>literal</script>", itemId: "item", turnId: "turn", startSeq: 2, completed: true };
const page: AgentSearchPage = { schemaVersion: 1, hits: [hit], next: null };
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((el) => el.getAttribute("aria-label") === label || el.textContent === label)!;
const input = () => document.querySelector<HTMLInputElement>('input[type="search"]')!;
async function type(value: string) { await act(async () => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input(), value);
  input().dispatchEvent(new Event("input", { bubbles: true }));
}); }
async function mount(search: AgentSearchGateway["search"], open = true) {
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const list = vi.fn<AgentCatalogGateway["list"]>(async () => ({ schemaVersion: 1 as const, conversations: [conversation], next: null }));
  const onSelect = vi.fn();
  await act(async () => root.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "original" }} selectedId={null}
    onSelect={onSelect} search={{ search }} catalog={{ list, rename: vi.fn(), renameStatus: vi.fn() }} />));
  if (open) await act(async () => button("搜索对话").click());
  return { list, onSelect };
}
it("submits scoped literal search, renders plain excerpts and selects exact message identity", async () => {
  const search = vi.fn<AgentSearchGateway["search"]>(async (request) => ({ ...page, hits: [{ ...hit, conversation: { ...conversation, archived: request.scope.archived } }] }));
  const { onSelect, list } = await mount(search);
  await type("  跨片段  ");
  expect(search).not.toHaveBeenCalled();
  await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
  expect(search).toHaveBeenCalledExactlyOnceWith({ term: "跨片段", scope: { archived: false, limit: 50, before: null, binding: { kind: "manuscript", resourceId: "original" } } });
  expect(onSelect).not.toHaveBeenCalled(); expect(document.querySelector("script")).toBeNull();
  expect(document.querySelector(".conversation-search-excerpt")?.textContent).toBe(hit.excerpt);
  await act(async () => document.querySelector<HTMLButtonElement>(".conversation-list-row > button")!.click());
  expect(onSelect).toHaveBeenCalledExactlyOnceWith("owner", false, hit);
  await act(async () => button("查看归档对话").click());
  expect(search.mock.calls.at(-1)?.[0]).toMatchObject({ term: "跨片段", scope: { archived: true, before: null } });
  await act(async () => button("清空对话搜索").click());
  expect(list.mock.calls.at(-1)?.[0]).toMatchObject({ archived: true, before: null });
  expect(document.activeElement).toBe(input());
});
it("discards older search pages, preserves filters on retry and cannot write after host unmount", async () => {
  let finish!: (page: AgentSearchPage) => void;
  const search = vi.fn().mockImplementationOnce(() => new Promise<AgentSearchPage>((resolve) => { finish = resolve; }))
    .mockRejectedValueOnce("unavailable").mockResolvedValue(page);
  await mount(search);
  await type("old"); await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
  await type("new"); await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
  await act(async () => finish({ ...page, hits: [{ ...hit, excerpt: "stale" }] }));
  expect(document.body.textContent).not.toContain("stale");
  await act(async () => button("重新加载对话").click());
  expect(search.mock.calls.at(-1)?.[0]).toMatchObject({ term: "new" });
  expect(document.body.textContent).toContain(hit.excerpt);
  search.mockImplementationOnce(() => new Promise<AgentSearchPage>((resolve) => { finish = resolve; }));
  await type("late"); await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
  await act(async () => root.unmount());
  await act(async () => finish(page));
  expect(document.querySelector(".conversation-list-row")).toBeNull();
});
it("keeps IME confirmation out of search and clears with Escape without bubbling", async () => {
  const search = vi.fn<AgentSearchGateway["search"]>(async (request) => ({ ...page, hits: [{ ...hit, conversation: { ...conversation, archived: request.scope.archived } }] }));
  await mount(search); await type("组合输入");
  await act(async () => input().dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })));
  await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
  expect(search).not.toHaveBeenCalled();
  await act(async () => input().dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
  await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(search).toHaveBeenCalledOnce();
  const bubble = vi.fn(); document.body.addEventListener("keydown", bubble);
  await act(async () => input().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
  expect(bubble).not.toHaveBeenCalled(); expect(input()).toBeNull(); expect(document.activeElement).toBe(button("搜索对话"));
  document.body.removeEventListener("keydown", bubble);
  await act(async () => button("搜索对话").click());
  await type("🙂".repeat(201)); await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
  expect(input().getAttribute("aria-invalid")).toBe("true"); expect(search).toHaveBeenCalledOnce();
});

it("reveals an explicitly opened branch without selecting again or accepting stale search results", async () => {
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const child = { ...conversation, id: "child", title: "Branch", archived: true };
  let finish!: (page: AgentSearchPage) => void;
  const search = vi.fn<AgentSearchGateway["search"]>(() => new Promise((resolve) => { finish = resolve; }));
  const list = vi.fn<AgentCatalogGateway["list"]>(async (scope) => ({ schemaVersion: 1, conversations: scope.archived ? [child] : [conversation], next: null }));
  const onSelect = vi.fn();
  const render = (reveal: typeof child | null) => root.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "original" }}
    selectedId={reveal?.id ?? "owner"} revealConversation={reveal} onSelect={onSelect} search={{ search }} catalog={{ list, rename: vi.fn(), renameStatus: vi.fn() }} />);
  await act(async () => render(null)); await act(async () => button("搜索对话").click()); await type("Pending search"); await act(async () => document.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
  await act(async () => render(child));
  expect(input().value).toBe(""); expect(button("返回当前对话")).toBeDefined();
  expect(document.querySelector('[data-conversation-id="child"] [aria-current="true"]')).not.toBeNull();
  await act(async () => finish(page)); expect(document.body.textContent).not.toContain("跨片段");
  expect(onSelect).not.toHaveBeenCalled();
  await type("New search draft"); const calls = list.mock.calls.length;
  await act(async () => render(child)); expect(input().value).toBe("New search draft"); expect(list).toHaveBeenCalledTimes(calls);
});
