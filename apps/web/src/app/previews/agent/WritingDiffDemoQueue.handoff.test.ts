import {expect,it} from "vitest";
import type {AgentSteeringHandoffIntent} from "@research-assistant/core/agent";
import type {AgentSubmissionGateway} from "@research-assistant/data/agent";
import {WritingDiffDemoQueue} from "./WritingDiffDemoQueue";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";

it("retains a frozen queue handoff across mock reopen and rejects conflicting edits",async()=>{
  const conversationId=crypto.randomUUID(),manuscriptId=crypto.randomUUID(),turnId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const scope={conversationId,manuscriptId},queue=new WritingDiffDemoQueue({} as AgentSubmissionGateway);
  const added=await queue.gateway.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Frozen queue input"}});
  const intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId,steeringInputId:crypto.randomUUID(),queueInputId:inputId,
    expectedRevision:added.revision,expectedEpoch:queue.snapshot().handoffEpoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,
    queuePaused:added.paused,inputPaused:!!added.inputs[0]?.paused,origin:added.inputs[0]?.origin??null,text:"Frozen queue input",userInput:[]};
  expect(()=>queue.claimSteeringHandoff({...intent,text:"changed"},turnId)).toThrow();
  expect(queue.claimSteeringHandoff(intent,turnId)).toEqual({epoch:intent.expectedEpoch,replayed:false});
  expect(queue.claimSteeringHandoff(intent,turnId)).toEqual({epoch:intent.expectedEpoch,replayed:true});
  await expect(queue.gateway.edit({...scope,requestId:intent.operationId,expectedRevision:added.revision,edit:{type:"add",inputId:crypto.randomUUID(),text:"Other input"}})).rejects.toMatchObject({code:"conflict"});
  await expect(queue.gateway.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:added.revision,edit:{type:"remove",inputId}})).rejects.toMatchObject({code:"conflict"});
  const restored=new WritingDiffDemoQueue({} as AgentSubmissionGateway);restored.restore(queue.snapshot());
  expect(restored.claimSteeringHandoff(intent,null)).toEqual({epoch:intent.expectedEpoch,replayed:true});
  const current=await restored.gateway.read(scope);
  await expect(restored.gateway.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:current.revision,edit:{type:"remove",inputId}})).rejects.toMatchObject({code:"conflict"});
  expect(current.inputs.map(item=>item.inputId)).toEqual([inputId]);
});

