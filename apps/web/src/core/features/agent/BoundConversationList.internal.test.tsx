/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";
vi.mock("../../services/notifications", () => ({notify: {error: vi.fn(), success: vi.fn()}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("separates internal chats, selection recovery, search and archives while keeping them selectable", async () => {
  const internal = {id: "internal", title: "结构化阅读 · Paper", internal: true, archived: false, durableCount: 5,
    createdAt: "2026-10-17T00:00:00.000Z", updatedAt: "2026-10-17T00:00:00.000Z"};
  const list = vi.fn(async (request) => ({schemaVersion: 1 as const, conversations: request.internal && !request.archived ? [internal] : [], next: null}));
  const search = vi.fn(async request => ({schemaVersion: 1 as const, hits: [{conversation: internal, excerpt: request.term,
    itemId: null, turnId: null, startSeq: null, completed: true}], next: null}));
  const select = vi.fn();
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  try {
    await act(async () => root.render(<BoundConversationList binding={null} selectedId={internal.id} onSelect={select}
      catalog={{list, rename: vi.fn(), renameStatus: vi.fn()}} metadata={{read: async () => internal}} search={{search}} />));
    expect(container.querySelector('[data-conversation-id="internal"]')).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="内部对话"]')!.click());
    expect(list).toHaveBeenLastCalledWith(expect.objectContaining({internal: true, archived: false, binding: null}));
    expect(container.querySelector('[aria-label="新对话"]')).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[data-conversation-id="internal"] > button')!.click());
    expect(select).toHaveBeenCalledWith("internal", false);
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="搜索对话"]')!.click());
    const input = container.querySelector<HTMLInputElement>('input[type="search"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, "Paper");
      input.dispatchEvent(new Event("input", {bubbles: true}));
    });
    await act(async () => input.closest("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({term: "Paper", scope: expect.objectContaining({internal: true})}));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="查看归档对话"]')!.click());
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({scope: expect.objectContaining({internal: true, archived: true})}));
    expect(container.querySelector('[aria-label="清空归档"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="返回普通对话"]')).toBeNull();
    expect(container.querySelectorAll('.conversation-list-toolbar .lucide-arrow-left')).toHaveLength(1);
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="返回当前对话"]')!.click());
    expect(search).toHaveBeenLastCalledWith(expect.objectContaining({scope: expect.objectContaining({internal: true, archived: false})}));
    expect(container.querySelectorAll('.conversation-list-toolbar .lucide-arrow-left')).toHaveLength(1);
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="返回普通对话"]')!.click());
    expect(container.querySelector('[data-conversation-id="internal"]')).toBeNull();
    expect(container.querySelector('[aria-label="新对话"]')).not.toBeNull();
    expect(select).toHaveBeenCalledTimes(1);
  } finally { await act(async () => root.unmount()); container.remove(); }
});
