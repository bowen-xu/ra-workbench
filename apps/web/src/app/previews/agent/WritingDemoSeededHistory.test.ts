import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoSubmission } from "./WritingDiffDemoSubmission";
import { seedDemoToolHistory } from "./WritingDemoToolHistory";

const cleanups: (() => void)[] = [];
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); vi.useRealTimers(); });

async function fixture() {
  vi.useFakeTimers();
  const manuscriptId=crypto.randomUUID(), conversationId=crypto.randomUUID();
  const storage=new WritingDiffDemoStorage(manuscriptId,[{path:"main.tex",before:"text",after:"text",kind:"modified",additions:0,deletions:0}]);
  const agent=new WritingDiffDemoAgentStorage(storage,undefined);
  const execution=new WritingDiffDemoSubmission(storage,agent,manuscriptId);
  cleanups.push(()=>{execution.dispose();agent.history.dispose();});
  execution.historyOnly=true;execution.delayMs=1000;
  const file=await storage.files.readFile(manuscriptId,"main.tex");
  const preview=await execution.submission.prepareWriting({conversationId,requestId:crypto.randomUUID(),text:"Task",context:{manuscriptId,files:[{relativePath:"main.tex",expectedDiskSha256:file.sha256}]}});
  const scope={manuscriptId,conversationId,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId};
  await execution.workspace.prepare({scope,buffers:[]});
  await execution.submission.startWriting({...scope,model:preview.input.model,mode:"deepSeekNonThinking"});
  const admission={scope,turnId:preview.turnId};
  execution.continueSeededHistory(admission,seedDemoToolHistory(agent.history,admission,true)!);
  const read=async()=>(await agent.history.gateway.readHistory({conversationId,afterSeq:0,limit:500})).events;
  return {execution,agent,preview,scope,read};
}

it("completes the seeded tool, consumes a pending steer, and finishes after the configured wait",async()=>{
  const f=await fixture(),inputId=crypto.randomUUID();
  await f.execution.steer({conversationId:f.scope.conversationId,manuscriptId:f.scope.manuscriptId,turnId:f.preview.turnId,inputId,text:"继续检查"});
  await vi.advanceTimersByTimeAsync(999);
  expect((await f.read()).some(event=>event.payload.type==="itemStarted"&&event.payload.itemId===inputId)).toBe(false);
  await vi.advanceTimersByTimeAsync(1);
  await f.agent.reviews.review(f.scope);
  const events=await f.read(),user=events.findIndex(event=>event.payload.type==="itemStarted"&&event.payload.itemId===inputId);
  const results=events.flatMap((event,index)=>event.payload.type==="itemStarted"&&event.payload.kind.type==="toolResult"?[index]:[]);
  expect(user).toBeGreaterThan(results.at(-1)!);
  expect(events.at(-1)?.payload).toMatchObject({type:"turnFinished",outcome:"completed"});
});

it("reschedules the seeded wait and does not publish further events after interruption",async()=>{
  const f=await fixture();
  await vi.advanceTimersByTimeAsync(500);f.execution.reschedule(2000);
  await vi.advanceTimersByTimeAsync(1000);
  expect((await f.read()).some(event=>event.payload.type==="turnFinished")).toBe(false);
  await f.execution.turns.interrupt({conversationId:f.scope.conversationId,turnId:f.preview.turnId});
  const interrupted=await f.read();await vi.advanceTimersByTimeAsync(2000);
  expect(await f.read()).toEqual(interrupted);
  expect(interrupted.at(-1)?.payload).toMatchObject({type:"turnFinished",outcome:"canceled"});
});
