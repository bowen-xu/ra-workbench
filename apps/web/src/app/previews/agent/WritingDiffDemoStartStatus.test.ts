import {afterEach,expect,it,vi} from "vitest";
import {agentWritingCompactionStartRequest,executionModeForModel,applyAgentEvents,createAgentConversationProjection,decodeAgentEvent,
  type AgentSubmissionPreview,type AgentWritingPrepareRequest,type AgentWritingStartRequest} from "@research-assistant/core/agent";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";
type Side="ordinary"|"manual";
const cleanups:(()=>void)[]=[];
afterEach(()=>{cleanups.splice(0).forEach(run=>run());vi.useRealTimers();vi.restoreAllMocks();});
async function fixture(side:Side){
  const scope={manuscriptId:crypto.randomUUID(),conversationId:crypto.randomUUID()};
  const storage=new WritingDiffDemoStorage(scope.manuscriptId,[{path:"main.tex",before:"original",after:"original",kind:"modified",additions:0,deletions:0}]);
  const agent=new WritingDiffDemoAgentStorage(storage,undefined),runtime=createWritingDiffDemoRuntime(storage,agent,scope.manuscriptId,scope.conversationId);
  runtime.execution.historyOnly=true;cleanups.push(()=>runtime.dispose());
  const disk=await storage.files.readFile(scope.manuscriptId,"main.tex");
  const request:AgentWritingPrepareRequest={conversationId:scope.conversationId,requestId:crypto.randomUUID(),text:side==="manual"?"/compact":"Original ordinary",
    context:{manuscriptId:scope.manuscriptId,files:[{relativePath:"main.tex",expectedDiskSha256:disk.sha256}]}};
  const prepare=async(input=request)=>side==="ordinary"?runtime.gateways.submission.prepareWriting(input):
    (await runtime.compaction.gateway.prepareWriting({request:input,expectedRevision:(await agent.history.gateway.open(scope.conversationId)).watermark})).preview;
  const startRequest=(preview:AgentSubmissionPreview):AgentWritingStartRequest=>({...scope,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId,
    model:preview.input.model,mode:executionModeForModel(preview.input.model,preview.input.reasoningEffort)!});
  const methods=(value:typeof runtime,history:typeof agent)=>({
    status:(preview:AgentSubmissionPreview)=>side==="ordinary"?value.gateways.submission.startStatus(startRequest(preview)):
      value.compaction.gateway.startStatus(agentWritingCompactionStartRequest({preview,expectedRevision:value.compaction.snapshot().records.get(JSON.stringify([preview.conversationId,preview.requestId]))!.prepared.expectedRevision})),
    events:()=>history.history.gateway.readHistory({conversationId:scope.conversationId,afterSeq:0,limit:500}).then(page=>page.events),
  });
  const start=async(preview:AgentSubmissionPreview)=>{
    if(side==="ordinary"){
      await runtime.gateways.workspace.prepare({scope:{...scope,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId},buffers:[]});
      await runtime.gateways.submission.startWriting(startRequest(preview));
      await runtime.execution.turns.interrupt({conversationId:scope.conversationId,turnId:preview.turnId});
    }else{
      const record=runtime.compaction.snapshot().records.get(JSON.stringify([preview.conversationId,preview.requestId]))!;
      await runtime.compaction.gateway.start(agentWritingCompactionStartRequest(record.prepared));await vi.advanceTimersByTimeAsync(1000);
    }
  };
  const reopen=()=>{const records=runtime.execution.snapshot(),compact=runtime.compaction.snapshot(),queue=runtime.queue.snapshot(),events=agent.history.snapshot();
    const nextAgent=new WritingDiffDemoAgentStorage(storage,undefined),next=createWritingDiffDemoRuntime(storage,nextAgent,scope.manuscriptId,scope.conversationId);
    nextAgent.history.restore(events);next.execution.restore(records);next.queue.restore(queue);next.compaction.restore(compact);cleanups.push(()=>next.dispose());
    return {runtime:next,agent:nextAgent,...methods(next,nextAgent)};};
  return {scope,storage,agent,runtime,request,prepare,start,startRequest,reopen,...methods(runtime,agent)};
}
it.each(["ordinary","manual"] as const)("%s status distinguishes real frozen preparation and actual history after snapshot/reopen",async side=>{
  vi.useFakeTimers();const f=await fixture(side),preview=await f.prepare();
  expect((await f.status(preview)).status.type).toBe("prepared");expect(await f.events()).toHaveLength(0);
  await f.start(preview);const original=await f.status(preview);expect(original.status).toMatchObject({type:"started",result:{turnId:preview.turnId,alreadyStarted:true}});
  const history=await f.events(),reopened=f.reopen();
  const read=vi.spyOn(f.storage.files,"readFile").mockRejectedValue(new Error("current source absent"));
  const prepare=vi.spyOn(reopened.runtime.gateways.submission,"prepareWriting"),start=vi.spyOn(reopened.runtime.gateways.submission,"startWriting");
  const compact=vi.spyOn(reopened.runtime.gateways.commands!,"compact");
  expect(await reopened.status(preview)).toEqual(original);expect(await reopened.events()).toEqual(history);
  expect(read).not.toHaveBeenCalled();expect(prepare).not.toHaveBeenCalled();expect(start).not.toHaveBeenCalled();expect(compact).not.toHaveBeenCalled();
});
it.each(["ordinary","manual"] as const)("%s stored execution flag cannot replace a real TurnStarted fact",async side=>{
  const f=await fixture(side),preview=await f.prepare();
  if(side==="ordinary"){const records=f.runtime.execution.snapshot();f.runtime.execution.restore(records.map(([key,row])=>[key,{...row,started:true,outcome:"completed"}] as const));}
  else {const records=f.runtime.compaction.snapshot();for(const row of records.records.values()){row.started=true;row.outcome="completed";}f.runtime.compaction.restore(records);}
  expect((await f.status(preview)).status.type).toBe("prepared");expect(await f.events()).toHaveLength(0);
});
it("legacy ordinary records need no invented preparation proof to observe an actual accepted task",async()=>{
  const f=await fixture("ordinary"),preview=await f.prepare();await f.start(preview);const original=await f.status(preview);
  f.runtime.execution.restore(f.runtime.execution.snapshot().map(([key,row])=>{const {preparation:_proof,purpose:_purpose,...legacy}=row;return [key,legacy] as const;}));
  expect(await f.status(preview)).toEqual(original);
});
it.each(["ordinary","manual"] as const)("%s exact scope/model/purpose checks remain independent of mutable dependencies",async side=>{
  const f=await fixture(side),preview=await f.prepare();
  if(side==="ordinary"){
    for(const patch of [{contextSnapshotId:crypto.randomUUID()},{manuscriptId:crypto.randomUUID()},{model:{...preview.input.model,revision:"changed"}}])
      expect((await f.runtime.gateways.submission.startStatus({...f.startRequest(preview),...patch})).status.type).toBe("unconfirmed");
    expect((await f.runtime.compaction.gateway.startStatus(agentWritingCompactionStartRequest({preview:{...preview,input:{...preview.input,text:"/compact"}},expectedRevision:0}))).status.type).toBe("unconfirmed");
  }else{
    const request=agentWritingCompactionStartRequest({preview,expectedRevision:0});
    expect((await f.runtime.compaction.gateway.startStatus({...request,expectedRevision:1})).status.type).toBe("unconfirmed");
    expect((await f.runtime.gateways.submission.startStatus(f.startRequest(preview))).status.type).toBe("unconfirmed");
  }
});
it.each(["ordinary","manual"] as const)("%s rollback hides the visible turn but the original accepted fact remains observable",async side=>{
  vi.useFakeTimers();const f=await fixture(side);
  if(side==="manual"){
    const prior=await f.runtime.gateways.submission.prepareWriting({...f.request,requestId:crypto.randomUUID(),text:"Earlier ordinary"});
    await f.runtime.gateways.workspace.prepare({scope:{...f.scope,requestId:prior.requestId,contextSnapshotId:prior.contextSnapshotId},buffers:[]});
    await f.runtime.gateways.submission.startWriting(f.startRequest(prior));
    await f.runtime.execution.turns.interrupt({conversationId:f.scope.conversationId,turnId:prior.turnId});
  }
  const preview=await f.prepare();await f.start(preview);const original=await f.status(preview);
  // This preview has no rollback writer. Restore its data snapshot with a valid
  // immutable rollback marker; the original execution above was real mock I/O.
  const snapshot=f.agent.history.snapshot(),events=snapshot.events.get(f.scope.conversationId)!;
  snapshot.events.set(f.scope.conversationId,[...events,decodeAgentEvent({schemaVersion:1,conversationId:f.scope.conversationId,
    seq:events.length+1,timestampMs:Date.now(),requestId:crypto.randomUUID(),payload:{type:"conversationRolledBack",turnId:preview.turnId,userTurns:1}})]);
  f.agent.history.restore(snapshot);
  const projection=applyAgentEvents(createAgentConversationProjection(f.scope.conversationId),await f.events());expect(projection.turns.has(preview.turnId)).toBe(false);
  expect(await f.status(preview)).toEqual(original);expect(await f.reopen().status(preview)).toEqual(original);
});
it("manual read does not consume a configured real start ACK-loss fault",async()=>{
  vi.useFakeTimers();const f=await fixture("manual"),preview=await f.prepare();
  f.runtime.compaction.failNext("start","afterCommit");expect((await f.status(preview)).status.type).toBe("prepared");
  await expect(f.runtime.compaction.gateway.start(agentWritingCompactionStartRequest({preview,expectedRevision:0}))).rejects.toThrow();
  expect((await f.status(preview)).status.type).toBe("started");await vi.advanceTimersByTimeAsync(1000);
});
it.each(["ordinary","manual"] as const)("%s observations cannot reinterpret an actual queued preparation",async side=>{
  const f=await fixture(side),inputId=crypto.randomUUID();
  const queue=await f.runtime.queue.gateway.edit({...f.scope,requestId:crypto.randomUUID(),expectedRevision:0,
    edit:{type:"add",inputId,text:f.request.text}});
  const prepared=side==="ordinary"?(await f.runtime.queue.preview.prepare({inputId,expectedRevision:queue.revision,submission:f.request})).preview:
    (await f.runtime.compaction.gateway.prepareWritingQueued({inputId,expectedQueueRevision:queue.revision,previousRequestId:null,
      expectedRevision:0,submission:f.request})).preview;
  expect((await f.status(prepared)).status.type).toBe("unconfirmed");expect(await f.events()).toHaveLength(0);
});
it.each(["ordinary","manual"] as const)("%s supplemental-only history cannot prove the original writing manuscript before or after an actual start",async side=>{
  vi.useFakeTimers();const f=await fixture(side),hash="a".repeat(64),resource={kind:"manuscriptFile" as const,resourceId:crypto.randomUUID(),path:"foreign.tex"};
  f.runtime.execution.contextSources=async()=>[{sourceId:`resource:${hash}`,revision:`grant:${hash}`,kind:"manuscript",resourceId:resource.path,
    anchor:JSON.stringify({supplementalContext:true,manuscriptId:f.scope.manuscriptId,content:"resourceGrant",resource,contextAccess:"readOnly"}),text:"authorized supplemental file"}];
  const preview=await f.prepare({...f.request,context:{...f.request.context,files:[]},resources:[resource]});
  const check=async()=>{
    expect((await f.status(preview)).status).toMatchObject({type:"unconfirmed",detail:{safeCause:"writing_start_scope_unconfirmed"}});
    const manuscriptId=crypto.randomUUID();
    const wrong=side==="ordinary"?await f.runtime.gateways.submission.startStatus({...f.startRequest(preview),manuscriptId}):
      await f.runtime.compaction.gateway.startStatus({...agentWritingCompactionStartRequest({preview,expectedRevision:0}),host:{kind:"manuscript",manuscriptId}});
    expect(wrong.status.type).toBe("unconfirmed");
  };
  await check();expect(await f.events()).toHaveLength(0);await f.start(preview);await check();
  const original=await f.events(),reopened=f.reopen(),read=vi.spyOn(f.storage.files,"readFile").mockRejectedValue(new Error("current source absent"));
  expect((await reopened.status(preview)).status).toMatchObject({type:"unconfirmed",detail:{safeCause:"writing_start_scope_unconfirmed"}});
  expect(await reopened.events()).toEqual(original);expect(read).not.toHaveBeenCalled();
});
it.each(["ordinary","manual"] as const)("%s retrieval-only frozen anchor can prove the historical writing manuscript",async side=>{
  vi.useFakeTimers();const f=await fixture(side),traceId=crypto.randomUUID();
  f.runtime.execution.retrievalPreparation=async()=>({context:[{sourceId:`retrieval:${traceId}`,revision:`trace:${traceId}`,kind:"retrieval",resourceId:"global-research",
    anchor:JSON.stringify({manuscriptId:f.scope.manuscriptId,traceId}),text:"frozen retrieval source"}],commit:accept=>accept()});
  const preview=await f.prepare({...f.request,context:{...f.request.context,files:[]},retrieval:{traceId,query:f.request.text}});
  expect((await f.status(preview)).status.type).toBe("prepared");await f.start(preview);
  expect((await f.status(preview)).status).toMatchObject({type:"started",result:{turnId:preview.turnId,alreadyStarted:true}});
  expect((await f.reopen().status(preview)).status.type).toBe("started");
});
