// @vitest-environment happy-dom
import {beforeEach, expect, it} from "vitest";
import {EMPTY_AGENT_CONTEXT, type AgentContextConfigurationSaveIntent, type AgentContextState} from "@research-assistant/core/agent";
import {createStructuredDocumentAdapter, HbsSession, type DocumentCheckpoint, type HbsDocumentAdapter} from "@research-assistant/core/hbs";
import {createIndexedPreviewStorage} from "../../../runtime/previewIndexedStorage";
import {configurePreviewDatabase} from "../../../runtime/previewIndexedDatabase";
import {createMemoryPreviewDatabase} from "../../../../app/runtime/testing/memoryPreviewDatabase";
import type {ConversationOperationLock} from "../conversationOperationLock";
import {ContextConfigurationStorageOutbox} from "./contextConfigurationOutbox";
import {ContextConfigurationPreservation} from "./contextConfigurationPreservation";
import {openContextConfigurationHbs} from "./contextConfigurationHbs";
import {ContextConfigurationOwner} from "./contextConfigurationOwner";
import type {AgentContextConfigurationSaveGateway} from "@research-assistant/data/agent";
import {loadRecoveryDraft} from "../../../services/recoveryDraftStore";
import {recoveryDraftKey} from "../../../services/recoveryDraftStore";

const conversation="11111111-1111-4111-8111-111111111111",epoch="22222222-2222-4222-8222-222222222222";
const adapter=createStructuredDocumentAdapter((a,b)=>a===b?[]:[{fromA:0,toA:a.length,fromB:0,toB:b.length}]) as HbsDocumentAdapter<AgentContextState>;
const item={resource:{kind:"paperMetadata" as const,resourceId:"synthetic"},label:"Synthetic"};
const edited:AgentContextState={...EMPTY_AGENT_CONTEXT,temporary:[item]};
const snapshot={version:1,saved:EMPTY_AGENT_CONTEXT,baseline:EMPTY_AGENT_CONTEXT};
const checkpoint:DocumentCheckpoint<AgentContextState>={format:1,kind:adapter.kind,observed:snapshot,
  ancestor:EMPTY_AGENT_CONTEXT,baselineAncestor:EMPTY_AGENT_CONTEXT,content:edited,ordinary:adapter.diff(EMPTY_AGENT_CONTEXT,edited)};
let database:ReturnType<typeof createMemoryPreviewDatabase>,lock:ConversationOperationLock;
beforeEach(()=>{
  database=createMemoryPreviewDatabase();configurePreviewDatabase(database.open);
  const held=new Set<string>();lock={async run<T>(name:string,operation:()=>Promise<T>):Promise<T>{
    if(held.has(name))throw Error("LOCK_BUSY");held.add(name);try{return await operation();}finally{held.delete(name);}
  }};
});
function preservation(storage:Storage,environment=epoch){
  const outbox=new ContextConfigurationStorageOutbox(storage,conversation,lock);
  return new ContextConfigurationPreservation(storage,conversation,outbox,()=>environment,adapter,lock);
}

function ownerGateway():AgentContextConfigurationSaveGateway {
  return {
    environment:async request=>({schemaVersion:1,...request,epoch,sqlFence:"0",configuration:{revision:1,state:EMPTY_AGENT_CONTEXT}}),
    status:async()=>({type:"noSqlReceipt",epoch,sqlFence:"0",closed:false}),
    save:async()=>{throw Error("unexpected authority write");},
  };
}

