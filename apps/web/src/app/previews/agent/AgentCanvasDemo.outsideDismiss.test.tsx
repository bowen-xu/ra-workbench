/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("keeps the Canvas preview open on outside clicks while retaining explicit hide", async () => {
  const host = document.createElement("div"), outside = document.createElement("button");
  outside.textContent = "预览外部控件";
  document.body.append(host, outside);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await vi.waitFor(async () => {
      await act(async () => Promise.resolve());
      expect(host.querySelector('[data-action="stop"]')).not.toBeNull();
    }, { timeout: 4000 });
    const panel = host.querySelector('.writing-agent-panel');
    expect(panel).not.toBeNull();
    await act(async () => {
      outside.focus();
      outside.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      outside.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
      outside.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
      outside.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      outside.click();
    });
    expect(host.querySelector('.writing-agent-panel')).toBe(panel);
    expect(host.querySelector('[data-action="stop"]')).not.toBeNull();
    expect(document.activeElement).toBe(outside);
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="隐藏 AI 写作助手"]')?.click());
    expect(host.querySelector('.writing-agent-panel')).toBe(panel);
    expect((panel as HTMLElement).hidden).toBe(true);
    expect(host.querySelector('[aria-label="打开对话"]')).not.toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove(); outside.remove();
  }
});
