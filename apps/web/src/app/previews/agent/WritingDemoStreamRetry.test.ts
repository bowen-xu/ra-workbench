import { afterEach, expect, it, vi } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, agentTurnReconnecting } from "@research-assistant/core/agent";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoSubmission } from "./WritingDiffDemoSubmission";
import { projectConversationMessages } from "../../../core/features/agent/projectConversationMessages";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";

const manuscriptId = "00000000-0000-4000-8000-000000000003";
const conversationId = "00000000-0000-4000-8000-000000000001";
const text = "这是重新生成的完整解释😀，已完成的工具不会再次执行。";
const live: { execution: WritingDiffDemoSubmission; agent: WritingDiffDemoAgentStorage }[] = [];
afterEach(() => { for (const f of live.splice(0)) { f.execution.dispose(); f.agent.history.dispose(); } vi.useRealTimers(); });
async function fixture(single = true) {
  const storage = new WritingDiffDemoStorage(manuscriptId, initialChanges);
  const agent = new WritingDiffDemoAgentStorage(storage, undefined);
  const execution = new WritingDiffDemoSubmission(storage, agent, manuscriptId);
  live.push({ execution, agent });
  execution.progressiveFeedback = true;
  if (single) execution.singleTextResponse = text;
  execution.delayMs = single ? 900 : 2700;
  const files = await storage.files.listFiles(manuscriptId);
  const preview = await execution.submission.prepareWriting({ conversationId, requestId: crypto.randomUUID(), text: "解释选段",
    context: { manuscriptId, files: files.map(f => ({ relativePath: f.relativePath, expectedDiskSha256: f.sha256 })) } });
  const scope = { manuscriptId, conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
  await execution.workspace.prepare({scope, buffers:[]});
  vi.useFakeTimers();
  await execution.submission.startWriting({...scope,model:preview.input.model,mode:"deepSeekNonThinking"});
  const read = async () => (await agent.history.gateway.readHistory({conversationId,afterSeq:0,limit:500})).events;
  const projection = async () => applyAgentEvents(createAgentConversationProjection(conversationId), await read());
  return { execution, agent, preview, read, projection, scope };
}

it("reconnects after visible text, replaces the failed draft and preserves the original user input", async () => {
  const f = await fixture();
  await vi.advanceTimersByTimeAsync(900);
  expect(projectConversationMessages(await f.projection()).some(m => m.content === text.slice(0,3))).toBe(true);
  f.execution.simulateReconnect(conversationId);
  f.execution.simulateReconnect(conversationId); // A double click cannot spawn another retry.
  const pending = await f.projection();
  expect(agentTurnReconnecting(pending.turns.get(f.preview.turnId)!)).toBe(true);
  expect(projectConversationMessages(pending).some(m => m.failure)).toBe(false);
  f.execution.reschedule(100); f.execution.setTextPacing({charactersPerChunk:4,intervalMs:100});
  await vi.advanceTimersByTimeAsync(2999);
  expect(await f.read()).toHaveLength(pending.seq);
  await vi.advanceTimersByTimeAsync(1);
  await vi.advanceTimersByTimeAsync(10000);
  await f.agent.reviews.review(f.scope);
  const done = await f.projection();
  expect(done.turns.get(f.preview.turnId)?.outcome).toBe("completed");
  expect(projectConversationMessages(done).filter(m => m.role === "assistant").map(m => m.content)).toEqual([text]);
  const events = await f.read();
  expect(events.filter(e => e.payload.type === "itemStarted" && e.payload.kind.type === "userMessage")).toHaveLength(1);
  expect(events.filter(e => e.payload.type === "modelRequestFailed")).toHaveLength(1);
  const draft = [...done.turns.get(f.preview.turnId)!.items.values()].find(item => item.interrupted);
  expect(draft).toMatchObject({text:text.slice(0,3),completed:false});
});

it("preserves a completed tool result when regenerating its unfinished explanation", async () => {
  const f = await fixture(false);
  await vi.advanceTimersByTimeAsync(2700); // call, result, first text chunk
  const prior = await f.read();
  expect(prior.some(e => e.payload.type === "itemStarted" && e.payload.kind.type === "assistantMessage")).toBe(true);
  f.execution.simulateReconnect(conversationId);
  await vi.advanceTimersByTimeAsync(10000);
  await f.agent.reviews.review(f.scope);
  const done = await f.projection();
  expect(done.turns.get(f.preview.turnId)?.outcome).toBe("completed");
  expect([...done.turns.get(f.preview.turnId)!.items.values()].filter(item => item.kind.type === "toolCall")).toHaveLength(1);
  expect([...done.turns.get(f.preview.turnId)!.items.values()].filter(item => item.kind.type === "toolResult")).toHaveLength(1);
  expect(projectConversationMessages(done).filter(m => !m.kind && m.role === "assistant").map(m => m.content)).toEqual(["已准备 0 个文件的修改。"]);
});

it("shows the last partial answer and one error when retries are exhausted", async () => {
  const f = await fixture(); await vi.advanceTimersByTimeAsync(900);
  f.execution.simulateReconnect(conversationId, true);
  for (let ordinal = 1; ordinal <= 5; ordinal++) {
    expect(projectConversationMessages(await f.projection()).some(message => message.reconnectAttempt === ordinal)).toBe(true);
    await vi.advanceTimersByTimeAsync(3000);
  }
  await f.agent.reviews.review(f.scope);
  const done = await f.projection(), messages = projectConversationMessages(done);
  expect(done.turns.get(f.preview.turnId)?.outcome).toBe("failed");
  expect(messages.filter(m => m.failure)).toHaveLength(1);
  expect(messages.find(m => m.role === "assistant" && !m.failure)?.content).toBe(text.slice(0,3) + "\n\n（未完成）");
  expect((await f.read()).filter(e => e.payload.type === "modelRequestFailed")).toHaveLength(6);
});

it("cancellation during reconnect prevents new attempts and text", async () => {
  const f = await fixture(); await vi.advanceTimersByTimeAsync(900);
  f.execution.simulateReconnect(conversationId);
  await f.execution.turns.interrupt({conversationId,turnId:f.preview.turnId});
  const canceled = await f.read(); await vi.advanceTimersByTimeAsync(10000);
  expect(await f.read()).toEqual(canceled);
  expect((await f.projection()).turns.get(f.preview.turnId)?.outcome).toBe("canceled");
});
it("restores a long prefix immediately and resets reconnects as soon as new output arrives", async () => {
  const f = await fixture();
  f.execution.setTextPacing({charactersPerChunk:3,intervalMs:2000});
  await vi.advanceTimersByTimeAsync(6900);
  const before = projectConversationMessages(await f.projection()).find(m=>m.role === "assistant")!.content;
  expect(before.length).toBeGreaterThan(3);
  f.execution.simulateReconnect(conversationId);
  await vi.advanceTimersByTimeAsync(3000);
  await vi.advanceTimersByTimeAsync(2000);
  const recovered = projectConversationMessages(await f.projection()).find(m=>m.role === "assistant")!;
  expect(recovered.content.startsWith(before)).toBe(true);
  expect(recovered.content.length).toBe(before.length + 3);
  f.execution.simulateReconnect(conversationId);
  expect(projectConversationMessages(await f.projection()).some(m=>m.reconnectAttempt === 1)).toBe(true);
  await vi.advanceTimersByTimeAsync(3000);
  await vi.advanceTimersByTimeAsync(2000);
  f.execution.simulateReconnect(conversationId, true);
  for(let ordinal=1;ordinal<=5;ordinal++) {
    expect(projectConversationMessages(await f.projection()).some(m=>m.reconnectAttempt === ordinal)).toBe(true);
    await vi.advanceTimersByTimeAsync(3000);
  }
  await f.agent.reviews.review(f.scope);
  expect((await f.projection()).turns.get(f.preview.turnId)?.outcome).toBe("failed");
});
