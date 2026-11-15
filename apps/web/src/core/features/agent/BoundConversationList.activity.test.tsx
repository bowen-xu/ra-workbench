/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";
import { invalidateConversationCatalog } from "../../runtime/conversationCatalogEvents";
vi.mock("../../services/notifications", () => ({notify: {error: vi.fn(), success: vi.fn()}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("shows running rows and off-page internal activity without changing catalog filters or selection", async () => {
  const chat = {id: "chat", title: "Chat", archived: false, durableCount: 1, createdAt: "now", updatedAt: "now"};
  let running = [{id: "chat", internal: false}, {id: "off-page-internal", internal: true}];
  const activity = vi.fn(async () => running), list = vi.fn(async () => ({schemaVersion: 1 as const, conversations: [chat], next: null}));
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host), select = vi.fn();
  try {
    await act(async () => root.render(<BoundConversationList binding={null} selectedId="chat" onSelect={select} catalog={{activity, list, rename: vi.fn(), renameStatus: vi.fn()}}/>));
    const internal = host.querySelector<HTMLButtonElement>('[aria-label="内部对话"]')!;
    expect(internal.getAttribute("aria-busy")).toBe("true");
    expect(internal.querySelector(".lucide-bot")).not.toBeNull();
    expect(internal.querySelector(".agent-launcher-spinner")).not.toBeNull();
    expect(host.querySelector('[aria-label="对话正在运行"]')).not.toBeNull();
    expect(list).toHaveBeenCalledWith(expect.not.objectContaining({internal: true}));
    running = [];
    await act(async () => { invalidateConversationCatalog(); await new Promise(resolve => setTimeout(resolve, 10)); });
    expect(internal.getAttribute("aria-busy")).toBe("false");
    expect(internal.querySelector(".lucide-bot")).not.toBeNull();
    expect(host.querySelector(".agent-launcher-spinner")).toBeNull();
    expect(select).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); host.remove(); }
});

it("pauses activity reads while hidden and coalesces invalidations during a pending read", async () => {
  vi.useFakeTimers();
  let finish!: (value: []) => void;
  const activity = vi.fn(() => new Promise<[]>(resolve => {finish = resolve;}));
  const catalog = {activity, list: async () => ({schemaVersion: 1 as const, conversations: [], next: null}), rename: vi.fn(), renameStatus: vi.fn()};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    const render = (hidden: boolean) => root.render(<div hidden={hidden}><BoundConversationList binding={null} selectedId={null} onSelect={vi.fn()} catalog={catalog}/></div>);
    await act(async () => render(true));
    await act(async () => vi.advanceTimersByTimeAsync(2000));
    expect(activity).not.toHaveBeenCalled();
    await act(async () => render(false));
    await act(async () => vi.advanceTimersByTimeAsync(1000));
    expect(activity).toHaveBeenCalledTimes(1);
    await act(async () => { invalidateConversationCatalog(); invalidateConversationCatalog(); await vi.advanceTimersByTimeAsync(10); });
    expect(activity).toHaveBeenCalledTimes(1);
    await act(async () => { finish([]); await vi.advanceTimersByTimeAsync(10); });
    expect(activity).toHaveBeenCalledTimes(2);
    await act(async () => root.unmount());
    await act(async () => { finish([]); await vi.advanceTimersByTimeAsync(2000); });
    expect(activity).toHaveBeenCalledTimes(2);
  } finally { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); }
});
