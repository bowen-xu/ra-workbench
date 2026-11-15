import {expect,it} from "vitest";
import type {AgentContextPreview} from "@research-assistant/core/agent";
import {MockConversationQueue} from "./MockConversationQueue";

function fixture() {
  const values=new Map<string,string>();
  const storage:Storage={get length(){return values.size;},key:i=>[...values.keys()][i]??null,getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
  const conversationId=crypto.randomUUID(),inputId=crypto.randomUUID();
  const queue=new MockConversationQueue(storage,async()=>undefined);
  const userInput=[{type:"text" as const,text:"Follow up",textElements:[]},{type:"skill" as const,name:"review",identity:{environment_id:"application",source_id:"review/SKILL.md",revision:"v1"}}];
  const request={conversationId,requestId:crypto.randomUUID(),expectedRevision:0,edit:{type:"add" as const,inputId,text:"Follow up",userInput}};
  const preview:AgentContextPreview={schemaVersion:1,conversationId,requestId:crypto.randomUUID(),turnId:crypto.randomUUID(),contextSnapshotId:crypto.randomUUID(),input:{text:request.edit.text,userInput,model:{providerId:"fixture",modelId:"mock",revision:null},reasoningEffort:null,context:[]}};
  return {queue,storage,conversationId,inputId,request,preview};
}

it("persists acknowledged queue edits, exact references and stop state across reopen",async()=>{
  const {queue,storage,conversationId,request}=fixture();
  const added=await queue.gateway.edit(request);
  expect(await queue.gateway.edit(request)).toEqual(added);
  await expect(queue.gateway.edit({...request,edit:{...request.edit,text:"Different",userInput:[]}})).rejects.toMatchObject({code:"conflict"});
  queue.pause(conversationId);
  const reopened=new MockConversationQueue(storage,async()=>undefined);
  const restored=await reopened.gateway.read({conversationId});
  expect(restored.paused).toBe(true);
  expect(restored.inputs).toEqual(added.inputs);
  expect(await reopened.gateway.read({conversationId:crypto.randomUUID()})).toMatchObject({inputs:[]});
});

it("retains a claimed head until admission and admits an exact retry once",async()=>{
  const {queue,storage,conversationId,request,preview,inputId}=fixture();
  await queue.gateway.edit(request);
  const confirmation=queue.claim(preview,{inputId,expectedRevision:1});
  const frozen={...preview,queue:confirmation};
  expect(queue.read(conversationId).inputs).toHaveLength(1);
  expect(queue.isAdmitted(frozen)).toBe(false);
  expect(queue.confirmation(frozen)).toEqual(confirmation);
  queue.admit(frozen);queue.admit(frozen);
  expect(queue.read(conversationId)).toMatchObject({inputs:[],revision:3,preparingInputId:null});
  expect(queue.confirmation(frozen)).toEqual(confirmation);
  expect(queue.isAdmitted(frozen)).toBe(true);
  const reopened=new MockConversationQueue(storage,async()=>undefined);
  expect(reopened.isAdmitted(frozen)).toBe(true);
  expect(reopened.isAdmitted({...frozen,queue:{...confirmation,expectedRevision:confirmation.expectedRevision+1}})).toBe(false);
});

it("does not consume a changed or paused submission",async()=>{
  const {queue,conversationId,request,preview,inputId}=fixture();
  await queue.gateway.edit(request);
  expect(()=>queue.claim({...preview,input:{...preview.input,text:"changed"}},{inputId,expectedRevision:1})).toThrow();
  const confirmation=queue.claim(preview,{inputId,expectedRevision:1});
  queue.pause(conversationId);
  expect(()=>queue.admit({...preview,queue:confirmation})).toThrow();
  expect(queue.read(conversationId).inputs).toHaveLength(1);
  const restored={...preview,queue:queue.confirmation({...preview,queue:confirmation})!};
  queue.admit(restored,true);
  expect(queue.read(conversationId)).toMatchObject({paused:false,inputs:[]});
  queue.pause(conversationId);
  queue.admit(restored,true);
  expect(queue.read(conversationId).paused).toBe(true);
  expect(()=>queue.requireStartable(restored)).toThrow();
  expect(()=>queue.requireStartable(preview)).not.toThrow();
  const paused=queue.read(conversationId);
  await queue.gateway.edit({conversationId,requestId:crypto.randomUUID(),expectedRevision:paused.revision,edit:{type:"resume"}});
  expect(()=>queue.requireStartable(restored)).not.toThrow();
});

it("binds a guarded context queue edit to its original data epoch",async()=>{
  const {storage,conversationId,request}=fixture(), epoch=crypto.randomUUID();
  const queue=new MockConversationQueue(storage,async()=>undefined,epoch);
  const guarded={...request,expectedEpoch:epoch};
  const foreign={...guarded,expectedEpoch:crypto.randomUUID()};
  await expect(queue.gateway.edit(foreign)).rejects.toMatchObject({code:"conflict"});
  expect(queue.read(conversationId).revision).toBe(0);
  const result=await queue.gateway.edit(guarded);
  expect(await queue.gateway.edit(guarded)).toEqual(result);
  await expect(queue.gateway.edit(request)).rejects.toMatchObject({code:"conflict"});
});
