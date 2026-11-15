import {expect,it,vi} from "vitest";
import type {AgentSteeringHandoffIntent} from "@research-assistant/core/agent";
import {agentSteeringHandoffResolvedId,type AgentSubmissionGateway} from "@research-assistant/data/agent";
import {WritingDiffDemoHistory} from "./WritingDiffDemoHistory";
import {WritingDiffDemoQueue} from "./WritingDiffDemoQueue";
import type {WritingDiffDemoSubmission} from "./WritingDiffDemoSubmission";
import {createWritingDemoHandoffGateway} from "./WritingDemoHandoffGateway";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";

it("derives receipts and settles only after original mock history and admission agree",async()=>{
  const conversationId=crypto.randomUUID(),manuscriptId=crypto.randomUUID(),turnId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const history=new WritingDiffDemoHistory(),queue=new WritingDiffDemoQueue({} as AgentSubmissionGateway);
  const turn={scope:{conversationId,manuscriptId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId};
  history.start(turn,"Original task");
  const scope={conversationId,manuscriptId},added=await queue.gateway.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Original queued input"}});
  const epoch=queue.handoffDataEpoch(),intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId,steeringInputId:crypto.randomUUID(),queueInputId:inputId,
    expectedRevision:added.revision,expectedEpoch:epoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,
    origin:added.inputs[0]?.origin??null,text:"Original queued input",userInput:[]};
  const admission=vi.fn(()=>"absent" as "absent"|"matching"|"conflicting");
  const submission={steeringHandoffAdmission:admission} as unknown as WritingDiffDemoSubmission;
  const gateway=createWritingDemoHandoffGateway(queue,history,submission,manuscriptId,async()=>undefined);
  expect((await gateway.prepare(intent)).operationId).toBe(intent.operationId);
  expect((await gateway.read(conversationId,epoch)).items[0]?.history).toBe("unconfirmed");
  await expect(gateway.finish(intent,epoch)).rejects.toMatchObject({code:"conflict"});
  history.steer(turn,intent.steeringInputId,intent.text);admission.mockReturnValue("matching");
  expect((await gateway.read(conversationId,epoch)).items[0]?.history).toBe("confirmed");
  const proof=await gateway.finish(intent,epoch);
  expect(proof.operationId).toBe(await agentSteeringHandoffResolvedId("agent.steering.handoff.finish",intent.operationId));
  expect((await queue.gateway.read(scope)).inputs).toHaveLength(0);
  expect(await gateway.resolution(intent,epoch)).toBe("finished");
  expect((await gateway.finish(intent,epoch)).replayed).toBe(true);
  for(const requestId of [intent.operationId,proof.operationId])
    await expect(queue.gateway.edit({...scope,requestId,expectedRevision:2,edit:{type:"add",inputId:crypto.randomUUID(),text:"Unrelated"}})).rejects.toMatchObject({code:"conflict"});
  history.dispose();
});

it("retains an unconsumed item after a terminal turn and replays the release receipt",async()=>{
  const conversationId=crypto.randomUUID(),manuscriptId=crypto.randomUUID(),turnId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const history=new WritingDiffDemoHistory(),queue=new WritingDiffDemoQueue({} as AgentSubmissionGateway);
  const turn={scope:{conversationId,manuscriptId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId};history.start(turn,"Task");
  const scope={conversationId,manuscriptId},added=await queue.gateway.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Pending"}});
  const epoch=queue.handoffDataEpoch(),intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId,steeringInputId:crypto.randomUUID(),queueInputId:inputId,
    expectedRevision:added.revision,expectedEpoch:epoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,
    origin:added.inputs[0]?.origin??null,text:"Pending",userInput:[]};
  const submission={steeringHandoffAdmission:()=>"absent"} as unknown as WritingDiffDemoSubmission;
  const gateway=createWritingDemoHandoffGateway(queue,history,submission,manuscriptId,async()=>undefined);
  await gateway.prepare(intent);await expect(gateway.release(intent,epoch)).rejects.toMatchObject({code:"conflict"});
  history.finish(turn,"canceled");const receipt=await gateway.release(intent,epoch);
  expect(receipt.operationId).toBe(await agentSteeringHandoffResolvedId("agent.steering.handoff.release",intent.operationId));
  expect(await queue.gateway.read(scope)).toMatchObject({paused:true,inputs:[{inputId,text:"Pending"}]});
  expect(await gateway.resolution(intent,epoch)).toBe("released");expect((await gateway.release(intent,epoch)).replayed).toBe(true);
  history.dispose();
});