it("repairs a failed local checkpoint without discarding the edited configuration",async()=>{
  const storage=await createIndexedPreviewStorage("context-owner-repair:"),gateway=ownerGateway();
  const owner=await ContextConfigurationOwner.open({storage,conversationId:conversation,gateway,newId:()=>crypto.randomUUID(),lock});
  database.failNext(new DOMException("full","QuotaExceededError"));
  await expect(owner.edit(edited)).rejects.toThrow("MODEL_PRESERVATION_FAILED");
  expect(owner.getSnapshot().session.content).toEqual(edited);
  await expect(owner.edit(EMPTY_AGENT_CONTEXT)).rejects.toThrow("MODEL_PRESERVATION_FAILED");
  expect(owner.getSnapshot().session.content).toEqual(edited);
  await owner.repairLocal();
  expect(owner.getSnapshot().session.preservationError).toBeUndefined();
  const reopened=await createIndexedPreviewStorage("context-owner-repair:");
  const restored=await ContextConfigurationOwner.open({storage:reopened,conversationId:conversation,gateway,newId:()=>crypto.randomUUID(),lock});
  expect(restored.getSnapshot().session.content).toEqual(edited);
});

it("reopens a clean blocked session after the original recovery read becomes available",async()=>{
  const storage=await createIndexedPreviewStorage("context-owner-read-repair:"),gateway=ownerGateway();
  const original=await ContextConfigurationOwner.open({storage,conversationId:conversation,gateway,newId:()=>crypto.randomUUID(),lock});
  await original.edit(edited);
  const key=recoveryDraftKey({scope:"agent-context-hbs",resourceId:conversation,slot:"configuration"});
  const read=storage.getItem.bind(storage);let fail=true;
  storage.getItem=(name:string)=>{if(name===key && fail){fail=false;throw Error("temporary local read failure");}return read(name);};
  const owner=await ContextConfigurationOwner.open({storage,conversationId:conversation,gateway,newId:()=>crypto.randomUUID(),lock});
  expect(owner.getSnapshot().session.preservationError).toContain("temporary local read failure");
  await expect(owner.edit(EMPTY_AGENT_CONTEXT)).rejects.toThrow("MODEL_PRESERVATION_FAILED");
  await owner.repairLocal();
  expect(owner.getSnapshot().session.preservationError).toBeUndefined();
  expect(owner.getSnapshot().session.content).toEqual(edited);
});

it("repairs a failed transport outbox before retrying the same HBS request",async()=>{
  const storage=await createIndexedPreviewStorage("context-owner-outbox-repair:");
  const requests:AgentContextConfigurationSaveIntent[]=[];
  const gateway:AgentContextConfigurationSaveGateway={...ownerGateway(),save:async intent=>{
    requests.push(structuredClone(intent));return {schemaVersion:1,operationId:intent.operationId,conversationId:conversation,
      epoch,family:"agent.context.configuration.save",sqlFence:"0",replayed:false,result:null};
  }};
  const owner=await ContextConfigurationOwner.open({storage,conversationId:conversation,gateway,newId:()=>crypto.randomUUID(),lock});
  await owner.edit(edited);
  const write=storage.setItem.bind(storage);let inject=true;
  storage.setItem=(key,value)=>{if(inject && key.startsWith("research-assistant:context-configuration-save:v1:")){
    inject=false;database.failNext(new DOMException("full","QuotaExceededError"));}write(key,value);};
  await expect(owner.save()).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  expect(requests).toHaveLength(0);
  expect(owner.getSnapshot().session.content).toEqual(edited);
  await owner.repairLocal();
  await owner.retrySave();
  expect(requests).toHaveLength(1);expect(requests[0]?.state).toEqual(edited);
  owner.dispose();
});

it("does not overwrite an unreadable checkpoint from another data epoch during repair",async()=>{
  const storage=await createIndexedPreviewStorage("context-owner-epoch-repair:"),first=preservation(storage);
  await first.load();await first.save(checkpoint);
  const key=recoveryDraftKey({scope:"agent-context-hbs",resourceId:conversation,slot:"configuration"}),before=storage.getItem(key);
  const otherEpoch="33333333-3333-4333-8333-333333333333";
  const gateway:AgentContextConfigurationSaveGateway={...ownerGateway(),environment:async request=>({schemaVersion:1,...request,
    epoch:otherEpoch,sqlFence:"0",configuration:{revision:1,state:EMPTY_AGENT_CONTEXT}})};
  const owner=await ContextConfigurationOwner.open({storage,conversationId:conversation,gateway,newId:()=>crypto.randomUUID(),lock});
  expect(owner.getSnapshot().session.preservationError).toContain("CONTEXT_CONFIGURATION_PRESERVATION_EPOCH_MISMATCH");
  await expect(owner.repairLocal()).rejects.toThrow("CONTEXT_CONFIGURATION_PRESERVATION_EPOCH_MISMATCH");
  expect(storage.getItem(key)).toBe(before);
  expect(owner.getSnapshot().phase).toBe("retained");
});

