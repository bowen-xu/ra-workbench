// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentCanvasHost, AgentCanvasShell } from "../agent/AgentCanvasShell";
import * as appStorage from "../../runtime/appStorage";
import { KeyboardShortcutSettings } from "./KeyboardShortcutSettings";
import { SHORTCUT_STORAGE_KEY } from "./ShortcutPreferences";
import { notify } from "../../services/notifications";

vi.mock("../agent/presentation/WritingAgentPanel", () => ({ WritingAgentPanel: () => null }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let cleanup: (() => Promise<void>) | undefined;
beforeEach(() => {
  vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");
  const values = new Map<string, string>();
  const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  vi.spyOn(appStorage, "getAppStorage").mockReturnValue(storage);
});
afterEach(async () => { await cleanup?.(); cleanup = undefined; vi.restoreAllMocks(); });
async function mount() {
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  function Resource(): React.JSX.Element {
    const [open, setOpen] = useState(false);
    return <AgentCanvasHost open={open} onOpenChange={setOpen} onNewConversation={vi.fn()} conversations={<input aria-label="搜索测试对话" />}>
      {controls => <section className="writing-agent-panel" hidden={!open}>{controls.headerControls}<input aria-label="聊天草稿" defaultValue="保留草稿" /></section>}
    </AgentCanvasHost>;
  }
  const app = <AgentCanvasShell><KeyboardShortcutSettings /><Resource /><input aria-label="源码输入" /></AgentCanvasShell>;
  const render = async () => { await act(async () => root.render(app)); };
  await render(); cleanup = async () => { await act(async () => root.unmount()); container.remove(); };
  const button = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(node => node.getAttribute("aria-label") === name || node.textContent === name)!;
  const press = async (target: Element, key: string, options: KeyboardEventInit = {}) => {
    const event = new KeyboardEvent("keydown", { key, metaKey: true, bubbles: true, cancelable: true, ...options });
    await act(async () => { target.dispatchEvent(event); }); return event;
  };
  return { container, root, app, button, press };
}
it("toggles from editor/composer focus, opens both for the list command, and yields during composition/modal/repeat", async () => {
  const view = await mount(), source = view.container.querySelector('[aria-label="源码输入"]')!;
  const panel = () => view.container.querySelector<HTMLElement>(".writing-agent-panel")!;
  expect(panel().hidden).toBe(true);
  const laterHandler = vi.fn(); source.addEventListener("keydown", laterHandler);
  const event = await view.press(source, "."); expect(event.defaultPrevented).toBe(true); expect(panel().hidden).toBe(false);
  expect(laterHandler).not.toHaveBeenCalled(); source.removeEventListener("keydown", laterHandler);
  const draft = view.container.querySelector<HTMLInputElement>('[aria-label="聊天草稿"]')!;
  await view.press(draft, "."); expect(panel().hidden).toBe(true); expect(draft.value).toBe("保留草稿");
  await view.press(source, "l"); expect(panel().hidden).toBe(false);
  const drawer = view.container.querySelector<HTMLElement>('[aria-label="对话列表面板"]')!;
  expect(drawer.hidden).toBe(false);
  await view.press(drawer.querySelector("input")!, "Escape", { metaKey: false });
  expect(drawer.hidden).toBe(false); expect(panel().hidden).toBe(false);
  await view.press(draft, "Escape", { metaKey: false });
  expect(drawer.hidden).toBe(false); expect(panel().hidden).toBe(false);
  await view.press(drawer.querySelector("input")!, "l"); expect(drawer.hidden).toBe(true); expect(panel().hidden).toBe(false);
  expect(document.activeElement?.getAttribute("aria-label")).toBe("对话列表");
  await view.press(draft, ".", { repeat: true }); await view.press(draft, ".", { isComposing: true });
  await view.press(draft, ".", { shiftKey: true }); expect(panel().hidden).toBe(false);
  const modal = document.createElement("dialog"); modal.setAttribute("open", ""); document.body.append(modal);
  try { expect((await view.press(draft, ".")).defaultPrevented).toBe(false); expect(panel().hidden).toBe(false); }
  finally { modal.remove(); }
  await view.press(draft, "."); expect(panel().hidden).toBe(true);
});
it("records without triggering commands, rejects duplicates/standard keys, persists new bindings and resets defaults", async () => {
  const view = await mount(), chat = view.button("修改打开／关闭聊天窗快捷键"), source = view.container.querySelector('[aria-label="源码输入"]')!;
  await act(async () => chat.click()); await view.press(chat, "l");
  expect(view.container.querySelector('[role="status"]')?.textContent).toContain("另一个操作");
  expect(view.container.querySelector<HTMLElement>(".writing-agent-panel")?.hidden).toBe(true);
  await view.press(chat, "q"); expect(view.container.querySelector('[role="status"]')?.textContent).toContain("系统");
  await view.press(chat, "Escape", { metaKey: false }); expect(chat.getAttribute("aria-pressed")).toBe("false");
  await act(async () => chat.click()); await view.press(chat, "k", { shiftKey: true });
  expect(chat.textContent).toBe("⇧ ⌘ K");
  expect(JSON.parse(appStorage.getAppStorage().getItem(SHORTCUT_STORAGE_KEY)!).toggleChat.key).toBe("k");
  await view.press(source, "."); expect(view.container.querySelector<HTMLElement>(".writing-agent-panel")?.hidden).toBe(true);
  await view.press(source, "k", { shiftKey: true }); expect(view.container.querySelector<HTMLElement>(".writing-agent-panel")?.hidden).toBe(false);
  await act(async () => view.root.render(null)); await act(async () => view.root.render(view.app));
  expect(view.button("修改打开／关闭聊天窗快捷键").textContent).toBe("⇧ ⌘ K");
  await act(async () => view.button("恢复默认快捷键").click());
  expect(view.button("修改打开／关闭聊天窗快捷键").textContent).toBe("⌘ .");
  await view.press(view.container.querySelector('[aria-label="源码输入"]')!, ".");
  expect(view.container.querySelector<HTMLElement>(".writing-agent-panel")?.hidden).toBe(false);
  await view.press(view.button("修改打开／关闭聊天窗快捷键"), ".");
  expect(view.container.querySelector<HTMLElement>(".writing-agent-panel")?.hidden).toBe(true);
});
it("keeps the active binding and reports a failed preference save", async () => {
  const view = await mount(), chat = view.button("修改打开／关闭聊天窗快捷键");
  const error = vi.spyOn(notify, "error").mockImplementation(() => "test");
  vi.spyOn(appStorage.getAppStorage(), "setItem").mockImplementation(() => { throw Error("unavailable"); });
  await act(async () => chat.click()); await view.press(chat, "k", { shiftKey: true });
  expect(chat.textContent).toBe("⇧ ⌘ K"); expect(error).toHaveBeenCalledOnce();
});

it("restores one modified shortcut while retaining the other custom binding, and cancels inline recording", async () => {
  const view = await mount();
  const chat = view.button("修改打开／关闭聊天窗快捷键"), conversations = view.button("修改打开／关闭对话列表快捷键");
  await act(async () => chat.click()); await view.press(chat, "k", { shiftKey: true });
  await act(async () => conversations.click()); await view.press(conversations, "j", { shiftKey: true });
  const restore = view.button("恢复打开／关闭聊天窗默认快捷键"); expect(restore).toBeDefined();
  await act(async () => restore.click());
  expect(chat.textContent).toBe("⌘ ."); expect(conversations.textContent).toBe("⇧ ⌘ J");
  expect(view.button("恢复打开／关闭聊天窗默认快捷键")).toBeUndefined();
  await act(async () => chat.click()); await act(async () => view.button("取消").click());
  expect(chat.textContent).toBe("⌘ .");
});

it("persists unassigned shortcuts and does not react to them until restored", async () => {
  const view = await mount();
  await act(async () => view.button("删除打开／关闭聊天窗快捷键").click());
  await act(async () => view.button("删除打开／关闭对话列表快捷键").click());
  expect(JSON.parse(appStorage.getAppStorage().getItem(SHORTCUT_STORAGE_KEY)!).toggleChat).toBeNull();
  await act(async () => view.root.render(null)); await act(async () => view.root.render(view.app));
  expect(view.button("修改打开／关闭聊天窗快捷键").textContent).toBe("未设置");
  await view.press(view.container.querySelector('[aria-label="源码输入"]')!, ".");
  expect(view.container.querySelector<HTMLElement>(".writing-agent-panel")?.hidden).toBe(true);
  await act(async () => view.button("恢复打开／关闭聊天窗默认快捷键").click());
  expect(view.button("修改打开／关闭聊天窗快捷键").textContent).toBe("⌘ .");
  expect(view.button("修改打开／关闭对话列表快捷键").textContent).toBe("未设置");
});
