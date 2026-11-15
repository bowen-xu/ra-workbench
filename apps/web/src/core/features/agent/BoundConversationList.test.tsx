/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";
import type { AgentCatalogGateway } from "@research-assistant/data/agent";
import type { AgentRenameRequest } from "@research-assistant/core/agent";
import { renameProof, renameConversationId } from "./conversationRenameTestFacts";
import { agentCreationGateway } from "../../services/agentGateway";
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
vi.mock("../../services/agentGateway", async importOriginal => ({ ...await importOriginal<typeof import("../../services/agentGateway")>(), agentArchiveGateway: {}, agentCatalogGateway: {}, agentCreationGateway: {createBound: vi.fn(), createBoundStatus: vi.fn()}, agentDeletionGateway: {}, agentSearchGateway: {}, agentConversationExportGateway: {} }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("reuses an invisible local draft and only allows creation in the current view", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  const onSelect = vi.fn();
  const catalog: AgentCatalogGateway = {
    list: vi.fn(async () => ({ schemaVersion: 1 as const, conversations: [], next: null })),
    rename: vi.fn(), renameStatus: vi.fn(),
  };
  try {
    await act(async () => root.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "draft" }} selectedId={null} onSelect={onSelect} catalog={catalog} />));
    const create = container.querySelector<HTMLButtonElement>('[aria-label="新对话"]');
    await act(async () => { create?.click(); create?.click(); });
    expect(onSelect).toHaveBeenCalledTimes(2);
    const calls = onSelect.mock.calls;
    expect(calls[0]?.[0]).toBe(calls[1]?.[0]);
    expect(calls[0]?.[3]).toBe(true);
    expect(container.querySelector("[data-conversation-id]")).toBeNull();
    expect(container.textContent).not.toContain("输入消息后创建");
    expect(agentCreationGateway.createBound).not.toHaveBeenCalled();
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "查看归档对话")?.click());
    expect(container.querySelector('[aria-label="新对话"]')).toBeNull();
    expect(container.querySelector("[data-conversation-id]")).toBeNull();
    await act(async () => create?.click());
    expect(onSelect).toHaveBeenCalledTimes(2);
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "返回当前对话")?.click());
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="新对话"]')?.click());
    expect(onSelect).toHaveBeenLastCalledWith(calls[0]?.[0], false, undefined, true);
    expect(onSelect).toHaveBeenCalledTimes(3);
  } finally { await act(async () => root.unmount()); }
});

it("does not select a conversation after its host has unmounted", async () => {
  const container = document.createElement("div");
  const secondRoot = createRoot(container);
  const onSelect = vi.fn();
  const catalog: AgentCatalogGateway = { list: vi.fn(async () => ({ schemaVersion: 1 as const, conversations: [], next: null })), rename: vi.fn(), renameStatus: vi.fn() };
  await act(async () => secondRoot.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "original" }} selectedId={null} onSelect={onSelect} catalog={catalog} />));
  await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="新对话"]')?.click());
  await act(async () => secondRoot.unmount());
  expect(onSelect).toHaveBeenCalledWith(expect.any(String), false, undefined, true);
});

it.each([false, true])("updates a renamed row without selecting it and refreshes metadata only when selected (%s)", async (selected) => {
  const show = vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  const conversation = { id: renameConversationId, title: "Original", archived: false, durableCount: 0, createdAt: "2026-09-10T00:00:00.000Z", updatedAt: "2026-09-10T00:00:00.000Z" };
  const first = { schemaVersion: 1 as const, conversations: [conversation], next: { id: conversation.id, createdAt: conversation.createdAt } };
  let finishPage!: (page: typeof first) => void;
  const list = vi.fn().mockResolvedValueOnce(first).mockImplementationOnce(() => new Promise((resolve) => { finishPage = resolve; }));
  const rename = vi.fn(async (request: AgentRenameRequest) => renameProof(request));
  const metadata = { read: vi.fn(async () => ({ ...conversation, title: "Renamed" })) };
  const onSelect = vi.fn();
  const onRefreshSelected = vi.fn();
  try {
    await act(async () => root.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "draft" }} selectedId={selected ? conversation.id : null} onSelect={onSelect} onRefreshSelected={onRefreshSelected} metadata={metadata} catalog={{ list, rename, renameStatus: vi.fn() }} />));
    onRefreshSelected.mockClear();
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent?.includes("加载更多"))!.click());
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((button) => button.textContent?.includes("重命名"))!.click());
    const input = document.querySelector<HTMLInputElement>("dialog input")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "Renamed");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => document.querySelector("dialog form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(container.querySelector("strong")?.textContent).toBe("Renamed");
    await act(async () => finishPage(first));
    expect(container.querySelector("strong")?.textContent).toBe("Renamed");
    expect(onSelect).not.toHaveBeenCalled();
    expect(onRefreshSelected).toHaveBeenCalledTimes(selected ? 1 : 0);
    expect(container.querySelector('[aria-busy="true"]')).toBeNull();
  } finally { await act(async () => root.unmount()); container.remove(); show.mockRestore(); }
});