it("runs a scoped handoff through the composed writing mock and checks its original receipt after reopen",async()=>{
  vi.useFakeTimers();const manuscriptId=crypto.randomUUID(),conversationId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const files=new WritingDiffDemoStorage(manuscriptId,[{path:"main.tex",before:"text",after:"text",kind:"modified",additions:0,deletions:0}]);
  const agent=new WritingDiffDemoAgentStorage(files,undefined),runtime=createWritingDiffDemoRuntime(files,agent,manuscriptId,conversationId);
  let reopened:ReturnType<typeof createWritingDiffDemoRuntime>|undefined;
  try{
    runtime.execution.historyOnly=true;const file=await files.files.readFile(manuscriptId,"main.tex");
    const preview=await runtime.execution.submission.prepareWriting({conversationId,requestId:crypto.randomUUID(),text:"Task",context:{manuscriptId,files:[{relativePath:"main.tex",expectedDiskSha256:file.sha256}]}});
    const scope={conversationId,manuscriptId,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId};
    await runtime.execution.workspace.prepare({scope,buffers:[]});
    await runtime.execution.submission.startWriting({...scope,model:preview.input.model,mode:"deepSeekNonThinking"});
    const added=await runtime.queue.gateway.edit({conversationId,manuscriptId,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Queued"}});
    const gateway=runtime.gateways.context!.steeringHandoff!,page=await gateway.read(conversationId),intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId:preview.turnId,
      steeringInputId:crypto.randomUUID(),queueInputId:inputId,expectedRevision:added.revision,expectedEpoch:page.epoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,
      queuePaused:added.paused,inputPaused:false,origin:added.inputs[0]?.origin??null,text:"Queued",userInput:[]};
    await gateway.prepare(intent);
    await runtime.execution.steer({conversationId,manuscriptId,turnId:preview.turnId,inputId:intent.steeringInputId,text:intent.text,userInput:[]});
    runtime.execution.consumeSteersAtFixtureBoundary(conversationId,preview.turnId);expect((await gateway.read(conversationId,page.epoch)).items[0]?.history).toBe("confirmed");
    await gateway.finish(intent,page.epoch);
    const nextAgent=new WritingDiffDemoAgentStorage(files,undefined);reopened=createWritingDiffDemoRuntime(files,nextAgent,manuscriptId,conversationId);
    reopened.conversations.restore(runtime.conversations.snapshot());nextAgent.history.restore(agent.history.snapshot());
    reopened.execution.restore(runtime.execution.snapshot());reopened.queue.restore(runtime.queue.snapshot());
    const next=reopened.gateways.context!.steeringHandoff!;
    expect(await next.resolution(intent,page.epoch)).toBe("finished");expect((await next.finish(intent,page.epoch)).replayed).toBe(true);
    expect((await reopened.queue.gateway.read({conversationId,manuscriptId})).inputs).toHaveLength(0);
  }finally{runtime.dispose();reopened?.dispose();vi.useRealTimers();}
});

it("releases an admitted but unconsumed steer after cancellation without sending it again",async()=>{
  vi.useFakeTimers();const manuscriptId=crypto.randomUUID(),conversationId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const files=new WritingDiffDemoStorage(manuscriptId,[{path:"main.tex",before:"text",after:"text",kind:"modified",additions:0,deletions:0}]);
  const agent=new WritingDiffDemoAgentStorage(files,undefined),runtime=createWritingDiffDemoRuntime(files,agent,manuscriptId,conversationId);
  try{
    runtime.execution.historyOnly=true;const file=await files.files.readFile(manuscriptId,"main.tex");
    const preview=await runtime.execution.submission.prepareWriting({conversationId,requestId:crypto.randomUUID(),text:"Task",context:{manuscriptId,files:[{relativePath:"main.tex",expectedDiskSha256:file.sha256}]}});
    const scope={conversationId,manuscriptId,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId};
    await runtime.execution.workspace.prepare({scope,buffers:[]});
    await runtime.execution.submission.startWriting({...scope,model:preview.input.model,mode:"deepSeekNonThinking"});
    const added=await runtime.queue.gateway.edit({conversationId,manuscriptId,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Queued"}});
    const gateway=runtime.gateways.context!.steeringHandoff!,page=await gateway.read(conversationId),intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId:preview.turnId,
      steeringInputId:crypto.randomUUID(),queueInputId:inputId,expectedRevision:added.revision,expectedEpoch:page.epoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,
      queuePaused:added.paused,inputPaused:false,origin:added.inputs[0]?.origin??null,text:"Queued",userInput:[]};
    const request={conversationId,manuscriptId,turnId:preview.turnId,inputId:intent.steeringInputId,text:intent.text,userInput:[]};
    await gateway.prepare(intent);await runtime.execution.steer(request);
    expect(runtime.execution.steeringHandoffAdmission(intent)).toBe("matching");
    await expect(gateway.release(intent,page.epoch)).rejects.toMatchObject({code:"conflict"});
    expect(runtime.execution.steeringHandoffAdmission(intent)).toBe("matching");
    await runtime.execution.turns.interrupt({conversationId,turnId:preview.turnId});
    expect((await gateway.read(conversationId,page.epoch)).items[0]?.history).toBe("unconfirmed");
    await gateway.release(intent,page.epoch);
    expect(runtime.execution.steeringHandoffAdmission(intent)).toBe("absent");
    expect(await runtime.queue.gateway.read({conversationId,manuscriptId})).toMatchObject({paused:true,inputs:[{inputId,text:"Queued"}]});
    expect((await gateway.release(intent,page.epoch)).replayed).toBe(true);
    await expect(runtime.execution.steer(request)).rejects.toMatchObject({code:"closed"});
    await vi.advanceTimersByTimeAsync(3000);
    expect(agent.history.steeringHandoffHistory(intent)).toBe("unconfirmed");
  }finally{runtime.dispose();vi.useRealTimers();}
});
