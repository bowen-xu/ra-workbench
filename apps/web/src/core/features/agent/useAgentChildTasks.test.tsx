/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentChildTaskSummary } from "@research-assistant/core/agent";
import type { AgentSubtaskGateway } from "@research-assistant/data/agent";

const list = vi.hoisted(() => vi.fn());
vi.mock("../../services/agentGateway", () => ({ agentSubtaskGateway: { list } }));

import { useAgentChildTasks } from "./useAgentChildTasks";

const task = (status: AgentChildTaskSummary["status"]): AgentChildTaskSummary => ({
  schemaVersion: 1,
  childConversationId: "child",
  parentConversationId: "parent",
  parentContextSnapshotId: "parent-context",
  childContextSnapshotId: "child-context",
  depth: 1,
  modelCalls: 1,
  toolCalls: 0,
  createdAt: "2026-01-01T00:00:00.000Z",
  finishedAt: status === "running" ? null : "2026-01-01T00:00:01.000Z",
  status,
});

function Harness({ conversationId, onError, gateway, refreshKey }: { conversationId: string; onError?: (reason: unknown) => void;
  gateway?: Pick<AgentSubtaskGateway, "list" | "subscribe">; refreshKey?: unknown }) {
  const { tasks } = useAgentChildTasks(conversationId, refreshKey, onError, gateway);
  return <output data-testid="status">{tasks.map((item) => item.status).join(",")}</output>;
}

async function renderHarness(container: HTMLElement, conversationId: string) {
  const root = createRoot(container);
  await act(async () => root.render(<Harness conversationId={conversationId} />));
  return root;
}

it("does not poll running children and refreshes on focus", async () => {
  vi.useFakeTimers();
  const container = document.createElement("div");
  const first = Promise.resolve([task("running")]);
  const second = Promise.resolve([task("completed")]);
  list.mockReset().mockReturnValueOnce(first).mockReturnValueOnce(second);
  const root = await renderHarness(container, "parent");
  try {
    await act(async () => first);
    expect(container.textContent).toBe("running");
    expect(list).toHaveBeenCalledTimes(1);
    await act(async () => { vi.advanceTimersByTime(1000); });
    expect(list).toHaveBeenCalledTimes(1);
    await act(async () => { window.dispatchEvent(new Event("focus")); await second; });
    expect(container.textContent).toBe("completed");
    expect(list).toHaveBeenCalledTimes(2);
    await act(async () => { vi.advanceTimersByTime(3000); });
    expect(list).toHaveBeenCalledTimes(2);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
  }
});

it("subscribes before baseline, bridges delayed registration once, and releases the listener", async () => {
  const order: string[] = [], stop = vi.fn();
  let changed = () => {}, registered!: (stop: () => void) => void;
  let status: AgentChildTaskSummary["status"] = "running";
  const read = vi.fn(async () => { order.push("read"); return [task(status)]; });
  const gateway = {list: read, subscribe: vi.fn((_id: string, notify: () => void) => {
    order.push("subscribe"); changed = notify;
    return new Promise<() => void>(resolve => { registered = resolve; });
  })};
  const container = document.createElement("div"), root = createRoot(container);
  try {
    await act(async () => root.render(<Harness conversationId="parent" gateway={gateway} refreshKey="turn" />));
    expect(order).toEqual(["subscribe", "read"]);
    // Equivalent text-sequence renders retain the same lifecycle key.
    await act(async () => root.render(<Harness conversationId="parent" gateway={gateway} refreshKey="turn" />));
    expect(read).toHaveBeenCalledTimes(1);
    await act(async () => { registered(stop); });
    expect(read).toHaveBeenCalledTimes(2);
    status = "completed";
    await act(async () => changed());
    expect(container.textContent).toBe("completed");
    expect(read).toHaveBeenCalledTimes(3);
    await act(async () => root.render(<Harness conversationId="parent" gateway={gateway} refreshKey="idle" />));
    expect(read).toHaveBeenCalledTimes(4);
  } finally { await act(async () => root.unmount()); }
  expect(stop).toHaveBeenCalledTimes(1);
  await act(async () => { changed(); window.dispatchEvent(new Event("focus")); });
  expect(read).toHaveBeenCalledTimes(4);
});

it("releases a subscription whose registration finishes after unmount", async () => {
  let registered!: (stop: () => void) => void;
  const stop = vi.fn(), read = vi.fn(async () => []);
  const gateway = {list: read, subscribe: () => new Promise<() => void>(resolve => { registered = resolve; })};
  const root = createRoot(document.createElement("div"));
  await act(async () => root.render(<Harness conversationId="parent" gateway={gateway} />));
  await act(async () => root.unmount());
  await act(async () => { registered(stop); });
  expect(stop).toHaveBeenCalledTimes(1);
  expect(read).toHaveBeenCalledTimes(1);
});

it("drops a late response from a previous conversation", async () => {
  let resolveA!: (value: AgentChildTaskSummary[]) => void;
  let resolveB!: (value: AgentChildTaskSummary[]) => void;
  const responseA = new Promise<AgentChildTaskSummary[]>((resolve) => { resolveA = resolve; });
  const responseB = new Promise<AgentChildTaskSummary[]>((resolve) => { resolveB = resolve; });
  vi.useRealTimers();
  list.mockReset().mockReturnValueOnce(responseA).mockReturnValueOnce(responseB);
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Harness conversationId="a" />));
    await act(async () => root.render(<Harness conversationId="b" />));
    await act(async () => { resolveA([task("failed")]); await responseA; });
    expect(container.textContent).toBe("");
    await act(async () => { resolveB([task("completed")]); await responseB; });
    expect(container.textContent).toBe("completed");
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

it("drops a late failure from a previous conversation without clearing current tasks or reporting it", async () => {
  let rejectA!: (reason: unknown) => void;
  let resolveB!: (value: AgentChildTaskSummary[]) => void;
  const responseA = new Promise<AgentChildTaskSummary[]>((_, reject) => { rejectA = reject; });
  const responseB = new Promise<AgentChildTaskSummary[]>((resolve) => { resolveB = resolve; });
  const onError = vi.fn();
  list.mockReset().mockReturnValueOnce(responseA).mockReturnValueOnce(responseB);
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<Harness conversationId="a" onError={onError} />));
    await act(async () => root.render(<Harness conversationId="b" onError={onError} />));
    await act(async () => { resolveB([task("completed")]); await responseB; });
    expect(container.textContent).toBe("completed");
    await act(async () => { rejectA(new Error("stale")); await responseA.catch(() => undefined); });
    expect(container.textContent).toBe("completed");
    expect(onError).not.toHaveBeenCalled();
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

it("ignores a late response after the host unmounts", async () => {
  let resolve!: (value: AgentChildTaskSummary[]) => void;
  const response = new Promise<AgentChildTaskSummary[]>((next) => { resolve = next; });
  const onError = vi.fn();
  list.mockReset().mockReturnValueOnce(response);
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<Harness conversationId="parent" onError={onError} />));
  await act(async () => root.unmount());
  await act(async () => { resolve([task("completed")]); await response; });
  expect(onError).not.toHaveBeenCalled();
  container.remove();
});
