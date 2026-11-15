/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("previews one final text through the production panel with no work disclosure", async () => {
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await vi.waitFor(async () => { await act(async () => new Promise(resolve => setTimeout(resolve, 10)));
      expect(container.querySelector('[data-action="stop"]')).not.toBeNull(); }, { timeout: 4000 });
    vi.useFakeTimers();
    await act(async () => {
      const scenario = container.querySelector<HTMLSelectElement>('[aria-label="模拟消息场景"]')!;
      scenario.value = "single"; scenario.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await vi.waitFor(async () => { await act(async () => new Promise(resolve => setTimeout(resolve, 10)));
      expect(container.querySelector('[data-agent-activity="thinking"]')).not.toBeNull();
      expect(container.querySelector('[aria-label="模拟消息场景"]')).toHaveProperty("value", "single"); }, { timeout: 4000 });
    await act(async () => vi.advanceTimersByTimeAsync(3000));
    expect(container.querySelector('.writing-agent-turn-process')).toBeNull();
    await vi.waitFor(async () => { await act(async () => Promise.resolve());
      expect(container.querySelectorAll('.writing-agent-message.assistant')).toHaveLength(1); }, { timeout: 4000 });
    for (let step = 0; step < 40 && container.querySelector('[data-action="stop"]'); step++) {
      await act(async () => vi.advanceTimersByTimeAsync(300));
      expect(container.querySelector('.writing-agent-turn-process')).toBeNull();
    }
    await vi.waitFor(async () => { await act(async () => Promise.resolve());
      expect(container.querySelector('[data-action="stop"]')).toBeNull(); }, { timeout: 4000 });
    expect(container.querySelector('.writing-agent-message.assistant')?.textContent).toContain("不调用工具，也不修改稿件。");
    expect(container.querySelector('.writing-agent-activity-group')).toBeNull();
    expect(container.querySelector('.writing-agent-live-activity')).toBeNull();
  } finally { await act(async () => root.unmount()); container.remove(); vi.useRealTimers(); }
});
