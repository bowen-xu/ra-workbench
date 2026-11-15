// @vitest-environment happy-dom
import {expect, it, vi} from "vitest";
import {decodeAgentWritingPrepareRequest} from "@research-assistant/core/agent";
import golden from "../../../../../tests/fixtures/agent-writing-prepare-native-v1.json";
import {getAppRuntime, selectAppRuntime} from "../runtime/appRuntime";
import {createTestNamespacedStorage} from "../runtime/testing/createTestNamespacedStorage";
import {registerStoragePersistence} from "../runtime/storagePersistence";
import {freezeNewPreviewOwnerIntent, NATIVE_PREVIEW_OWNER_CAPABILITY} from "../runtime/nativePreviewOwnerIntent";
import type {NativeAgentPreview} from "../runtime/nativeAgentPreview";
import {agentInputQueuePreviewGateway, agentSubmissionGateway} from "./agentGateway";
import {agentContextGateway} from "./agentContextGateway";

it("uses the frozen owner through production ordinary and queued gateways after reopening", async () => {
  const previous = getAppRuntime(), prefix = `owner-gateway-${crypto.randomUUID()}:`;
  const storage = createTestNamespacedStorage(window.localStorage, prefix);
  registerStoragePersistence(storage, {backend:"IndexedDB",pending:false,error:undefined,resetting:false,
    flush:async()=>{},retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  const fixtureId = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  const preview = {bootstrap:{generation:"generation",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},
    snapshotConversationOwner:()=>({generation:"generation",fixtureId}), invoke:vi.fn(async (_command:string, args:unknown) => ({schemaVersion:1,status:"missing",generation:"generation",...(args as {request:{conversationId:string; requestId:string}}).request}))} as unknown as NativeAgentPreview;
  const request = decodeAgentWritingPrepareRequest(golden.ordinary.intent);
  await freezeNewPreviewOwnerIntent(preview,storage,request.conversationId,request.requestId);
  const reopened = createTestNamespacedStorage(window.localStorage,prefix);
  registerStoragePersistence(reopened, {backend:"IndexedDB",pending:false,error:undefined,resetting:false,
    flush:async()=>{},retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  const failure = new Error("transport reached");
  const invoke = vi.fn().mockRejectedValue(failure);
  selectAppRuntime({mode:"preview", storage:reopened, gateways:{},nativeAgentPreview:preview,invoke,dispose(){}});
  try {
    await expect(agentSubmissionGateway.prepareWriting(request)).rejects.toBe(failure);
    expect(invoke).toHaveBeenCalledWith("agent_prepare_writing_submission", {request}, {previewOwnerIntent:{conversationId:request.conversationId,fixtureId}});
    const id = (n:number) => `abcdef00-1234-5678-abcd-${String(n).padStart(12,"0")}`;
    const queued = {inputId:id(1),expectedRevision:8,start:{conversationId:request.conversationId,requestId:request.requestId,
      manuscriptId:id(3),contextSnapshotId:id(5),model:{providerId:"provider",modelId:"model",revision:null},mode:"chatGptDefault" as const}};
    await expect(agentInputQueuePreviewGateway.start(queued)).rejects.toBe(failure);
    expect(invoke).toHaveBeenCalledWith("agent_start_writing_queued_submission", {request:queued}, {previewOwnerIntent:{conversationId:request.conversationId,fixtureId}});
    const independent = {conversationId:request.conversationId, requestId:request.requestId, text:"test", resources:[]};
    await expect(agentContextGateway.prepare(independent)).rejects.toBe(failure);
    expect(invoke).toHaveBeenCalledWith("agent_prepare_context_submission", {request:independent}, {previewOwnerIntent:{conversationId:request.conversationId,fixtureId}});
    const calls = invoke.mock.calls.length;
    await expect(agentSubmissionGateway.prepareWriting({...request, requestId:fixtureId})).rejects.toThrow("agent_preview_owner_intent_missing");
    expect(invoke).toHaveBeenCalledTimes(calls);
    const unknown = {...request, requestId:fixtureId};
    await expect(agentSubmissionGateway.prepareWritingStatus(unknown)).rejects.toBe(failure);
    expect(invoke).toHaveBeenLastCalledWith("agent_read_writing_submission_prepare_status", {request:unknown});
  } finally {selectAppRuntime(previous); reopened.clear();}
});

it("recovers a cold request owner from native proof before sending it through the production gateway", async () => {
  const previous=getAppRuntime(),storage=createTestNamespacedStorage(window.localStorage,`owner-cold-${crypto.randomUUID()}:`);
  const request=decodeAgentWritingPrepareRequest(golden.ordinary.intent), fixtureId="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  let finish!:()=>void;
  registerStoragePersistence(storage,{backend:"IndexedDB",pending:false,error:undefined,resetting:false,
    flush:()=>new Promise<void>(resolve=>{finish=resolve;}),retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  const lookup=vi.fn(async()=>({schemaVersion:1,status:"found",generation:"g",conversationId:request.conversationId,requestId:request.requestId,fixtureId}));
  const preview={bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},invoke:lookup} as unknown as NativeAgentPreview;
  const failure=new Error("business reached"),invoke=vi.fn().mockRejectedValue(failure);
  selectAppRuntime({mode:"preview",storage,gateways:{},nativeAgentPreview:preview,invoke,dispose(){}});
  try {
    const work=agentSubmissionGateway.prepareWriting(request);
    await vi.waitFor(()=>expect(lookup).toHaveBeenCalledOnce());
    expect(invoke).not.toHaveBeenCalled();
    finish(); await expect(work).rejects.toBe(failure);
    expect(invoke).toHaveBeenCalledWith("agent_prepare_writing_submission",{request},{previewOwnerIntent:{conversationId:request.conversationId,fixtureId}});
  } finally {selectAppRuntime(previous);storage.clear();}
});

it("checks a legacy request without owner proof as a read only operation and never replays it", async () => {
  const previous=getAppRuntime(),storage=createTestNamespacedStorage(window.localStorage,`owner-status-${crypto.randomUUID()}:`);
  const request=decodeAgentWritingPrepareRequest(golden.ordinary.intent);
  registerStoragePersistence(storage,{backend:"IndexedDB",pending:false,error:undefined,resetting:false,flush:async()=>{},retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  const lookup=vi.fn();
  const preview={bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},invoke:lookup} as unknown as NativeAgentPreview;
  const invoke=vi.fn(async(command:string)=>{if(command!=="agent_read_writing_submission_prepare_status")throw new Error("unexpected mutation");
    return {schemaVersion:1,intent:request,status:{type:"notFound"}};});
  selectAppRuntime({mode:"preview",storage,gateways:{},nativeAgentPreview:preview,invoke,dispose(){}});
  try {
    const result=await agentSubmissionGateway.prepareWritingStatus(request);
    expect(result.status.type).toBe("notFound");
    expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_read_writing_submission_prepare_status",{request});
    expect(lookup).not.toHaveBeenCalled(); expect(storage.length).toBe(0);
  } finally {selectAppRuntime(previous);storage.clear();}
});
