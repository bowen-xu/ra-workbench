/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it.each(["recover", "exhaust"])("previews partial output and %s using the production conversation panel", async mode => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const settle = async (check: () => void) => vi.waitFor(async () => { await act(async () => Promise.resolve()); check(); }, {timeout:4000});
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await settle(() => expect(host.querySelector('[data-action="stop"]')).not.toBeNull());
    vi.useFakeTimers();
    await act(async () => {
      const scene = host.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
      scene.value = "single"; scene.dispatchEvent(new Event("change",{bubbles:true}));
    });
    await settle(() => expect(host.querySelector('[data-agent-activity="thinking"]')).not.toBeNull());
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    await settle(() => expect(host.querySelectorAll('.writing-agent-message.assistant')).toHaveLength(1));
    await act(async () => {
      const recovery = host.querySelector<HTMLSelectElement>('[aria-label="模拟中断恢复"]')!;
      recovery.value = mode; recovery.dispatchEvent(new Event("change",{bubbles:true}));
    });
    await act(async () => [...host.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === "模拟网络中断")!.click());
    await settle(() => expect(host.textContent).toContain("正在重新连接"));
    expect(host.querySelector('[data-agent-activity="reconnecting"]')?.textContent).toContain("1/5");
    const body = host.querySelector('.writing-agent-message.assistant')!;
    const reconnect = host.querySelector('[data-agent-activity="reconnecting"]')!;
    expect(body.compareDocumentPosition(reconnect) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(host.textContent).not.toContain("Agent 请求失败");
    expect(host.textContent).not.toContain("（未完成）");
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    for (let step=0;step<50 && host.querySelector('[data-action="stop"]');step++) {
      await act(async () => vi.advanceTimersByTimeAsync(300));
    }
    await settle(() => expect(host.querySelector('[data-action="stop"]')).toBeNull());
    expect(host.textContent).not.toContain("正在重新连接");
    expect(host.querySelector('[data-agent-activity="reconnect-history"]')?.textContent).toContain("模拟故障");
    if (mode === "recover") {
      expect(host.querySelectorAll('.writing-agent-message.assistant')).toHaveLength(1);
      expect(host.textContent).toContain("不调用工具，也不修改稿件。");
      expect(host.textContent).not.toContain("（未完成）");
      expect(host.textContent).not.toContain("Agent 请求失败");
    } else {
      expect(host.textContent).toContain("（未完成）");
      expect(host.textContent).toContain("模拟故障：与模型服务的网络连接中断。");
      expect(host.querySelector('.writing-agent-turn-status')?.textContent).toContain("未完成");
    }
  } finally { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); }
});