it("reopens HBS S′ without writing the authority",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-preserve:"),first=preservation(storage);
  expect(await first.load()).toBeUndefined();await first.save(checkpoint);
  const reopened=await createIndexedPreviewStorage("context-hbs-preserve:"),restored=preservation(reopened);
  let commits=0;
  const session=await HbsSession.open({adapter,preservation:restored,requestId:()=>crypto.randomUUID(),
    port:{read:async()=>snapshot,commit:async()=>{commits++;throw Error("unexpected commit");}}});
  expect(session.getSnapshot().content).toEqual(edited);expect(commits).toBe(0);session.dispose();
});

it("does not overwrite a checkpoint changed by another owner",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-cas:"),first=preservation(storage),second=preservation(storage);
  await first.load();await second.load();await first.save(checkpoint);
  await expect(second.save({...checkpoint,content:EMPTY_AGENT_CONTEXT})).rejects.toThrow("CONTEXT_CONFIGURATION_PRESERVATION_CHANGED");
  const reopened=await createIndexedPreviewStorage("context-hbs-cas:"),restored=preservation(reopened);
  expect((await restored.load())?.content).toEqual(edited);
});

it("rejects a checkpoint from another data epoch without changing it",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-epoch:"),first=preservation(storage);
  await first.load();await first.save(checkpoint);
  const other=preservation(storage,"33333333-3333-4333-8333-333333333333");
  await expect(other.load()).rejects.toThrow("CONTEXT_CONFIGURATION_PRESERVATION_EPOCH_MISMATCH");
  expect((await preservation(storage).load())?.content).toEqual(edited);
});

it("retains the failed S′ write behind a barrier until explicitly repaired",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-failure:"),first=preservation(storage),other=preservation(storage);
  await first.load();database.failNext(new DOMException("full","QuotaExceededError"));
  await expect(first.save(checkpoint)).rejects.toThrow();
  await expect(other.load()).rejects.toThrow();
  const before=await createIndexedPreviewStorage("context-hbs-failure:");expect(await preservation(before).load()).toBeUndefined();
  await first.repair();
  const after=await createIndexedPreviewStorage("context-hbs-failure:");expect((await preservation(after).load())?.content).toEqual(edited);
});

it("does not trap an unchanged S′ after an oversized checkpoint is rejected before storage",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-too-large:"),owner=preservation(storage);
  await owner.load();
  const tooLarge={...checkpoint,content:{...edited,temporary:[{...item,label:"x".repeat(9*1024*1024)}]}};
  await expect(owner.save(tooLarge)).rejects.toThrow("8 MiB");
  await owner.save(checkpoint);expect((await preservation(storage).load())?.content).toEqual(edited);
});

it("does not repair a failed transport outbox through S′ recovery",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-independent-repair:"),outbox=new ContextConfigurationStorageOutbox(storage,conversation,lock);
  const saved=new ContextConfigurationPreservation(storage,conversation,outbox,()=>epoch,adapter,lock);
  await saved.load();database.failNext(new DOMException("full","QuotaExceededError"));
  const intent={operationId:"44444444-4444-4444-8444-444444444444",conversationId:conversation,expectedEpoch:epoch,expectedRevision:1,state:edited};
  await expect(outbox.run(()=>outbox.write(intent))).rejects.toThrow();
  await expect(saved.repair()).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCAL_UNCONFIRMED");
  const before=await createIndexedPreviewStorage("context-hbs-independent-repair:");
  const reader=new ContextConfigurationStorageOutbox(before,conversation,lock);
  expect(await reader.run(()=>reader.read())).toBeNull();
  await outbox.repair();expect(await outbox.run(()=>outbox.read())).toEqual(intent);
});