it.each([
  ["manuscript", "搜索当前稿件的对话"], ["paper", "搜索当前文献的对话"],
  ["note", "搜索当前笔记的对话"], ["readingNote", "搜索当前阅读笔记的对话"],
] as const)("keeps the search label and catalog scope aligned for %s", async (kind, label) => {
  const container = document.createElement("div"), root = createRoot(container);
  const list = vi.fn(async () => ({schemaVersion: 1 as const, conversations: [], next: null}));
  try {
    await act(async () => root.render(<BoundConversationList binding={{kind, resourceId: "same-id"}} selectedId={null}
      onSelect={vi.fn()} catalog={{list, rename: vi.fn(), renameStatus: vi.fn()}} />));
    expect(container.querySelector("input")).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="搜索对话"]')!.click());
    expect(container.querySelector(`[aria-label="${label}"]`)).not.toBeNull();
    expect(container.querySelector("p")).toBeNull();
    expect(container.textContent).not.toContain("刷新列表");
    expect(list).toHaveBeenCalledWith(expect.objectContaining({binding: {kind, resourceId: "same-id"}}));
  } finally { await act(async () => root.unmount()); }
});

it("re-reads a selected conversation when a paginated list temporarily omits it", async () => {
  const id = "11111111-1111-4111-8111-111111111111";
  const conversation = { id, title: "恢复中的对话", archived: false, durableCount: 0, createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" };
  const catalog: AgentCatalogGateway = { list: vi.fn(async () => ({ schemaVersion: 1 as const, conversations: [], next: null })), rename: vi.fn(), renameStatus: vi.fn() };
  const metadata = { read: vi.fn(async () => conversation) };
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  try {
    await act(async () => root.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "draft" }} selectedId={id} onSelect={vi.fn()} catalog={catalog} metadata={metadata} />));
    expect(metadata.read).toHaveBeenCalledWith({ id, binding: { kind: "manuscript", resourceId: "draft" } });
    expect(container.querySelector(`[data-conversation-id="${id}"]`)).not.toBeNull();
    expect(container.textContent).toContain("恢复中的对话");
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it("shows pin and archive actions and opens the remaining actions from a context menu", async () => {
  const conversations = [
    { id: "first", title: "First", archived: false, durableCount: 0, createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" },
    { id: "second", title: "Second", archived: false, durableCount: 0, createdAt: "2026-01-02T00:00:00.000Z", updatedAt: "2026-01-02T00:00:00.000Z" },
  ];
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  const catalog: AgentCatalogGateway = { list: vi.fn(async () => ({ schemaVersion: 1 as const, conversations, next: null })), rename: vi.fn(), renameStatus: vi.fn() };
  try {
    await act(async () => root.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "pin-test" }} selectedId={null} onSelect={vi.fn()} catalog={catalog} />));
    expect(container.querySelector('[aria-label="置顶对话"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="归档对话"]')).not.toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[data-conversation-id="second"] [aria-label="置顶对话"]')?.click());
    expect([...container.querySelectorAll<HTMLElement>("[data-conversation-id]")].map((row) => row.dataset.conversationId)).toEqual(["second", "first"]);
    const second = container.querySelector<HTMLElement>('[data-conversation-id="second"]')!;
    await act(async () => second.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true })));
    expect([...document.querySelectorAll('[role="menuitem"]')].some((item) => item.textContent?.includes("重命名对话"))).toBe(true);
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it("uses permanent delete actions instead of pinning in the archived view", async () => {
  const conversation = { id: "archived", title: "Archived", archived: true, durableCount: 0, createdAt: "2026-01-01T00:00:00.000Z", updatedAt: "2026-01-01T00:00:00.000Z" };
  const container = document.createElement("div"); const root = createRoot(container);
  const catalog: AgentCatalogGateway = { list: vi.fn(async () => ({ schemaVersion: 1 as const, conversations: [conversation], next: null })), rename: vi.fn(), renameStatus: vi.fn() };
  try {
    await act(async () => root.render(<BoundConversationList binding={{ kind: "manuscript", resourceId: "archived-test" }} selectedId={null} onSelect={vi.fn()} catalog={catalog} />));
    await act(async () => [...container.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.getAttribute("aria-label") === "查看归档对话")?.click());
    expect(container.querySelector('[aria-label="永久删除对话"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="置顶对话"]')).toBeNull();
  } finally { await act(async () => root.unmount()); }
});
