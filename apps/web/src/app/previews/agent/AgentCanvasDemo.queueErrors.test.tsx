/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";
import { enqueueCanvasMessage } from "./AgentCanvasDemo.testSupport";
import type { WritingDemoQueueFailure } from "./WritingDiffDemoQueue";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it.each<WritingDemoQueueFailure>(["rejected", "beforeCommit", "afterCommit", "unavailable"])("recovers %s with shared UI and no legacy buttons", async failure => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const button = (text: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === text);
  const rows = () => [...host.querySelectorAll(".agent-input-queue-body li p")].map(item => item.textContent);
  const settle = async (check: () => void) => vi.waitFor(async () => { await act(async () => Promise.resolve()); check(); }, { timeout: 3000 });
  try {
    await act(async () => root.render(<StrictMode><AgentCanvasDemo /></StrictMode>));
    await enqueueCanvasMessage(host,"修改 keywords 部分");
    await enqueueCanvasMessage(host,"按百分比报告进度");
    await settle(() => expect(button("触发队列错误")?.disabled).toBe(false));
    const original = rows(); expect(original).toHaveLength(2);
    await act(async () => {
      const select = host.querySelector<HTMLSelectElement>('[aria-label="模拟队列错误"]')!;
      select.value = failure; select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => button("触发队列错误")?.click());
    expect(button("刷新队列")).toBeUndefined(); expect(button("按原请求重试")).toBeUndefined();
    expect(host.textContent).not.toContain("操作结果未确认"); expect(rows()).toEqual(original);
    if (failure !== "rejected") {
      expect(host.querySelector('.agent-input-queue-status')?.textContent).toBe("正在重新连接…");
      expect(button("触发队列错误")?.disabled).toBe(true);
    }
    if (failure === "unavailable") {
      await settle(() => expect(host.querySelector('[aria-label="重试队列同步"]')).not.toBeNull());
      await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="重试队列同步"]')!.click());
    }
    await settle(() => expect(button("触发队列错误")?.disabled).toBe(false));
    expect(host.querySelector('.agent-input-queue-recovery')).toBeNull(); expect(rows()).toEqual(original);
    await act(async () => button("重置假数据")?.click());
    await settle(() => expect(button("触发队列错误")?.disabled).toBe(false));
    expect(host.querySelector('[aria-label="模拟队列错误"]')).toHaveProperty("value", "rejected");
    expect(rows()).toEqual([]);
    await enqueueCanvasMessage(host,"修改 keywords 部分");
    await enqueueCanvasMessage(host,"按百分比报告进度");
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="移除第 1 条消息"]')?.click());
    await settle(() => expect(rows()).toEqual([original[1]]));
  } finally { await act(async () => root.unmount()); host.remove(); }
});
