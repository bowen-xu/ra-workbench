import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoSubmission } from "./WritingDiffDemoSubmission";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";

const manuscriptId = "00000000-0000-4000-8000-000000000003";
const conversationId = "00000000-0000-4000-8000-000000000001";
const live: { execution: WritingDiffDemoSubmission; agent: WritingDiffDemoAgentStorage }[] = [];
afterEach(() => { for (const f of live.splice(0)) { f.execution.dispose(); f.agent.history.dispose(); } vi.useRealTimers(); });
async function fixture(multiple: boolean) {
  const storage = new WritingDiffDemoStorage(manuscriptId, initialChanges);
  const agent = new WritingDiffDemoAgentStorage(storage, undefined);
  const execution = new WritingDiffDemoSubmission(storage, agent, manuscriptId);
  live.push({ execution, agent });
  execution.progressiveFeedback = true; execution.multipleResponses = multiple;
  execution.delayMs = multiple ? 12000 : 900;
  const files = await storage.files.listFiles(manuscriptId);
  const buffers = await Promise.all(files.map(async f => ({ path: f.relativePath, expectedDiskSha256: f.sha256,
    sessionId: crypto.randomUUID(), revision: crypto.randomUUID(), text: (await storage.files.readFile(manuscriptId, f.relativePath)).content })));
  const preview = await execution.submission.prepareWriting({ conversationId, requestId: crypto.randomUUID(), text: "修改稿件",
    context: { manuscriptId, files: files.map(f => ({ relativePath: f.relativePath, expectedDiskSha256: f.sha256 })) } });
  const scope = { manuscriptId, conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
  await execution.workspace.prepare({ scope, buffers });
  vi.useFakeTimers();
  await execution.submission.startWriting({ ...scope, model: preview.input.model, mode: "deepSeekNonThinking" });
  const read = async () => (await agent.history.gateway.readHistory({ conversationId, afterSeq: 0, limit: 500 })).events;
  return { execution, agent, preview, read, scope };
}
it("waits for model thinking after the tool result then streams three characters every 300 ms", async () => {
  const f = await fixture(false);
  await vi.advanceTimersByTimeAsync(300);
  const pending = await f.read();
  const call = pending.find(e => e.payload.type === "itemStarted" && e.payload.kind.type === "toolCall")!;
  expect(pending.some(e => e.payload.type === "itemCompleted" && e.payload.itemId === (call.payload as { itemId: string }).itemId)).toBe(false);
  await vi.advanceTimersByTimeAsync(300);
  expect((await f.read()).some(e => e.payload.type === "itemStarted" && e.payload.kind.type === "toolResult")).toBe(true);
  await vi.advanceTimersByTimeAsync(299);
  expect((await f.read()).some(e => e.payload.type === "itemStarted" && e.payload.kind.type === "assistantMessage")).toBe(false);
  let firstDeltaTime: number | undefined;
  let text = "";
  const expected = "已准备 3 个文件的修改。";
  for (let offset=0;offset<Array.from(expected).length;offset+=3) {
    if (offset > 0) await vi.advanceTimersByTimeAsync(299);
    const before=await f.read();
    await vi.advanceTimersByTimeAsync(1);
    const after=await f.read();
    const delta=after.slice(before.length).find(e=>e.payload.type==="textDelta");
    expect(delta?.payload).toMatchObject({type:"textDelta",text:Array.from(expected).slice(offset,offset+3).join("")});
    if(firstDeltaTime!==undefined)expect(delta!.timestampMs-firstDeltaTime).toBe(300);
    firstDeltaTime=delta!.timestampMs;
    text+=(delta!.payload as {text:string}).text;
  }
  expect(text).toBe(expected);
  await vi.advanceTimersByTimeAsync(1);
  await f.agent.reviews.review(f.scope);
  const finished=await f.read();
  expect(finished.at(-1)?.payload).toMatchObject({type:"turnFinished",outcome:"completed"});
  expect(finished.filter(e=>e.payload.type==="itemStarted"&&e.payload.kind.type==="toolCall")).toHaveLength(1);
});
it("spaces every intermediate tool and text event and allows Steer between returns", async () => {
  const f=await fixture(true);
  let steered=false;
  for(let step=0;step<200;step++) {
    await vi.advanceTimersByTimeAsync(300);
    const events=await f.read();
    const assistant=events.filter(e=>e.payload.type==="itemStarted"&&e.payload.kind.type==="assistantMessage");
    if(!steered&&assistant.length) { await f.execution.steer({manuscriptId,conversationId,turnId:f.preview.turnId,inputId:crypto.randomUUID(),text:"继续检查"});steered=true; }
    if(events.some(e=>e.payload.type==="turnFinished"))break;
  }
  await f.agent.reviews.review(f.scope);
  const events=await f.read();
  const messageIds=events.flatMap(e=>e.payload.type==="itemStarted"&&e.payload.kind.type==="assistantMessage"?[e.payload.itemId]:[]);
  expect(messageIds).toHaveLength(4);
  const results = events.filter(e => e.payload.type === "itemStarted" && e.payload.kind.type === "toolResult");
  const messages = events.filter(e => e.payload.type === "itemStarted" && e.payload.kind.type === "assistantMessage");
  expect(results).toHaveLength(4);
  for (let index = 0; index < results.length; index++) {
    expect(messages[index]!.timestampMs - results[index]!.timestampMs).toBe(1000);
  }
  for(const id of messageIds) {
    const chunks=events.filter(e=>e.payload.type==="textDelta"&&e.payload.itemId===id);
    expect(chunks.length).toBeGreaterThan(2);
    for(let index=0;index<chunks.length;index++) {
      expect(Array.from((chunks[index]!.payload as {text:string}).text).length).toBeLessThanOrEqual(3);
      if(index)expect(chunks[index]!.timestampMs-chunks[index-1]!.timestampMs).toBe(300);
    }
  }
  expect(events.at(-1)?.payload).toMatchObject({type:"turnFinished",outcome:"completed"});
});
it("keeps steer pending through streaming until the response boundary", async () => {
  const f=await fixture(false), inputId=crypto.randomUUID();
  f.execution.setTextPacing({charactersPerChunk:1,intervalMs:1000});
  await vi.advanceTimersByTimeAsync(900);
  await f.execution.steer({manuscriptId,conversationId,turnId:f.preview.turnId,inputId,text:"补充说明"});
  await vi.advanceTimersByTimeAsync(3000);
  expect((await f.read()).some(event=>event.payload.type==="itemStarted"&&event.payload.itemId===inputId)).toBe(false);
  await vi.advanceTimersByTimeAsync(20000);
  await f.agent.reviews.review(f.scope);
  const events=await f.read(), userIndex=events.findIndex(event=>event.payload.type==="itemStarted"&&event.payload.itemId===inputId);
  const response=events.find(event=>event.payload.type==="itemStarted"&&event.payload.kind.type==="assistantMessage");
  const responseId=response?.payload.type==="itemStarted"?response.payload.itemId:undefined;
  const doneIndex=events.findIndex(event=>event.payload.type==="itemCompleted"&&event.payload.itemId===responseId);
  expect(doneIndex).toBeGreaterThan(-1);expect(userIndex).toBeGreaterThan(doneIndex);
});
it("interrupts a pending tool without publishing later results or messages", async () => {
  const f = await fixture(false);
  await vi.advanceTimersByTimeAsync(300);
  await f.execution.turns.interrupt({ conversationId, turnId: f.preview.turnId });
  const canceled = await f.read();
  await vi.advanceTimersByTimeAsync(2000);
  expect(await f.read()).toEqual(canceled);
  expect(canceled.at(-1)?.payload).toMatchObject({ type: "turnFinished", outcome: "canceled" });
});
it("keeps thinking after a result when pacing changes and cancels before text starts", async () => {
  const f = await fixture(false);
  await vi.advanceTimersByTimeAsync(600);
  expect((await f.read()).some(e => e.payload.type === "itemStarted" && e.payload.kind.type === "toolResult")).toBe(true);
  f.execution.setTextPacing({ charactersPerChunk: 8, intervalMs: 10 });
  await vi.advanceTimersByTimeAsync(299);
  expect((await f.read()).some(e => e.payload.type === "itemStarted" && e.payload.kind.type === "assistantMessage")).toBe(false);
  await f.execution.turns.interrupt({ conversationId, turnId: f.preview.turnId });
  const canceled = await f.read();
  await vi.advanceTimersByTimeAsync(2000);
  expect(await f.read()).toEqual(canceled);
  expect(canceled.at(-1)?.payload).toMatchObject({ type: "turnFinished", outcome: "canceled" });
});
it("changes text pacing after tools without rescheduling tool waits or losing the emitted prefix", async () => {
  const f = await fixture(false);
  await vi.advanceTimersByTimeAsync(300);
  f.execution.setTextPacing({ charactersPerChunk: 2, intervalMs: 100 });
  await vi.advanceTimersByTimeAsync(299);
  expect((await f.read()).some(e => e.payload.type === "itemStarted" && e.payload.kind.type === "toolResult")).toBe(false);
  await vi.advanceTimersByTimeAsync(1);
  await vi.advanceTimersByTimeAsync(299);
  expect((await f.read()).some(e => e.payload.type === "itemStarted" && e.payload.kind.type === "assistantMessage")).toBe(false);
  await vi.advanceTimersByTimeAsync(1);
  const events = await f.read();
  const assistant = events.find(e => e.payload.type === "itemStarted" && e.payload.kind.type === "assistantMessage");
  const id = assistant?.payload.type === "itemStarted" ? assistant.payload.itemId : undefined;
  expect(id).toBeDefined();
  expect(events.filter(e => e.payload.type === "textDelta" && e.payload.itemId === id).at(-1)?.payload).toMatchObject({ text: "已准" });
  await vi.advanceTimersByTimeAsync(50);
  f.execution.setTextPacing({ charactersPerChunk: 1, intervalMs: 200 });
  await vi.advanceTimersByTimeAsync(199);
  expect((await f.read()).filter(e => e.payload.type === "textDelta" && e.payload.itemId === id)).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(3000);
  await f.agent.reviews.review(f.scope);
  const final = await f.read();
  const chunks = final.filter(e => e.payload.type === "textDelta" && e.payload.itemId === id);
  expect(chunks.map(e => e.payload.type === "textDelta" ? e.payload.text : "").join("")).toBe("已准备 3 个文件的修改。");
  expect(final.at(-1)?.payload).toMatchObject({ type: "turnFinished", outcome: "completed" });
});