it("fences an old handoff if the bound mock data environment changes",async()=>{
  const conversationId=crypto.randomUUID(),manuscriptId=crypto.randomUUID(),turnId=crypto.randomUUID(),inputId=crypto.randomUUID();
  let epoch=crypto.randomUUID();
  const queue=new WritingDiffDemoQueue({} as AgentSubmissionGateway);queue.bindHandoffEnvironment(()=>epoch);
  const added=await queue.gateway.edit({conversationId,manuscriptId,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Original"}});
  const intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId,steeringInputId:crypto.randomUUID(),queueInputId:inputId,
    expectedRevision:added.revision,expectedEpoch:epoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,
    origin:added.inputs[0]?.origin??null,text:"Original",userInput:[]};
  expect(queue.claimSteeringHandoff(intent,turnId).epoch).toBe(epoch);
  epoch=crypto.randomUUID();
  expect(()=>queue.handoffDataEpoch()).toThrow();
  expect(()=>queue.claimSteeringHandoff(intent,turnId)).toThrow();
  await expect(queue.gateway.edit({conversationId,manuscriptId,requestId:crypto.randomUUID(),expectedRevision:added.revision,edit:{type:"remove",inputId}})).rejects.toMatchObject({code:"conflict"});
});

it("binds the preview handoff epoch to the conversation catalog and fences mixed snapshots",()=>{
  const manuscriptId=crypto.randomUUID(),conversationId=crypto.randomUUID();
  const files=new WritingDiffDemoStorage(manuscriptId,[]),agent=new WritingDiffDemoAgentStorage(files,undefined);
  const runtime=createWritingDiffDemoRuntime(files,agent,manuscriptId,conversationId);
  const otherAgent=new WritingDiffDemoAgentStorage(files,undefined),other=createWritingDiffDemoRuntime(files,otherAgent,manuscriptId,conversationId);
  try{
    expect(runtime.queue.handoffDataEpoch()).toBe(runtime.conversations.dataEpoch());
    expect(other.queue.handoffDataEpoch()).not.toBe(runtime.queue.handoffDataEpoch());
    other.queue.restore(runtime.queue.snapshot());
    expect(()=>other.queue.handoffDataEpoch()).toThrow();
    other.conversations.restore(runtime.conversations.snapshot());
    expect(other.queue.handoffDataEpoch()).toBe(runtime.conversations.dataEpoch());
  }finally{runtime.dispose();other.dispose();}
});

it("settles only the proven original input and preserves a replayable result",async()=>{
  const conversationId=crypto.randomUUID(),manuscriptId=crypto.randomUUID(),turnId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const queue=new WritingDiffDemoQueue({} as AgentSubmissionGateway),scope={conversationId,manuscriptId};
  const added=await queue.gateway.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Original"}});
  const epoch=queue.handoffDataEpoch(),intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId,steeringInputId:crypto.randomUUID(),queueInputId:inputId,
    expectedRevision:added.revision,expectedEpoch:epoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,
    origin:added.inputs[0]?.origin??null,text:"Original",userInput:[]},operationId=crypto.randomUUID();
  queue.claimSteeringHandoff(intent,turnId);
  expect(()=>queue.settleSteeringHandoff(intent,epoch,"finish",operationId,"confirmed","conflicting",true)).toThrow();
  expect(()=>queue.settleSteeringHandoff(intent,epoch,"finish",operationId,"unconfirmed","matching",true)).toThrow();
  expect((await queue.gateway.read(scope)).inputs).toHaveLength(1);
  expect(queue.settleSteeringHandoff(intent,epoch,"finish",operationId,"confirmed","matching",true).replayed).toBe(false);
  expect((await queue.gateway.read(scope)).inputs).toHaveLength(0);
  const reopened=new WritingDiffDemoQueue({} as AgentSubmissionGateway);reopened.restore(queue.snapshot());
  expect(reopened.steeringHandoffResolution(intent,epoch)).toBe("finished");
  expect(reopened.settleSteeringHandoff(intent,epoch,"finish",operationId,"unconfirmed","absent",false).replayed).toBe(true);
  expect(()=>reopened.claimSteeringHandoff(intent,turnId)).toThrow();
});

it("releases an unconsumed terminal input without deleting it and fences a canceled registration",async()=>{
  const conversationId=crypto.randomUUID(),manuscriptId=crypto.randomUUID(),turnId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const queue=new WritingDiffDemoQueue({} as AgentSubmissionGateway),scope={conversationId,manuscriptId};
  const added=await queue.gateway.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Original"}});
  const epoch=queue.handoffDataEpoch(),intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId,turnId,steeringInputId:crypto.randomUUID(),queueInputId:inputId,
    expectedRevision:added.revision,expectedEpoch:epoch,scope:{kind:"manuscript",manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,
    origin:added.inputs[0]?.origin??null,text:"Original",userInput:[]};
  queue.claimSteeringHandoff(intent,turnId);
  expect(()=>queue.settleSteeringHandoff(intent,epoch,"release",crypto.randomUUID(),"unconfirmed","matching",true)).toThrow();
  const discard=()=>{throw new Error("discard must not run for an invalid release");};
  expect(()=>queue.settleSteeringHandoff(intent,epoch,"release",crypto.randomUUID(),"unconfirmed","matching",false,discard)).toThrow();
  expect(()=>queue.settleSteeringHandoff(intent,epoch,"release",crypto.randomUUID(),"incompleteOrConflicting","matching",true,discard)).toThrow();
  expect(()=>queue.settleSteeringHandoff(intent,epoch,"release",crypto.randomUUID(),"unconfirmed","conflicting",true,discard)).toThrow();
  queue.settleSteeringHandoff(intent,epoch,"release",crypto.randomUUID(),"unconfirmed","absent",true);
  const retained=await queue.gateway.read(scope);expect(retained.paused).toBe(true);expect(retained.inputs[0]?.inputId).toBe(inputId);
  const other={...intent,operationId:crypto.randomUUID(),steeringInputId:crypto.randomUUID(),expectedRevision:retained.revision,queuePaused:true};
  queue.settleSteeringHandoff(other,epoch,"cancel",crypto.randomUUID(),"unconfirmed","absent",true);
  expect(()=>queue.claimSteeringHandoff(other,turnId)).toThrow();
});
