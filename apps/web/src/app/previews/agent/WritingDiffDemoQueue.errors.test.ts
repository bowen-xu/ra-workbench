import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentInputQueueController, type AgentSubmissionGateway } from "@research-assistant/data/agent";
import { WritingDiffDemoQueue, type WritingDemoQueueFailure } from "./WritingDiffDemoQueue";
const scope = { manuscriptId: "00000000-0000-4000-8000-000000000002", conversationId: "00000000-0000-4000-8000-000000000001" };
const unused = async (): Promise<never> => { throw new Error("Queue faults must not submit to a model"); };
const submission: AgentSubmissionGateway = { prepareWriting: unused, prepareWritingStatus: unused, startStatus: unused, startWriting: unused, restoreWriting: unused };
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());
it.each<WritingDemoQueueFailure>(["beforeCommit", "afterCommit"])("automatically recovers %s with the identical receipt and one addition", async failure => {
  const queue = new WritingDiffDemoQueue(submission), edit = vi.spyOn(queue.gateway, "edit");
  const controller = new AgentInputQueueController(queue.gateway, scope, () => crypto.randomUUID()), acknowledge = vi.fn();
  try {
    await controller.reload(); queue.failNextEdit(failure);
    const pending = controller.edit({ type: "add", inputId: crypto.randomUUID(), text: "模拟追加" }, acknowledge);
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot()).toMatchObject({ editing: true, retrying: true, error: null });
    expect(acknowledge).not.toHaveBeenCalled();
    expect((await queue.gateway.read(scope)).inputs).toHaveLength(failure === "afterCommit" ? 1 : 0);
    await vi.advanceTimersByTimeAsync(300);
    expect(await pending).toBe(true); expect(acknowledge).toHaveBeenCalledOnce();
    expect(edit).toHaveBeenCalledTimes(2); expect(edit.mock.calls[1]).toEqual(edit.mock.calls[0]);
    expect(await queue.gateway.read(scope)).toMatchObject({ revision: 1, inputs: [{ text: "模拟追加" }] });
    expect(controller.getSnapshot()).toMatchObject({ uncertain: false, retrying: false, error: null });
  } finally { controller.dispose(); }
});
it("refreshes a definitive refusal without retries or clearing the draft", async () => {
  const queue = new WritingDiffDemoQueue(submission), edit = vi.spyOn(queue.gateway, "edit"), read = vi.spyOn(queue.gateway, "read");
  const controller = new AgentInputQueueController(queue.gateway, scope, () => crypto.randomUUID()), acknowledge = vi.fn();
  try {
    await controller.reload(); queue.failNextEdit("rejected");
    expect(await controller.edit({ type: "pause" }, acknowledge)).toBe(false);
    expect(controller.getSnapshot()).toMatchObject({ uncertain: false, error: "队列操作未应用，消息已保留。" });
    expect(edit).toHaveBeenCalledOnce(); expect(read).toHaveBeenCalledTimes(2); expect(acknowledge).not.toHaveBeenCalled();
    expect(await controller.retry()).toBe(false);
    expect(await controller.edit({ type: "pause" })).toBe(true);
  } finally { controller.dispose(); }
});
it("bounds connection retries and releases the original-request timer on disposal", async () => {
  const queue = new WritingDiffDemoQueue(submission), edit = vi.spyOn(queue.gateway, "edit");
  const controller = new AgentInputQueueController(queue.gateway, scope, () => crypto.randomUUID());
  try {
    await controller.reload(); queue.failNextEdit("unavailable");
    const pending = controller.edit({ type: "pause" });
    await vi.advanceTimersByTimeAsync(1200);
    expect(await pending).toBe(false); expect(edit).toHaveBeenCalledTimes(3);
    expect(controller.getSnapshot()).toMatchObject({ uncertain: true, retrying: false, editing: false });
    expect(await controller.edit({ type: "resume" })).toBe(false);
    const retry = controller.retry(); await vi.advanceTimersByTimeAsync(0);
    expect(edit.mock.calls[3]).toEqual(edit.mock.calls[0]);
    controller.dispose(); expect(await retry).toBe(false);
    await vi.runAllTimersAsync(); expect(edit).toHaveBeenCalledTimes(4);
  } finally { controller.dispose(); }
});
