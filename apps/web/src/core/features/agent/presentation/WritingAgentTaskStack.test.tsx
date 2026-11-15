/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { WritingAgentTaskStack } from "./WritingAgentTaskStack";

it("uses the shared queue and goal shell for mock and production data", async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  const root = createRoot(container);
  const onEdit = vi.fn(async () => true);
  const onDelete = vi.fn();
  const onToggle = vi.fn();
  const onGoalEdit = vi.fn();
  const state = {
    queue: {
      schemaVersion: 1 as const,
      conversationId: "conversation",
      manuscriptId: "manuscript",
      revision: 1,
      paused: false,
      preparingInputId: null,
      inputs: [{ inputId: "one", text: "修改当前稿件" }],
    },
    editing: false,
    loading: false,
    uncertain: false,
    error: null,
  };
  try {
    await act(async () => root.render(<WritingAgentTaskStack
      queue={{ state, disabled: false, onEdit, onReload: vi.fn(), onRetry: vi.fn() }}
      goal={{ text: "修改当前稿件 · sn-article.tex", state: "running", onDelete, onToggle, onEdit: onGoalEdit }} />));
    expect(container.querySelector(".writing-agent-task-stack.has-goal")).not.toBeNull();
    expect(container.querySelector(".writing-agent-input-queue")).not.toBeNull();
    expect(container.querySelector(".writing-agent-goal")).not.toBeNull();
    expect(container.querySelector(".writing-agent-goal strong")?.textContent).toBe("追求目标");
    expect(container.textContent).toContain("修改当前稿件 · sn-article.tex");
    for (const label of ["删除目标", "暂停目标", "编辑目标"]) {
      const action = container.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
      expect(action?.parentElement?.className).toBe("writing-agent-goal-actions");
      expect(action?.hasAttribute("title")).toBe(false);
    }
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="暂停目标"]')?.click());
    expect(onToggle).toHaveBeenCalledOnce();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="删除目标"]')?.click());
    expect(onDelete).toHaveBeenCalledOnce();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="编辑目标"]')?.click());
    const input = document.querySelector<HTMLInputElement>('[aria-label="目标内容"]');
    expect(input).not.toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, "更新后的目标");
      input!.dispatchEvent(new Event("input", { bubbles: true }));
      document.querySelector<HTMLButtonElement>(".writing-agent-goal-modal-actions button:last-child")?.click();
    });
    expect(onGoalEdit).toHaveBeenCalledWith("更新后的目标");
    await act(async () => root.render(<WritingAgentTaskStack goal={{ text: "更新后的目标", state: "paused", onToggle }} />));
    expect(container.querySelector(".writing-agent-goal strong")?.textContent).toBe("暂停目标");
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="开始目标"]')?.click());
    expect(onToggle).toHaveBeenCalledTimes(2);
    await act(async () => root.render(<WritingAgentTaskStack goal={{ text: "更新后的目标", state: "idle" }} />));
    expect(container.querySelector(".writing-agent-goal strong")?.textContent).toBe("追求目标");
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
