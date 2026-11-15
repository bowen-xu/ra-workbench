// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {AgentSteeringHandoffIntent} from "@research-assistant/core/agent";
import {MockWritingAgents} from "./MockWritingAgents";
import {createMockWritingGateway} from "./mockWritingGateway";
import {createBrowserWritingCheckpointGateway} from "../../../../../../plugins/writing_projects/src/features/writing/gateway/browserWritingCheckpointGateway";
import {createPreviewControls} from "../../../core/runtime/previewControls";
import {createTestNamespacedStorage} from "../../../core/runtime/testing/createTestNamespacedStorage";
import {registerWritingRecords} from "../../../core/features/writing-storage/browserWritingRecords";
import {memoryWritingRecords, memoryWritingState} from "../testing/memoryWritingRecords";

const owners:MockWritingAgents[]=[];
afterEach(()=>{for(const owner of owners.splice(0))owner.dispose();vi.useRealTimers();});
async function fixture(){
  const storage=createTestNamespacedStorage(localStorage,`${crypto.randomUUID()}:`);
  registerWritingRecords(storage,memoryWritingRecords(memoryWritingState()));
  const writing=createMockWritingGateway(storage),manuscript=await writing.createManuscript({title:"Handoff test"});
  const controls=createPreviewControls(storage,()=>undefined);controls.configure({delayMs:0,nextFailure:null});
  const open=()=>{const owner=new MockWritingAgents(storage,writing,createBrowserWritingCheckpointGateway(storage),controls);owners.push(owner);return owner;};
  const owner=open(),backend=owner.backend(manuscript.id),conversationId=backend.conversationId;
  return{open,owner,backend,writing,conversationId,manuscriptId:manuscript.id,ports:owner.gateways()};
}
async function queued(f:Awaited<ReturnType<typeof fixture>>,turnId:string){
  const inputId=crypto.randomUUID(),scope={conversationId:f.conversationId,manuscriptId:f.manuscriptId};
  const queue=await f.ports.agentInputQueueGateway!.edit({...scope,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add",inputId,text:"Queued instruction"}});
  const page=await f.owner.steeringHandoff.read(f.conversationId);
  const intent:AgentSteeringHandoffIntent={operationId:crypto.randomUUID(),conversationId:f.conversationId,turnId,
    steeringInputId:crypto.randomUUID(),queueInputId:inputId,expectedRevision:queue.revision,expectedEpoch:page.epoch,
    scope:{kind:"manuscript",manuscriptId:f.manuscriptId},queueIndex:0,queuePaused:false,inputPaused:false,
    origin:queue.inputs[0]?.origin??null,text:"Queued instruction",userInput:[]};
  return{intent,epoch:page.epoch,scope};
}

it("reads the exact idle conversation and rejects unknown identities or stale epochs",async()=>{
  const f=await fixture(),page=await f.owner.steeringHandoff.read(f.conversationId);
  expect(page).toMatchObject({conversationId:f.conversationId,items:[]});
  expect((await f.owner.steeringHandoff.read(f.conversationId,page.epoch)).epoch).toBe(page.epoch);
  await expect(f.owner.steeringHandoff.read(crypto.randomUUID())).rejects.toThrow("MOCK_HANDOFF_CONVERSATION_UNAVAILABLE");
  await expect(f.owner.steeringHandoff.read(f.conversationId,crypto.randomUUID())).rejects.toMatchObject({code:"conflict"});
});

it("routes queued steering through the original admission and persists its finish receipt",async()=>{
  vi.useFakeTimers();const f=await fixture(),{execution}=f.backend.runtime;
  execution.historyOnly=true;
  const file=await f.writing.readFile(f.manuscriptId,"main.tex");
  const preview=await f.ports.agentSubmissionGateway!.prepareWriting({conversationId:f.conversationId,requestId:crypto.randomUUID(),text:"Task",
    context:{manuscriptId:f.manuscriptId,files:[{relativePath:"main.tex",expectedDiskSha256:file.sha256}]}});
  const scope={conversationId:f.conversationId,manuscriptId:f.manuscriptId,requestId:preview.requestId,contextSnapshotId:preview.contextSnapshotId};
  await f.ports.agentWorkspaceGateway!.prepare({scope,buffers:[]});
  await f.ports.agentSubmissionGateway!.startWriting({...scope,model:preview.input.model,mode:"deepSeekNonThinking"});
  const {intent,epoch}=await queued(f,preview.turnId),gateway=f.owner.steeringHandoff;
  await gateway.prepare(intent);
  await f.ports.agentSteeringGateway!.steerWriting({conversationId:f.conversationId,manuscriptId:f.manuscriptId,turnId:preview.turnId,
    inputId:intent.steeringInputId,text:intent.text,userInput:[]});
  await expect(gateway.finish(intent,epoch)).rejects.toMatchObject({code:"conflict"});
  execution.consumeSteersAtFixtureBoundary(f.conversationId,preview.turnId);
  expect((await gateway.read(f.conversationId,epoch)).items[0]?.history).toBe("confirmed");
  await gateway.finish(intent,epoch);
  const reopened=f.open(),next=reopened.steeringHandoff;
  expect(await next.resolution(intent,epoch)).toBe("finished");
  expect((await next.finish(intent,epoch)).replayed).toBe(true);
  expect((await reopened.gateways().agentInputQueueGateway!.read({conversationId:f.conversationId,manuscriptId:f.manuscriptId})).inputs).toEqual([]);
});

it("preserves an unresolved handoff across reopening and releases its original queued input",async()=>{
  const f=await fixture(),turn={scope:{conversationId:f.conversationId,manuscriptId:f.manuscriptId,requestId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID()},turnId:crypto.randomUUID()};
  f.backend.agent.history.start(turn,"Task");
  const {intent,epoch,scope}=await queued(f,turn.turnId);
  await f.owner.steeringHandoff.prepare(intent);
  const reopened=f.open(),gateway=reopened.steeringHandoff;
  expect((await gateway.read(f.conversationId,epoch)).items).toMatchObject([{intent,history:"unconfirmed"}]);
  await gateway.release(intent,epoch);
  expect(await gateway.resolution(intent,epoch)).toBe("released");
  expect(await reopened.gateways().agentInputQueueGateway!.read(scope)).toMatchObject({paused:true,inputs:[{inputId:intent.queueInputId,text:intent.text}]});
});
