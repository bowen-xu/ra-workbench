import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoHistory } from "./WritingDiffDemoHistory";
import { applyAgentEvents, createAgentConversationProjection } from "@research-assistant/core/agent";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";
import type {AgentSteeringHandoffIntent} from "@research-assistant/core/agent";

const admission = () => ({ scope: { manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID(),
  requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() });
afterEach(() => vi.useRealTimers());
it("proves a steer from its exact complete history rather than equal text alone",()=>{
  const history=new WritingDiffDemoHistory(),original=admission(),itemId=crypto.randomUUID();
  const intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId:original.scope.conversationId,turnId:original.turnId,
    steeringInputId:itemId,queueInputId:crypto.randomUUID(),expectedRevision:1,expectedEpoch:crypto.randomUUID(),
    scope:{kind:"manuscript",manuscriptId:original.scope.manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,origin:null,text:"Same text",userInput:[]};
  history.start(original,"Same text");
  expect(history.steeringHandoffHistory(intent)).toBe("unconfirmed");
  history.feedbackEvents(original,[{type:"itemStarted",turnId:original.turnId,itemId,kind:{type:"userMessage"}},
    {type:"textDelta",turnId:original.turnId,itemId,text:"Same "}]);
  expect(history.steeringHandoffHistory(intent)).toBe("incompleteOrConflicting");
  history.feedbackEvents(original,[{type:"textDelta",turnId:original.turnId,itemId,text:"text"},{type:"itemCompleted",turnId:original.turnId,itemId}]);
  expect(history.steeringHandoffHistory(intent)).toBe("confirmed");
  expect(history.steeringHandoffHistory({...intent,text:"Different"})).toBe("incompleteOrConflicting");
  expect(history.steeringHandoffHistory({...intent,turnId:crypto.randomUUID()})).toBe("incompleteOrConflicting");
  const reopened=new WritingDiffDemoHistory();reopened.restore(history.snapshot());
  expect(reopened.steeringHandoffHistory(intent)).toBe("confirmed");
  reopened.dispose();history.dispose();
});
it("wakes only the matching history reader and bounds idle waits and reset cleanup", async () => {
  vi.useFakeTimers();
  const history = new WritingDiffDemoHistory(), first = admission(), other = admission();
  const result = vi.fn(), request = { conversationId: first.scope.conversationId, afterSeq: 0, waitMs: 30_000 };
  const waiting = history.gateway.waitForHistory(request).then(result);
  history.start(other, "unrelated"); await Promise.resolve(); expect(result).not.toHaveBeenCalled();
  history.start(first, "matching"); await waiting;
  expect(result).toHaveBeenCalledWith({ schemaVersion: 1, conversationId: request.conversationId, afterSeq: 0, watermark: 5 });
  const timeout = history.gateway.waitForHistory({ ...request, afterSeq: 5, waitMs: 10 });
  await vi.advanceTimersByTimeAsync(10); expect((await timeout).watermark).toBeNull();
  const pending = history.gateway.waitForHistory({ ...request, afterSeq: 5 });
  history.dispose(); expect((await pending).watermark).toBeNull(); expect(vi.getTimerCount()).toBe(0);
  await expect(history.gateway.waitForHistory(request)).rejects.toThrow("已关闭");
});
it("releases a stopped history reader without affecting another reader", async () => {
  vi.useFakeTimers();
  const history = new WritingDiffDemoHistory(), first = admission();
  const request = { conversationId: first.scope.conversationId, afterSeq: 0, waitMs: 30_000 };
  const controller = new AbortController();
  const stopped = history.gateway.waitForHistory(request, controller.signal);
  const rejection = expect(stopped).rejects.toMatchObject({ name: "AbortError" });
  const other = history.gateway.waitForHistory(request);
  controller.abort(); await rejection;
  expect(vi.getTimerCount()).toBe(1);
  history.start(first, "still delivered");
  expect((await other).watermark).toBe(5);
  expect(vi.getTimerCount()).toBe(0);
  history.dispose();
});
it("keeps frozen event IDs and timestamps through pagination, repeat completion and returned snapshot mutation", async () => {
  const history = new WritingDiffDemoHistory(), first = admission();
  history.start(first, "actual request"); history.complete(first, initialChanges);
  const request = { conversationId: first.scope.conversationId, afterSeq: 0, limit: 500 };
  const before = await history.gateway.readHistory(request);
  const expected = structuredClone(before);
  history.complete(first, initialChanges);
  expect(await history.gateway.readHistory(request)).toEqual(expected);
  const sliced = await history.gateway.readHistory({ ...request, limit: 4 });
  expect(sliced.events).toEqual(expected.events.slice(0, 4)); expect(sliced.nextAfterSeq).toBe(4);
  Object.assign(before.events[0]!, { timestampMs: 0 });
  expect(await history.gateway.readHistory(request)).toEqual(expected);
  const projection = applyAgentEvents(createAgentConversationProjection(request.conversationId), expected.events);
  const call = [...projection.turns.get(first.turnId)!.items.values()].find(({ kind }) => kind.type === "toolCall");
  const patch = JSON.parse(call!.text) as { patch: string };
  expect(Object.keys(patch)).toEqual(["patch"]);
  expect(patch.patch).toContain(`*** Update File: source://writings/${first.scope.manuscriptId}/main.tex`);
  expect(patch.patch).toContain("*** End Patch");
  expect(projection.turns.get(first.turnId)?.outcome).toBe("completed");
  expect([...projection.turns.get(first.turnId)!.items.values()].find(({ kind }) => kind.type === "userMessage")?.text).toBe("actual request");
  history.dispose();
});
