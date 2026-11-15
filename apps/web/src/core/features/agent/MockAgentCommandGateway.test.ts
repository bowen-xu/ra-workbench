import { afterEach, expect, it, vi } from "vitest";
import { MockAgentCommandGateway } from "./MockAgentCommandGateway";

afterEach(() => vi.useRealTimers());
it("captures the configured delay for each new compaction", async () => {
  vi.useFakeTimers();
  const gateway = new MockAgentCommandGateway(3000), signal = new AbortController().signal;
  const finished = vi.fn();
  const first = gateway.compact({conversationId: "first", signal}).then(finished);
  await vi.advanceTimersByTimeAsync(100);
  gateway.delayMs = 50;
  await vi.advanceTimersByTimeAsync(2899);
  expect(finished).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1); await first;
  expect(finished).toHaveBeenCalledOnce();
  const next = gateway.compact({conversationId: "second", signal}).then(finished);
  await vi.advanceTimersByTimeAsync(49); expect(finished).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(1); await next;
  expect(finished).toHaveBeenCalledTimes(2); gateway.dispose();
});
it("compacts synthetic usage for one conversation without changing another", async () => {
  vi.useFakeTimers();
  const gateway = new MockAgentCommandGateway();
  const request = { conversationId: "first", signal: new AbortController().signal };
  expect(await gateway.readUsage(request)).toEqual({ inputTokens: 82000, contextWindowTokens: 128000 });
  const pending = gateway.compact(request);
  await vi.advanceTimersByTimeAsync(1000);
  expect(await pending).toEqual({ beforeTokens: 82000, usage: { inputTokens: 18000, contextWindowTokens: 128000 } });
  expect((await gateway.readUsage({ ...request, conversationId: "second" })).inputTokens).toBe(82000);
  gateway.dispose();
});
it.each(["abort", "dispose"])("cancels pending mock compaction on %s", async action => {
  vi.useFakeTimers();
  const gateway = new MockAgentCommandGateway(), controller = new AbortController();
  const request = { conversationId: "first", signal: controller.signal };
  const pending = gateway.compact(request);
  const rejected = expect(pending).rejects.toMatchObject({ name: "AbortError" });
  await vi.advanceTimersByTimeAsync(100);
  if (action === "abort") controller.abort(); else gateway.dispose();
  await rejected;
  expect(vi.getTimerCount()).toBe(0);
  if (action === "abort") {
    expect((await gateway.readUsage({ ...request, signal: new AbortController().signal })).inputTokens).toBe(82000);
    gateway.dispose();
  }
});

it("replays the original command receipt by conversation and queue identity", async () => {
  vi.useFakeTimers();
  const gateway = new MockAgentCommandGateway(), signal = new AbortController().signal;
  const request = {conversationId: "first", requestId: "queue-item", signal};
  const pending = gateway.compact(request);
  await vi.advanceTimersByTimeAsync(1000);
  const result = await pending;
  expect(await gateway.compact(request)).toEqual(result);
  expect(vi.getTimerCount()).toBe(0);
  const other = gateway.compact({...request, conversationId: "second"});
  await vi.advanceTimersByTimeAsync(1000);
  expect((await other).beforeTokens).toBe(82000);
  gateway.dispose();
});
