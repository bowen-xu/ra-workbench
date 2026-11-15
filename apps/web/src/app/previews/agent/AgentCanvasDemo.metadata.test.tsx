/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";
import { notify } from "../../../core/services/notifications";
import { selectAppStorage } from "../../../core/runtime/appStorage";
beforeEach(() => {
  vi.spyOn(notify,"error");
  const values = new Map<string,string>();
  selectAppStorage({get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()});
});
afterEach(() => vi.restoreAllMocks());

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

async function withCanvas(check: (host: HTMLDivElement, button: (text: string) => HTMLButtonElement, scene: (value: string) => Promise<void>, draft: (text: string) => Promise<void>) => Promise<void>) {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const button = (text: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === text)!;
  const scene = async (value: string) => {
    await act(async () => {
      const select = host.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
      select.value = value; select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false); });
  };
  const draft = async (text: string) => { await act(async () => {
    const input = host.querySelector<HTMLTextAreaElement>("textarea")!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }); };
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector('[data-action="stop"]')).not.toBeNull(); });
    await check(host, button, scene, draft);
  } finally { await act(async () => root.unmount()); host.remove(); }
}

it("checks silently in the background and resumes a waiting send once after success", async () => {
  await withCanvas(async (host, button, scene, draft) => {
    await scene("metadataChecking");
    expect(host.textContent).not.toContain("正在核对对话状态");
    await draft("等待核对后发送");
    expect(host.querySelector<HTMLButtonElement>('[data-action="send"]')?.disabled).toBe(false);
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    expect(host.querySelector('.agent-composer-input-state [aria-label="输入框准备状态"]')).not.toBeNull();
    expect(host.querySelector('[data-action="stop"]')).toBeNull();
    await act(async () => button("完成模拟核对").click());
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector('[data-action="stop"]')).not.toBeNull(); });
    expect(host.querySelector('[aria-label="输入框准备状态"]')).toBeNull();
    expect([...host.querySelectorAll('.writing-agent-message.user')].filter(message => message.textContent?.includes("等待核对后发送"))).toHaveLength(1);
    await act(async () => button("重置假数据").click());
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false); });
    expect(host.textContent).not.toContain("正在核对对话状态");
    expect(host.querySelector('[data-action="stop"]')).toBeNull();
  });
});

it("preserves the draft on a failed check, retries the check, then permits a new send", async () => {
  await withCanvas(async (host, button, scene, draft) => {
    await scene("metadataFailed"); await draft("失败时保留的消息");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    expect(host.querySelector('[aria-label="输入框准备状态"]')).not.toBeNull();
    await act(async () => button("完成模拟核对").click());
    await vi.waitFor(() => expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("无法核对对话状态")));
    expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("失败时保留的消息");
    expect(host.querySelector('[data-action="stop"]')).toBeNull();
    expect(button("重新核对对话状态")).toBeUndefined();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    await vi.waitFor(() => expect(host.querySelector('[data-action="stop"]')).not.toBeNull());
  });
});

it("cancels a waiting send without losing its draft or sending after the check finishes", async () => {
  await withCanvas(async (host, button, scene, draft) => {
    await scene("metadataChecking"); await draft("取消等待的消息");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    await act(async () => { window.dispatchEvent(new KeyboardEvent("keydown", {key:"Escape",cancelable:true})); });
    await act(async () => button("完成模拟核对").click());
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector<HTMLButtonElement>('[data-action="send"]')?.disabled).toBe(false); });
    expect(host.querySelector('[data-action="stop"]')).toBeNull();
    expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("取消等待的消息");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    await vi.waitFor(() => expect(host.querySelector('[data-action="stop"]')).not.toBeNull());
    expect(host.querySelector('[aria-label="输入框准备状态"]')).toBeNull();
  });
});

it("discards a pending send and late failure when switching mock conversations", async () => {
  await withCanvas(async (host, _button, scene, draft) => {
    await scene("metadataFailed"); await draft("旧场景待发消息");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    await scene("metadataChecking");
    expect(host.querySelector('[aria-label="输入框准备状态"]')).toBeNull();
    expect(host.querySelector('[data-action="stop"]')).toBeNull();
    expect(host.textContent).not.toContain("旧场景待发消息");
    expect(host.textContent).not.toContain("无法核对");
  });
});