it("does not retry shared storage while another transport owner holds its lock",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-repair-lock:");
  const first=new ContextConfigurationStorageOutbox(storage,conversation,lock),second=new ContextConfigurationStorageOutbox(storage,conversation,lock);
  const saved=new ContextConfigurationPreservation(storage,conversation,second,()=>epoch,adapter,lock);
  let release!:()=>void;const pending=first.run(()=>new Promise<void>(resolve=>{release=resolve;}));
  for(let n=0;n<100 && !release;n++)await Promise.resolve();expect(release).toBeTypeOf("function");
  await expect(saved.repair()).rejects.toThrow("LOCK_BUSY");release();await pending;
});

it("composes the real HBS session, durable outbox and S′ across a remount",async()=>{
  const storage=await createIndexedPreviewStorage("context-hbs-composition:");
  let saved:AgentContextState=EMPTY_AGENT_CONTEXT,revision=1,release!:()=>void,first=true,sequence=0;
  const receipts=new Set<string>();
  const gateway:AgentContextConfigurationSaveGateway={
    environment:async request=>({schemaVersion:1,...request,epoch,sqlFence:"0",configuration:{revision,state:saved}}),
    status:async request=>receipts.has(request.operationId)
      ? {type:"committed",proof:{schemaVersion:1,operationId:request.operationId,conversationId:conversation,epoch,family:"agent.context.configuration.save",sqlFence:"0",replayed:true,result:null}}
      : {type:"noSqlReceipt",epoch,sqlFence:"0",closed:false},
    save:async (request:AgentContextConfigurationSaveIntent)=>{
      if(first){first=false;await new Promise<void>(resolve=>{release=resolve;});}
      expect(request.expectedRevision).toBe(revision);saved=structuredClone(request.state);revision++;receipts.add(request.operationId);
      return {schemaVersion:1,operationId:request.operationId,conversationId:conversation,epoch,family:"agent.context.configuration.save",sqlFence:"0",replayed:false,result:null};
    },
  };
  const newId=()=>`00000000-0000-4000-8000-${String(++sequence).padStart(12,"0")}`;
  const owner=await openContextConfigurationHbs({storage,conversationId:conversation,gateway,newId,lock});
  const firstEdit:AgentContextState={...EMPTY_AGENT_CONTEXT,persistent:[item]};
  owner.session.edit(firstEdit);const pending=owner.session.save();
  for(let n=0;n<100 && !release;n++)await Promise.resolve();expect(release).toBeTypeOf("function");
  owner.session.edit(edited);await owner.session.save();
  const during=await createIndexedPreviewStorage("context-hbs-composition:");
  const preserved=loadRecoveryDraft({scope:"agent-context-hbs",resourceId:conversation,slot:"configuration"},during);
  expect((preserved?.payload as DocumentCheckpoint<AgentContextState>).content).toEqual(edited);
  expect(revision).toBe(1);release();await pending;
  for(let n=0;n<100;n++)await Promise.resolve();
  expect(owner.session.getSnapshot().content).toEqual(edited);expect(saved).toEqual(edited);expect(revision).toBe(3);
  await owner.session.flushPreservation();owner.session.dispose();
  const reopenedStorage=await createIndexedPreviewStorage("context-hbs-composition:");
  const reopened=await openContextConfigurationHbs({storage:reopenedStorage,conversationId:conversation,gateway,newId,lock});
  expect(reopened.session.getSnapshot().content).toEqual(edited);expect(revision).toBe(3);reopened.session.dispose();
});
