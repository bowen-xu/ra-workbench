/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentQueueState } from "@research-assistant/core/agent";
import type { AgentInputQueueState } from "@research-assistant/data/agent";
import { AgentInputQueue } from "./AgentInputQueue";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("keeps background queue reads invisible while preserving and locking existing rows", async () => {
  const empty: AgentQueueState = {
    schemaVersion: 1, conversationId: "conversation", revision: 1, paused: false,
    preparingInputId: null, inputs: [],
  };
  const populated: AgentQueueState = { ...empty, inputs: [{ inputId: "one", text: "待发送" }] };
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const render = (queue: AgentQueueState | null, loading: boolean, uncertain = false) => {
    const state: AgentInputQueueState<AgentQueueState> = { queue, loading, editing: false, uncertain, error: null };
    root.render(<AgentInputQueue state={state} disabled={false} onEdit={vi.fn(async () => true)}
      onReload={() => undefined} onRetry={() => undefined} onSteer={vi.fn(async () => true)} />);
  };
  try {
    await act(async () => render(null, true));
    expect(host.querySelector('[aria-label="队列消息"]')).toBeNull();

    await act(async () => render(empty, false));
    expect(host.querySelector('[aria-label="队列消息"]')).toBeNull();
    await act(async () => render(empty, true));
    expect(host.querySelector('[aria-label="队列消息"]')).toBeNull();
    await act(async () => render(empty, false));
    expect(host.querySelector('[aria-label="队列消息"]')).toBeNull();

    await act(async () => render(populated, true));
    expect(host.textContent).toContain("待发送");
    expect(host.textContent).not.toContain("正在读取队列");
    expect(host.querySelector<HTMLButtonElement>('[aria-label="移除第 1 条消息"]')?.disabled).toBe(true);
    expect(host.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')?.disabled).toBe(true);
    await act(async () => render(populated, false));
    expect(host.querySelector<HTMLButtonElement>('[aria-label="移除第 1 条消息"]')?.disabled).toBe(false);
    expect(host.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')?.disabled).toBe(false);
    await act(async () => render(null, true, true));
    expect(host.textContent).toContain("队列暂时无法同步");
    expect(host.querySelector('[aria-label="重试队列同步"]')).not.toBeNull();
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});
